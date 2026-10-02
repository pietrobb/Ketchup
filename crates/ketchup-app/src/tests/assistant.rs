//! The Assistant panel, requests, context, memory and replanning.

use super::*;

#[test]
fn oversized_external_assistant_model_catalog_falls_back_to_embedded_models() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-models.yaml");
    let mut catalog = String::from("- \"gpt-untrusted [api]\"\n");
    catalog.push_str(&" ".repeat(64 * 1024));
    std::fs::write(&path, catalog).unwrap();

    let catalog = assistant_model_catalog_text_from_path(Some(&path));

    assert_eq!(catalog, ASSISTANT_MODELS_YAML);
}

#[test]
fn external_assistant_model_catalog_requires_a_regular_utf8_file() {
    let directory = tempfile::tempdir().unwrap();
    let invalid_utf8 = directory.path().join("invalid.yaml");
    std::fs::write(&invalid_utf8, [0xff, 0xfe]).unwrap();

    assert_eq!(
        assistant_model_catalog_text_from_path(Some(&invalid_utf8)),
        ASSISTANT_MODELS_YAML
    );
    assert_eq!(
        assistant_model_catalog_text_from_path(Some(directory.path())),
        ASSISTANT_MODELS_YAML
    );
}

#[test]
fn bounded_external_assistant_model_catalog_remains_configurable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-models.yaml");
    let catalog = "- \"gpt-private [api]\"\n";
    std::fs::write(&path, catalog).unwrap();

    assert_eq!(assistant_model_catalog_text_from_path(Some(&path)), catalog);
}

#[test]
fn assistant_preview_planner_preserves_canonical_and_validator_diagnostics() {
    let app = KetchupApp::new();
    let canonical = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::Workflow(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(999),
                visible: false,
            },
        ))
        .unwrap_err();
    assert_eq!(
        canonical.phase,
        AssistantRejectionPhase::CanonicalValidation
    );
    assert_eq!(canonical.code, "canonical.occurrence_not_found");
    assert_eq!(canonical.operation, "workflow_intent");
    assert_eq!(
        canonical.target,
        format!("document:{}", app.document.current().document_id().0)
    );
    assert_eq!(canonical.failed_invariant, "occurrence 999 does not exist");
    assert!(canonical.retryable);
    assert_eq!(canonical.validate(), Ok(()));

    let invalid_model = serde_json::from_value::<AssistantModelIntent>(serde_json::json!({
        "replace_scene": false
    }))
    .unwrap();
    let intent = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::Model(invalid_model))
        .unwrap_err();
    assert_eq!(intent.phase, AssistantRejectionPhase::IntentValidation);
    assert_eq!(intent.code, "intent.model_invalid");
    assert_eq!(intent.validate(), Ok(()));

    let selection = AssistantValidationSelection {
        mode: "selected",
        requested: BTreeSet::new(),
        unknown: vec!["mystery".to_owned()],
    };
    let validator = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::ValidationRepair(selection))
        .unwrap_err();
    assert_eq!(validator.phase, AssistantRejectionPhase::DomainValidation);
    assert_eq!(validator.code, "validator.selection_invalid");
    assert_eq!(validator.validate(), Ok(()));
}

#[test]
fn structured_rejection_is_localized_and_drives_exactly_one_bounded_replan() {
    struct RejectedReplanTransport {
        contexts: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl AssistantTransport for RejectedReplanTransport {
        fn chat(
            &self,
            _handshake: AssistantHandshake,
            _request_id: &str,
            _message: &str,
            context: &serde_json::Value,
            _cancellation: AssistantCancellation,
        ) -> Result<AssistantChatResult, ketchup_rejection::Rejection> {
            self.contexts.lock().unwrap().push(context.clone());
            Ok(AssistantChatResult {
                message: "Still targets a missing occurrence.".to_owned(),
                model_intent: Some(missing_occurrence_model_intent()),
            })
        }
    }

    fn missing_occurrence_model_intent() -> AssistantModelIntent {
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
                occurrence_id: 999,
                delta_mm: [10.0, 0.0, 0.0],
            }],
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
        }
    }

    let transport = Arc::new(RejectedReplanTransport {
        contexts: std::sync::Mutex::new(Vec::new()),
    });
    let mut app = KetchupApp::new();
    app.assistant.transport = transport.clone();
    app.assistant.messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Move occurrence 999.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );
    app.assistant.pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(missing_occurrence_model_intent()),
        },
        message: "Move occurrence 999.".to_owned(),
        replan_attempted: false,
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    let context = egui::Context::default();

    app.poll_assistant_chat(&context);

    let first_diagnostic = app
        .assistant
        .messages
        .last()
        .and_then(|message| message.diagnostic.clone())
        .expect("the first rejection preserves its structured diagnostic");
    assert_eq!(first_diagnostic.code, "canonical.occurrence_not_found");
    assert!(app.assistant.chat_task.is_some());
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.assistant.pending_execution.is_none()
        && app.assistant.chat_task.is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
        app.poll_assistant_chat(&context);
    }
    assert!(app.assistant.pending_execution.is_some());

    let contexts = transport.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0]["assistant_replan"]["attempt"], 1);
    assert_eq!(contexts[0]["assistant_replan"]["max_attempts"], 1);
    assert_eq!(
        contexts[0]["assistant_replan"]["diagnostic"],
        serde_json::to_value(&first_diagnostic).unwrap()
    );
    assert!(assistant_context_byte_length(&contexts[0]) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES);
    drop(contexts);

    app.poll_assistant_chat(&context);

    assert!(app.assistant.chat_task.is_none());
    assert!(app.assistant.pending_execution.is_none());
    assert_eq!(transport.contexts.lock().unwrap().len(), 1);
    assert_eq!(
        app.assistant
            .messages
            .iter()
            .filter(|message| message.diagnostic.is_some())
            .count(),
        2
    );
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );
    let english = app.localized_assistant_rejection(&first_diagnostic, true);
    let slovak = KetchupApp::with_catalog(LocaleCatalog::slovak())
        .localized_assistant_rejection(&first_diagnostic, true);
    for text in [english, slovak] {
        assert!(text.contains("canonical.occurrence_not_found"));
        assert!(text.contains("occurrence 999 does not exist"));
    }
}

#[test]
fn canonical_error_codes_are_stable_machine_identifiers() {
    assert_eq!(
        CanonicalError::OccurrenceInAssemblyMate(OccurrenceId(17)).code(),
        "canonical.occurrence_in_assembly_mate"
    );
    assert_eq!(
        CanonicalError::DefinitionNotFound(DefinitionId(9)).code(),
        "canonical.definition_not_found"
    );
}

#[test]
fn manual_and_assistant_push_pull_share_the_identical_canonical_batch() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.set_push_pull_distance_input("15");
    assert!(app.start_preview());
    let manual = app.push_pull.smart_proposal.as_ref().unwrap().clone();
    app.cancel_preview();

    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
            target: FeatureId(2),
            value_text: "35".to_owned(),
        })
    );
    let assistant = app.assistant.proposal.as_ref().unwrap();
    assert_eq!(manual.batch(), assistant.batch());
    assert_eq!(manual.command_digest(), assistant.command_digest());
    assert_eq!(manual.principal(), ProposalPrincipal::ManualClient);
    assert_eq!(assistant.principal(), ProposalPrincipal::LocalAssistant);
    assert_eq!(app.document_revision(), manual.provenance_revision());
}

#[test]
fn assistant_preview_plan_rejects_source_proposal_stale_and_replay_atomically() {
    fn state(app: &KetchupApp) -> (u64, String, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.document.visible_undo_steps(),
        )
    }

    let visibility_intent = WorkflowIntent::SetOccurrenceVisibility {
        target: OccurrenceId(1),
        visible: false,
    };

    let mut proposal_tampered = KetchupApp::new();
    assert!(proposal_tampered.prepare_assistant_intent(visibility_intent.clone()));
    let mut visibility_plan = proposal_tampered.assistant.proposal.take().unwrap();
    assert!(
        proposal_tampered.prepare_assistant_intent(WorkflowIntent::RenameDefinition {
            target: INITIAL_BOX_DEFINITION,
            name: "Malicious replacement".to_owned(),
        })
    );
    visibility_plan.proposal = proposal_tampered
        .assistant
        .proposal
        .take()
        .unwrap()
        .proposal;
    let before_tamper = state(&proposal_tampered);
    proposal_tampered.assistant.proposal = Some(visibility_plan);
    assert!(!proposal_tampered.confirm_assistant_proposal());
    assert_eq!(state(&proposal_tampered), before_tamper);

    let mut source_tampered = KetchupApp::new();
    assert!(source_tampered.prepare_assistant_intent(visibility_intent.clone()));
    let mut plan = source_tampered.assistant.proposal.take().unwrap();
    plan.source = AssistantPreviewSource::Workflow(WorkflowIntent::SetOccurrenceVisibility {
        target: OccurrenceId(1),
        visible: true,
    });
    let before_source_tamper = state(&source_tampered);
    source_tampered.assistant.proposal = Some(plan);
    assert!(!source_tampered.confirm_assistant_proposal());
    assert_eq!(state(&source_tampered), before_source_tamper);

    let mut model_tampered = KetchupApp::new();
    let model_intent = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
            occurrence_id: 1,
            delta_mm: [5.0, 0.0, 0.0],
        }],
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
    };
    assert!(model_tampered.prepare_assistant_model_intent(model_intent));
    let mut model_plan = model_tampered.assistant.proposal.take().unwrap();
    assert!(model_tampered.prepare_assistant_intent(visibility_intent.clone()));
    model_plan.proposal = model_tampered.assistant.proposal.take().unwrap().proposal;
    let before_model_tamper = state(&model_tampered);
    model_tampered.assistant.proposal = Some(model_plan);
    assert!(!model_tampered.confirm_assistant_proposal());
    assert_eq!(state(&model_tampered), before_model_tamper);

    let mut stale = KetchupApp::new();
    assert!(stale.prepare_assistant_intent(visibility_intent.clone()));
    assert!(stale.create_box());
    let before_stale = state(&stale);
    assert!(!stale.confirm_assistant_proposal());
    assert_eq!(state(&stale), before_stale);

    let mut valid = KetchupApp::new();
    let initial = state(&valid);
    assert!(valid.prepare_assistant_intent(visibility_intent));
    let replay = valid.assistant.proposal.clone().unwrap();
    assert!(valid.confirm_assistant_proposal());
    assert_eq!(valid.document.visible_undo_steps(), initial.2 + 1);
    assert!(
        !valid
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .visible()
    );
    let committed = state(&valid);
    valid.assistant.proposal = Some(replay);
    assert!(!valid.confirm_assistant_proposal());
    assert_eq!(state(&valid), committed);
    assert!(valid.undo());
    assert_eq!(state(&valid), initial);
}

#[test]
fn assistant_conversation_changes_participate_in_document_dirty_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chat-dirty.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));

    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    app.assistant.messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Create a shelf.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    assert!(app.is_dirty());
    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    app.new_assistant_chat();
    assert!(app.is_dirty());
}

#[test]
fn assistant_context_exposes_bounded_identified_read_only_agent_state_view() {
    let app = KetchupApp::new();
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );

    let context = app.assistant_context();
    let state_view = context["state_view"].as_object().unwrap();
    let content = state_view["content"].as_str().unwrap();
    assert_eq!(state_view["format"], AGENT_STATE_VIEW);
    assert_eq!(state_view["complete"], true);
    assert_eq!(state_view["byte_length"], content.len());
    assert_eq!(
        state_view["sha256"],
        ketchup_model::graph::sha256_hex(content.as_bytes())
    );
    assert!(content.starts_with(&format!("schema={AGENT_STATE_VIEW}\n")));
    assert!(content.contains(&format!(
        "source.canonical_digest={}",
        app.document.current().canonical_digest()
    )));
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );

    let oversized = "semantic.line=bounded\n".repeat(MAX_ASSISTANT_STATE_VIEW_BYTES);
    let bounded = bounded_assistant_state_view(&oversized);
    let bounded_content = bounded["content"].as_str().unwrap();
    assert_eq!(bounded["complete"], false);
    assert_eq!(bounded["byte_length"], oversized.len());
    assert_eq!(
        bounded["sha256"],
        ketchup_model::graph::sha256_hex(oversized.as_bytes())
    );
    assert!(bounded_content.len() <= MAX_ASSISTANT_STATE_VIEW_BYTES);
    assert!(bounded_content.ends_with('\n'));
}

#[test]
fn assistant_context_exposes_the_selected_group_for_generic_rotation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Rotatable group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(GroupId(1)),
            },
        ]))
        .unwrap();
    assert!(app.select_group(GroupId(1)));

    let context = app.assistant_context();
    assert_eq!(context["selected_group_id"], 1);
}

#[test]
fn provider_context_remains_bounded_for_extreme_selection_and_history() {
    let long_text = "x".repeat(4 * 1024);
    let occurrences = (1..=100)
        .map(|id| serde_json::json!({ "occurrence_id": id, "name": long_text }))
        .collect::<Vec<_>>();
    let conversation = (0..20)
        .map(|_| serde_json::json!({ "role": "assistant", "text": long_text }))
        .collect::<Vec<_>>();
    let issues = (0..100)
        .map(|id| serde_json::json!({ "code": "test", "evidence": long_text, "id": id }))
        .collect::<Vec<_>>();
    let topology_face_references = (0..64)
        .map(|id| {
            serde_json::json!({
                "definition_id": 1,
                "target_feature_id": 2,
                "reference_id": format!("{id:064x}"),
            })
        })
        .collect::<Vec<_>>();
    let topology_edge_references = (64..128)
        .map(|id| {
            serde_json::json!({
                "definition_id": 1,
                "target_feature_id": 2,
                "reference_id": format!("{id:064x}"),
            })
        })
        .collect::<Vec<_>>();
    let context = serde_json::json!({
        "document_id": 1,
        "revision": 1,
        "canonical_digest": "digest",
        "state_view": {
            "format": AGENT_STATE_VIEW,
            "complete": false,
            "byte_length": long_text.len(),
            "sha256": "digest",
            "content": long_text,
        },
        "project_memory": {
            "schema": PROJECT_MEMORY_SCHEMA,
            "document_id": 1,
            "stored_count": 0,
            "retrieved_count": 0,
            "complete": true,
            "byte_length": 2,
            "entries": [],
        },
        "validation": {
            "schema": "ketchup.assistant-validation-context.v1",
            "complete": true,
            "issues_complete": true,
            "issue_count": issues.len(),
            "issues": issues,
        },
        "selected_occurrence_ids": (1..=100).collect::<Vec<_>>(),
        "selected_group_id": 1,
        "selected_profile_translation_target": null,
        "selected_parameter_edit_target": null,
        "topology_face_references_complete": true,
        "topology_face_references": topology_face_references,
        "topology_edge_references_complete": true,
        "topology_edge_references": topology_edge_references,
        "occurrence_count": 100,
        "occurrences_complete": true,
        "occurrences": occurrences,
        "boxes": [{ "name": long_text }],
        "conversation": conversation,
    });

    let bounded = bounded_assistant_provider_context(context);
    assert!(assistant_context_byte_length(&bounded) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES);
    assert_eq!(bounded["context_complete"], false);
    assert_eq!(bounded["occurrences_complete"], false);
    assert_eq!(bounded["validation"]["complete"], false);
    assert_eq!(bounded["validation"]["details_truncated"], true);
    assert_eq!(bounded["topology_face_references_complete"], false);
    assert!(
        bounded["topology_face_references"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(bounded["topology_edge_references_complete"], false);
    assert!(
        bounded["topology_edge_references"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn only_successful_assistant_completion_enters_project_memory() {
    let mut app = KetchupApp::new();
    app.assistant.messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Remember the shelf spacing.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            fea_review: None,
            result: AssistantChatResult {
                message: "The shelf spacing is 320 mm.".to_owned(),
                model_intent: None,
            },
            diagnostics: None,
        }))
        .unwrap();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    app.poll_assistant_chat(&egui::Context::default());
    assert_eq!(app.assistant.memory.entries.len(), 1);
    assert!(app.file.container_data.extensions().any(|entry| {
        entry.namespace() == ASSISTANT_CHAT_NAMESPACE && entry.path() == ASSISTANT_MEMORY_PATH
    }));

    let mut failed = KetchupApp::new();
    failed.assistant.messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Do not remember a failed request.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Err(
            Rejection::new("provider", RejectionPhase::Io).reason("provider unavailable")
        ))
        .unwrap();
    failed.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: failed.document.current().document_id(),
        revision_id: failed.document.current().revision_id(),
        canonical_digest: failed.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    failed.poll_assistant_chat(&egui::Context::default());
    assert!(failed.assistant.memory.entries.is_empty());
}

#[test]
fn assistant_geometry_program_uses_bounded_apply_and_verify_in_gui_document() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    install_initial_graph_result(&mut app);
    app.selection.occurrences = BTreeSet::from([InstancePath::root(OccurrenceId(1))]);
    let before = app.live_bridge_stamp();
    let undo_before = app.document.visible_undo_steps();
    app.assistant.pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: Some(AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Transform {
                selector: AssistantCadEntitySelector::CurrentSelection {},
                translation_mm: [10.0, 0.0, 0.0],
                rotation: None,
            }],
        }),
        result: AssistantChatResult {
            message: "Moved the selected body and verified it.".to_owned(),
            model_intent: None,
        },
        message: "Move the selected body 10 mm along X.".to_owned(),
        replan_attempted: false,
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.poll_assistant_chat(&egui::Context::default());

    let after = app.live_bridge_stamp();
    let verification = app.assistant.verification.as_ref().unwrap_or_else(|| {
        panic!(
            "bounded Assistant result missing: {}",
            app.assistant
                .messages
                .last()
                .map_or("no result", |message| message.text.as_str())
        )
    });
    assert!(app.assistant.proposal.is_none());
    assert_eq!(after.document_id, before.document_id);
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.mutation_epoch > before.mutation_epoch);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert_eq!(verification.canonical_digest, after.canonical_digest);
    assert_eq!(
        verification.validation_after.as_ref().unwrap()["state"],
        "passed"
    );
    assert!(app.undo());
    assert_eq!(
        app.live_bridge_stamp().canonical_digest,
        before.canonical_digest
    );
}

#[test]
fn ambiguous_assistant_request_surfaces_clarification_without_mutation() {
    let mut app = KetchupApp::new();
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            fea_review: None,
            result: AssistantChatResult {
                message: "Which of the two side panels should I extend?".to_owned(),
                model_intent: None,
            },
            diagnostics: None,
        }))
        .unwrap();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "ambiguous-request".to_owned(),
        message: "Extend the side panel.".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.poll_assistant_chat(&egui::Context::default());

    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );
    assert!(app.assistant.pending_execution.is_none());
    assert!(app.assistant.proposal.is_none());
    assert!(app.assistant.messages.iter().any(|message| {
        message.role == AssistantMessageRole::Assistant
            && message.text == "Which of the two side panels should I extend?"
    }));
}

#[test]
fn assistant_project_memory_retrieval_is_bounded_relevant_and_read_only() {
    let mut app = KetchupApp::new();
    for index in 0..140 {
        app.assistant.memory.remember(
            &format!("Fixture note {index}"),
            &format!("Fixture answer {index}"),
        );
    }
    app.assistant
        .memory
        .remember("Remember shelf spacing", "The shelf spacing is 320 mm.");
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );

    let context = app.assistant_context_for("What is the shelf spacing?");
    let memory = context["project_memory"].as_object().unwrap();
    let entries = memory["entries"].as_array().unwrap();
    assert_eq!(memory["schema"], PROJECT_MEMORY_SCHEMA);
    assert_eq!(
        memory["document_id"],
        app.document.current().document_id().0
    );
    assert_eq!(memory["stored_count"], MAX_PROJECT_MEMORY_STORED_ENTRIES);
    assert!(entries.len() <= MAX_PROJECT_MEMORY_ENTRIES);
    assert!(entries.iter().any(|entry| {
        entry["assistant"] == "The shelf spacing is 320 mm."
            && entry["sha256"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
    }));
    let encoded = serde_json::to_vec(entries).unwrap();
    assert_eq!(memory["byte_length"], encoded.len());
    assert!(encoded.len() <= MAX_PROJECT_MEMORY_CONTEXT_BYTES);
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );
}

#[test]
fn assistant_project_memory_persists_in_its_document_and_rejects_foreign_scope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-memory.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    app.assistant
        .memory
        .remember("Shelf material", "Use birch plywood.");
    assert!(app.save_document_to(&path));

    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_from(&path));
    let context = reopened.assistant_context_for("Which shelf material?");
    let memory = context["project_memory"]["entries"].as_array().unwrap();
    assert_eq!(memory.len(), 1);
    assert_eq!(memory[0]["assistant"], "Use birch plywood.");
    reopened.new_assistant_chat();
    assert_eq!(
        reopened.assistant_context_for("material")["project_memory"]["retrieved_count"],
        1
    );

    let document_id = reopened.document.current().document_id().0;
    let foreign = AssistantProjectMemory::empty(document_id + 1);
    let entry = ketchup_model::persistence::ExtensionEntry::new(
        ASSISTANT_CHAT_NAMESPACE,
        ASSISTANT_MEMORY_PATH,
        false,
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    reopened.file.container_data.set_extension(entry);
    let before = (
        reopened.document.current().revision_id(),
        reopened.document.current().canonical_digest(),
        reopened.document.visible_undo_steps(),
    );
    reopened.load_assistant_memory();
    assert_eq!(reopened.assistant.memory.document_id, document_id);
    assert!(reopened.assistant.memory.entries.is_empty());
    assert_eq!(
        (
            reopened.document.current().revision_id(),
            reopened.document.current().canonical_digest(),
            reopened.document.visible_undo_steps(),
        ),
        before
    );
}

#[test]
fn new_chat_cancels_the_active_assistant_request() {
    let mut app = KetchupApp::new();
    let cancellation = AssistantCancellation::default();
    let (_sender, receiver) = mpsc::channel();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_assistant_chat();

    assert!(cancellation.is_cancelled());
    assert!(app.assistant.chat_task.is_none());
}

#[test]
fn dropping_an_active_assistant_request_cancels_it() {
    let app = KetchupApp::new();
    let cancellation = AssistantCancellation::default();
    let (_sender, receiver) = mpsc::channel();
    let task = AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    };

    drop(task);

    assert!(cancellation.is_cancelled());
}

#[test]
fn new_and_open_cancel_active_assistant_requests() {
    let mut app = KetchupApp::new();
    let new_cancellation = AssistantCancellation::default();
    let (_new_sender, new_receiver) = mpsc::channel();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver: new_receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: new_cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_document();

    assert!(new_cancellation.is_cancelled());
    assert!(app.assistant.chat_task.is_none());

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-open-cancel.ketchup");
    assert!(app.save_document_to(&path));
    let open_cancellation = AssistantCancellation::default();
    let (_open_sender, open_receiver) = mpsc::channel();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver: open_receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: open_cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    assert!(app.open_document_from(&path));

    assert!(open_cancellation.is_cancelled());
    assert!(app.assistant.chat_task.is_none());
}

#[test]
fn assistant_panel_progress_phases_are_accessible_with_deterministic_channels() {
    let mut app = KetchupApp::new();
    let requesting = app.catalog.text("assistant-progress-requesting");
    let elapsed = app.catalog.format(
        "assistant-progress-elapsed",
        &BTreeMap::from([("time", "0:07".to_owned())]),
    );
    let executing = app.catalog.text("assistant-progress-executing");
    assert_ne!(
        assistant_clock_frame(Duration::ZERO),
        assistant_clock_frame(Duration::from_millis(250))
    );
    let (_sender, receiver) = mpsc::channel();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now() - Duration::from_secs(7),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(
            |context, app: &mut KetchupApp| {
                egui::CentralPanel::default().show(context, |ui| app.show_assistant(ui));
                app.poll_assistant_chat(context);
            },
            app,
        );

    harness.step();

    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&requesting)
                        || node.value().as_deref() == Some(&requesting))
            })
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&elapsed)
                        || node.value().as_deref() == Some(&elapsed))
            })
            .next()
            .is_some()
    );
    let state = harness.state_mut();
    state.assistant.chat_task = None;
    state.assistant.pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        message: "test".to_owned(),
        replan_attempted: false,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
                    occurrence_id: 1,
                    delta_mm: [25.0, 0.0, 0.0],
                }],
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
            }),
        },
        document_id: state.document.current().document_id(),
        revision_id: state.document.current().revision_id(),
        canonical_digest: state.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    // The executing phase lasts exactly one frame: the panel announces it,
    // and `poll_assistant_chat` consumes the pending execution at the end of
    // that same frame. `run` would keep stepping while anything still asks
    // for an immediate repaint, so it must not be used to observe a state
    // that is gone by the next frame.
    harness.step();

    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&executing)
                        || node.value().as_deref() == Some(&executing))
            })
            .next()
            .is_some()
    );
    assert!(
        harness.state().assistant.pending_execution.is_none(),
        "the announced execution must be consumed by the frame that announced it"
    );
}

#[test]
fn new_chat_discards_a_pending_assistant_execution_before_commit() {
    let mut app = KetchupApp::new();
    let revision = app.document.current().revision_id();
    app.assistant.pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        message: "test".to_owned(),
        replan_attempted: false,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
                    occurrence_id: 1,
                    delta_mm: [25.0, 0.0, 0.0],
                }],
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
            }),
        },
        document_id: app.document.current().document_id(),
        revision_id: revision,
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_assistant_chat();
    app.poll_assistant_chat(&egui::Context::default());

    assert!(app.assistant.pending_execution.is_none());
    assert_eq!(app.document.current().revision_id(), revision);
    assert!(app.assistant.verification.is_none());
}

#[test]
fn assistant_model_change_requires_explicit_confirmation_after_validation() {
    let mut app = KetchupApp::new();
    let revision = app.document.current().revision_id();
    let (sender, receiver) = mpsc::channel();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: revision,
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            fea_review: None,
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
                        occurrence_id: 1,
                        delta_mm: [25.0, 0.0, 0.0],
                    }],
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                }),
            },
            diagnostics: None,
        }))
        .unwrap();
    let context = egui::Context::default();

    app.poll_assistant_chat(&context);

    assert!(app.assistant.chat_task.is_none());
    assert!(app.assistant.pending_execution.is_some());
    assert_eq!(app.document.current().revision_id(), revision);

    app.poll_assistant_chat(&context);

    assert!(app.assistant.pending_execution.is_none());
    assert_eq!(app.document.current().revision_id(), revision);
    assert!(app.assistant.proposal.is_some());
    assert!(app.assistant.verification.is_none());
    assert!(app.assistant.messages.iter().any(|message| {
        message.role == AssistantMessageRole::Assistant && message.text == "Moved it."
    }));

    assert!(app.confirm_assistant_proposal());
    assert_eq!(app.document.current().revision_id(), revision + 1);
    assert!(app.assistant.verification.is_some());
}

#[test]
fn assistant_replace_scene_clears_collection_references_in_the_same_undo_step() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(1);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Original selection".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: collection,
                occurrence_ids: vec![OccurrenceId(1)],
            },
        ]))
        .unwrap();
    let before = app.document.current();

    assert!(apply_reviewed_model_intent(
        &mut app,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "Replacement".to_owned(),
                size_mm: [100.0, 80.0, 60.0],
                origin_mm: [0.0, 0.0, 0.0],
                subtract_boxes: Vec::new(),
            }],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
        }
    ));

    let replaced = app.document.current();
    assert_eq!(replaced.revision_id(), before.revision_id() + 1);
    assert_eq!(replaced.occurrences().count(), 1);
    assert_eq!(replaced.definitions().count(), 1);
    assert_eq!(
        replaced
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
    assert_eq!(
        app.document.current().canonical_digest(),
        before.canonical_digest()
    );
}

#[test]
fn stale_assistant_model_result_is_reported_without_mutating_the_newer_document() {
    let mut app = KetchupApp::new();
    let request_document_id = app.document.current().document_id();
    let request_revision_id = app.document.current().revision_id();
    let request_digest = app.document.current().canonical_digest();
    let (sender, receiver) = mpsc::channel();
    let cancellation = AssistantCancellation::default();
    app.assistant.chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation,
        document_id: request_document_id,
        revision_id: request_revision_id,
        canonical_digest: request_digest,
        source: "test".to_owned(),
    });
    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetOccurrenceTranslation {
            target: OccurrenceId(1),
            x_mm_text: "10".to_owned(),
            y_mm_text: "0".to_owned(),
            z_mm_text: "0".to_owned(),
        })
    );
    assert!(app.confirm_assistant_proposal());
    let changed_revision = app.document.current().revision_id();
    let changed_digest = app.document.current().canonical_digest();
    let undo_steps = app.document.visible_undo_steps();
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            fea_review: None,
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![ketchup_assistant::sidecar::AssistantTranslationIntent {
                        occurrence_id: 1,
                        delta_mm: [100.0, 0.0, 0.0],
                    }],
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                }),
            },
            diagnostics: None,
        }))
        .unwrap();

    let context = egui::Context::default();
    app.poll_assistant_chat(&context);
    app.poll_assistant_chat(&context);

    assert_eq!(app.document.current().revision_id(), changed_revision);
    assert_eq!(app.document.current().canonical_digest(), changed_digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
    assert!(app.assistant.messages.iter().any(|message| {
        message.role == AssistantMessageRole::Error
            && message.text == app.catalog.text("assistant-error-stale-response")
    }));
}

#[test]
fn assistant_conversation_round_trips_with_its_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chat-model.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    app.assistant.messages = vec![
        AssistantChatMessage {
            role: AssistantMessageRole::User,
            text: "Posuň hranol o 100 mm.".to_owned(),
            source: "Codex OAuth · gpt-test".to_owned(),
            diagnostic: None,
        },
        AssistantChatMessage {
            role: AssistantMessageRole::Error,
            text: "Výskyt 999 neexistuje.".to_owned(),
            source: "Codex OAuth · gpt-test".to_owned(),
            diagnostic: Some(
                *ketchup_application::diagnostics::assistant_canonical_rejection(
                    ketchup_model::document::CanonicalError::OccurrenceNotFound(OccurrenceId(999)),
                    "translate_occurrence",
                    "occurrence:999",
                ),
            ),
        },
    ];

    assert!(app.save_document_to(&path));
    let mut reopened = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    assert!(reopened.open_document_from(&path));
    assert_eq!(reopened.assistant.messages, app.assistant.messages);
    assert_eq!(reopened.file.path.as_deref(), Some(path.as_path()));

    reopened.new_assistant_chat();
    assert!(reopened.assistant.messages.is_empty());
    assert!(reopened.save_document_to(&path));
    let mut cleared = KetchupApp::new();
    assert!(cleared.open_document_from(&path));
    assert!(cleared.assistant.messages.is_empty());
}
