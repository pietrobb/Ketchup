mod harness;

use eframe::egui::{self, accesskit::Role};
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AppCommand, AssistantMessageRole, AssistantProvider, AssistantWorkspaceMode};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantApiDiagnostics, AssistantBeamNotchIntent,
    AssistantBottleFinishKind, AssistantBottleIntent, AssistantBoxIntent, AssistantChatResult,
    AssistantDistribution, AssistantGableRoofIntent, AssistantLinearArrayIntent,
    AssistantModelIntent, AssistantOrientedBeamIntent, AssistantParameterEditIntent,
    AssistantProfileTranslationIntent, AssistantStaircaseIntent, AssistantSubtractionIntent,
    AssistantTeapotIntent, AssistantTranslationIntent,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, NodeId, OccurrenceId, ProposalGoal, ProposalValue, TagId, Transform,
};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchSpec, WorkplaneSpec,
};
use ketchup_interaction::{LocaleCatalog, Vec3};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn apply_reviewed_model_intent(shell: &mut Shell, intent: AssistantModelIntent) -> bool {
    shell.app_mut().prepare_assistant_model_intent(intent)
        && shell.app_mut().confirm_assistant_proposal()
}

fn apply_reviewed_evaluator_inputs(shell: &mut Shell, inputs: &[(&str, f64)]) {
    for (index, (name, value)) in inputs.iter().enumerate() {
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateEvaluatorInput {
                    target: NodeId(900 + index as u64),
                    name: (*name).to_owned(),
                    value_text: value.to_string(),
                },)
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
    }
}

fn write_assistant_movable_pocket_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Furniture panel".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DefinitionId(1),
                name: "Panel outline".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DefinitionId(1),
                name: "10 mm panel".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(12),
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(14),
                definition_id: DefinitionId(1),
                name: "Fitting pocket profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 16.0], [10.0, 16.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(15),
                definition_id: DefinitionId(1),
                name: "6 mm fitting pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: FeatureId(13),
                    profile: FeatureId(14),
                    depth: Dimension::from_decimal("6").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Furniture panel".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn write_assistant_parameter_fixture(path: &std::path::Path) {
    let sketch = SketchSpec {
        workplane: FeatureId(11),
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [4.0, 5.0],
            radius_mm: 3.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::from_decimal("3").unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [4.0, 5.0],
                },
            },
        ],
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Editable circle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(1),
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DefinitionId(1),
                name: "Circle".to_owned(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DefinitionId(1),
                name: "Pad".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: FeatureId(12),
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("5").unwrap()),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Editable circle".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn wait_for_assistant_proposal(shell: &mut Shell) {
    let confirm = shell.catalog().text("assistant-confirm");
    for _ in 0..100 {
        shell.step();
        if shell.app().assistant_proposal().is_some() && shell.has_visible_label(&confirm) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("scripted assistant response did not reach accessible proposal review");
}

#[test]
fn assistant_enter_sends_and_shift_enter_keeps_composing() {
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        "Move the beam\nup 20 mm".to_owned(),
        AssistantChatResult {
            message: "Scripted answer".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let input_label = shell.catalog().text("assistant-input-hint");

    shell.focus_text_input(&input_label);
    shell.type_text("Move the beam");
    shell.key(egui::Key::Enter, egui::Modifiers::SHIFT);
    assert!(
        shell.app().assistant_messages().is_empty(),
        "Shift+Enter must not send the draft"
    );

    shell.type_text("up 20 mm");
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let messages = shell.app().assistant_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, AssistantMessageRole::User);
    assert_eq!(messages[0].text, "Move the beam\nup 20 mm");
    assert_eq!(messages[1].role, AssistantMessageRole::Assistant);
    assert_eq!(messages[1].text, "Scripted answer");
    assert_eq!(transport.remaining_responses(), 0);

    let new_chat = shell.catalog().text("assistant-new-chat");
    shell.click_row(&new_chat);
    assert!(shell.app().assistant_messages().is_empty());
}

#[test]
fn assistant_diagnostics_show_exact_api_usage_and_search_project_memory() {
    let request = "Remember that rafter spacing is 600 mm";
    let answer = "The rafter spacing is 600 mm.";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: answer.to_owned(),
            model_intent: None,
        },
    )]));
    transport.queue_diagnostics(AssistantApiDiagnostics {
        provider: "codex-oauth".to_owned(),
        model: "gpt-5.6-sol".to_owned(),
        duration_ms: 1_250,
        input_tokens: 1_234,
        output_tokens: 56,
        cache_read_tokens: 700,
        cache_write_tokens: 0,
        stop_reason: "completed".to_owned(),
        system_prompt: "Exact Kečup system prompt".to_owned(),
        request_payload: serde_json::json!({
            "model": "gpt-5.6-sol",
            "instructions": "Exact Kečup system prompt",
            "input": [{"role": "user", "content": request}],
        }),
        response_text: serde_json::json!({
            "message": answer,
            "model_intent": null,
        })
        .to_string(),
    });
    let mut shell = Shell::with_assistant_transport(transport);
    let diagnostics_title = shell.catalog().text("assistant-diagnostics-title");
    shell.click_button_label(&diagnostics_title);
    let capture = shell.catalog().text("assistant-diagnostics-capture");
    shell.click_role_and_label(Role::CheckBox, &capture);
    assert!(
        shell
            .app()
            .assistant_handshake()
            .capabilities
            .contains(&ketchup_core::assistant_sidecar::AssistantCapability::DebugObservability)
    );

    let input_label = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input_label);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shell.app().assistant_messages()[1].text, answer);
    shell.step();
    let token_summary = shell.catalog().format(
        "assistant-diagnostics-token-summary",
        &BTreeMap::from([
            ("input", "1234".to_owned()),
            ("output", "56".to_owned()),
            ("cache", "700".to_owned()),
            ("total", "1290".to_owned()),
            ("duration", "1.25".to_owned()),
        ]),
    );
    assert!(shell.has_visible_label(&token_summary));
    let exact_payload = serde_json::to_string_pretty(&serde_json::json!({
        "model": "gpt-5.6-sol",
        "instructions": "Exact Kečup system prompt",
        "input": [{"role": "user", "content": request}],
    }))
    .unwrap();
    assert!(shell.has_visible_label(&exact_payload));

    let memory_tab = shell.catalog().text("assistant-diagnostics-memory");
    shell.click_button_label(&memory_tab);
    let memory_search = shell.catalog().text("assistant-memory-search");
    shell.focus_text_input(&memory_search);
    shell.type_text("rafter");
    shell.step();
    assert!(shell.has_visible_label(answer));
}

#[test]
fn scripted_assistant_in_flight_requests_cancel_and_transport_survives_new_document() {
    let transport = Arc::new(
        ScriptedAssistantTransport::new([(
            "After New".to_owned(),
            AssistantChatResult {
                message: "Still scripted".to_owned(),
                model_intent: None,
            },
        )])
        .with_cancellation_request("Cancel by new chat")
        .with_cancellation_request("Cancel by new document"),
    );
    let dialogs = ScriptedFileDialogs::new().always_discard();
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport.clone());
    let input_label = shell.catalog().text("assistant-input-hint");

    shell.focus_text_input(&input_label);
    shell.type_text("Cancel by new chat");
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        if transport.started_cancellation_requests() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(transport.started_cancellation_requests(), 1);
    let new_chat = shell.catalog().text("assistant-new-chat");
    shell.click_row(&new_chat);
    for _ in 0..100 {
        if transport.completed_cancellations() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(transport.completed_cancellations(), 1);
    assert!(shell.app().assistant_messages().is_empty());

    shell.focus_text_input(&input_label);
    shell.type_text("Cancel by new document");
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        if transport.started_cancellation_requests() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(transport.started_cancellation_requests(), 2);
    shell.click_menu_command("menu-file", AppCommand::New);
    for _ in 0..100 {
        if transport.completed_cancellations() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(transport.completed_cancellations(), 2);
    assert!(shell.app().assistant_messages().is_empty());

    shell.focus_text_input(&input_label);
    shell.type_text("After New");
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(shell.app().assistant_messages().len(), 2);
    assert_eq!(shell.app().assistant_messages()[1].text, "Still scripted");
    assert_eq!(transport.remaining_responses(), 0);
    assert_eq!(
        transport.request_ids(),
        ["chat-1", "chat-3", "chat-4"].map(str::to_owned)
    );
}

#[test]
fn injected_assistant_results_are_validated_fail_closed() {
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        "Invalid result".to_owned(),
        AssistantChatResult {
            message: String::new(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let input_label = shell.catalog().text("assistant-input-hint");

    shell.focus_text_input(&input_label);
    shell.type_text("Invalid result");
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let messages = shell.app().assistant_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].role, AssistantMessageRole::Error);
    assert_eq!(messages[1].text, "assistant returned an empty message");
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn scripted_assistant_model_review_cancel_confirm_undo_and_redo_use_accesskit() {
    let scripted_result = |name: &str| AssistantChatResult {
        message: format!("Review {name}"),
        model_intent: Some(AssistantModelIntent {
            replace_scene: false,
            boxes: vec![AssistantBoxIntent {
                name: name.to_owned(),
                size_mm: [120.0, 80.0, 10.0],
                origin_mm: [0.0, 0.0, 0.0],
                subtract_boxes: Vec::new(),
            }],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }),
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            "Create a box, then let me review it".to_owned(),
            scripted_result("Cancelled scripted box"),
        ),
        (
            "Create the reviewed box".to_owned(),
            scripted_result("Confirmed scripted box"),
        ),
    ]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let input_label = shell.catalog().text("assistant-input-hint");
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_definitions = shell.app().definition_count();
    let initial_occurrences = shell.app().active_box_count();

    shell.focus_text_input(&input_label);
    shell.type_text("Create a box, then let me review it");
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    let cancel = shell.catalog().text("assistant-cancel");
    shell.click_row(&cancel);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), initial_revision);

    shell.focus_text_input(&input_label);
    shell.type_text("Create the reviewed box");
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().definition_count(), initial_definitions + 1);
    assert_eq!(shell.app().active_box_count(), initial_occurrences + 1);
    let committed_digest = shell.app().canonical_digest();
    assert_ne!(committed_digest, initial_digest);

    let undo_change = shell.catalog().text("assistant-undo-change");
    shell.click_row(&undo_change);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().definition_count(), initial_definitions);
    assert_eq!(shell.app().active_box_count(), initial_occurrences);

    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().definition_count(), initial_definitions + 1);
    assert_eq!(shell.app().active_box_count(), initial_occurrences + 1);
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn verbal_bottle_request_reviews_confirms_and_undoes_through_accesskit() {
    let request = "Create an editable ketchup bottle with a 30 mm body radius";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the editable bottle.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: vec![AssistantBottleIntent {
                    name: "Verbally created bottle".to_owned(),
                    body_radius_mm: 30.0,
                    body_height_mm: 110.0,
                    shoulder_rise_mm: 20.0,
                    neck_radius_mm: 12.0,
                    neck_height_mm: 25.0,
                    wall_thickness_mm: 2.0,
                    finish_kind: AssistantBottleFinishKind::Fillet,
                    finish_amount_mm: 2.0,
                    origin_mm: [90.0, 0.0, 0.0],
                    teapot: None,
                }],
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            }),
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_features = shell.app().feature_count();
    let initial_occurrences = shell.app().occurrence_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().feature_count(), initial_features + 5);
    assert_eq!(shell.app().occurrence_count(), initial_occurrences + 1);
    assert!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .any(|occurrence| occurrence.name() == "Verbally created bottle occurrence")
    );

    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn assistant_profile_translation_reviews_confirms_undoes_and_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory
        .path()
        .join("assistant-profile-translation.ketchup");
    write_assistant_movable_pocket_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);

    let profile_points = |shell: &Shell| match shell
        .app()
        .document_snapshot()
        .feature(FeatureId(14))
        .unwrap()
        .kind()
    {
        FeatureKind::Profile { points_mm } => points_mm.clone(),
        other => panic!("expected movable profile, got {other:?}"),
    };
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let before_points = profile_points(&shell);
    let occurrence_transform = shell
        .app()
        .document_snapshot()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: vec![AssistantProfileTranslationIntent {
                definition_id: 1,
                body_id: BodyId(1).0,
                profile_id: 14,
                delta_mm: [2.0, 3.0],
            }],
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        profile_points(&shell),
        before_points
            .iter()
            .map(|point| [point[0] + 2.0, point[1] + 3.0])
            .collect::<Vec<_>>()
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform(),
        occurrence_transform
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(profile_points(&shell), before_points);

    assert!(
        !shell
            .app_mut()
            .prepare_assistant_model_intent(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: Vec::new(),
                profile_translations: vec![AssistantProfileTranslationIntent {
                    definition_id: 1,
                    body_id: 1,
                    profile_id: 14,
                    delta_mm: [100.0, 0.0],
                }],
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            })
    );
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().assistant_proposal().is_none());
}

#[test]
fn assistant_parameter_edit_uses_the_selected_exact_target_for_feature_and_constraint() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("assistant-parameter-edit.ketchup");
    write_assistant_parameter_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let history = shell.catalog().text("feature-history-title");
    shell.click_role_and_label(Role::Button, &history);

    let before_digest = shell.app().canonical_digest();
    let context = shell.app().assistant_context();
    assert_eq!(
        context["selected_parameter_edit_target"],
        serde_json::json!({
            "definition_id": 1,
            "body_id": 1,
            "feature_id": 13,
            "constraint_id": null,
            "name": shell.catalog().text("feature-history-parameter-extent"),
            "current_value_mm": 5.0,
        })
    );
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: vec![AssistantParameterEditIntent {
                definition_id: 1,
                body_id: 1,
                feature_id: 13,
                constraint_id: None,
                value_mm: 7.5,
            }],
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(FeatureId(13))
            .unwrap()
            .kind(),
        FeatureKind::Pad(spec) if spec.extent.distance().millimetres() == 7.5
    ));
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    let feature = shell
        .app()
        .document_snapshot()
        .feature(FeatureId(12))
        .unwrap()
        .clone();
    let sketch_label = shell.catalog().format(
        "feature-history-select-feature",
        &BTreeMap::from([
            ("name", feature.name().to_owned()),
            ("id", "12".to_owned()),
            (
                "status",
                shell.catalog().text("feature-history-state-active"),
            ),
        ]),
    );
    shell.click_role_and_label(Role::Button, &sketch_label);
    let context = shell.app().assistant_context();
    assert_eq!(context["selected_parameter_edit_target"]["feature_id"], 12);
    assert_eq!(
        context["selected_parameter_edit_target"]["constraint_id"],
        1
    );
    assert_eq!(
        context["selected_parameter_edit_target"]["current_value_mm"],
        3.0
    );

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: vec![AssistantParameterEditIntent {
                definition_id: 1,
                body_id: 1,
                feature_id: 12,
                constraint_id: Some(1),
                value_mm: 4.5,
            }],
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));
    let snapshot = shell.app().document_snapshot();
    let FeatureKind::Sketch(sketch) = snapshot.feature(FeatureId(12)).unwrap().kind() else {
        panic!("expected editable sketch")
    };
    assert!(matches!(
        &sketch.constraints[0].kind,
        SketchConstraintKind::Radius { value, .. } if value.millimetres() == 4.5
    ));
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    assert!(
        !shell
            .app_mut()
            .prepare_assistant_model_intent(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: vec![AssistantParameterEditIntent {
                    definition_id: 1,
                    body_id: 1,
                    feature_id: 12,
                    constraint_id: Some(2),
                    value_mm: 4.5,
                }],
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            })
    );
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().assistant_proposal().is_none());
}

#[test]
fn assistant_selection_context_tracks_the_live_model_selection() {
    let mut shell = Shell::new();
    let none = shell.catalog().text("assistant-selection-none");
    assert!(shell.has_visible_label(&none));
    assert_eq!(
        shell.app().assistant_context()["selected_occurrence_ids"],
        serde_json::json!([])
    );

    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    let context = shell.app().assistant_context();
    assert_eq!(context["selected_occurrence_ids"], serde_json::json!([1]));
    let name = context["occurrences"][0]["name"].as_str().unwrap();
    let selected = shell.catalog().format(
        "assistant-selection-one",
        &BTreeMap::from([("name", name.to_owned())]),
    );
    assert!(shell.has_visible_label(&selected));

    shell.click_menu_command("menu-edit", AppCommand::Deselect);
    assert_eq!(
        shell.app().assistant_context()["selected_occurrence_ids"],
        serde_json::json!([])
    );
    assert!(shell.has_visible_label(&none));
}

#[test]
fn assistant_context_runs_current_collision_validation_without_mutating_or_adding_undo() {
    let mut shell = Shell::new();
    let subtraction = AssistantSubtractionIntent {
        size_mm: [10.0, 10.0, 10.0],
        origin_mm: [10.0, 10.0, 10.0],
    };
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Left cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: vec![subtraction.clone()],
                },
                AssistantBoxIntent {
                    name: "Right cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [50.0, 0.0, 0.0],
                    subtract_boxes: vec![subtraction],
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    let context = shell.app().assistant_context();
    let validation = &context["validation"];
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], false);
    assert_eq!(validation["collision"]["state"], "failed");
    assert_eq!(validation["collision"]["complete"], true);
    assert_eq!(validation["checked_occurrence_count"], 2);
    assert_eq!(validation["checked_pair_count"], 1);
    assert_eq!(validation["issue_count"], 1);
    assert_eq!(validation["issues"][0]["code"], "collision.detected");
    assert_eq!(validation["issues"][0]["left_name"], "Left cabinet");
    assert_eq!(validation["issues"][0]["right_name"], "Right cabinet");
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let right_id = validation["issues"][0]["right_occurrence_id"]
        .as_u64()
        .unwrap();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: vec![AssistantTranslationIntent {
                occurrence_id: right_id,
                delta_mm: [50.0, 0.0, 0.0],
            }],
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    let clean_revision = shell.app().document_revision();
    let clean_undo_steps = shell.app().undo_step_count();
    let clean = shell.app().assistant_context();
    assert_eq!(clean["validation"]["state"], "not_evaluated");
    assert_eq!(clean["validation"]["complete"], false);
    assert_eq!(clean["validation"]["collision"]["state"], "passed");
    assert_eq!(clean["validation"]["collision"]["complete"], true);
    assert_eq!(clean["validation"]["issue_count"], 0);
    assert_eq!(shell.app().document_revision(), clean_revision);
    assert_eq!(shell.app().undo_step_count(), clean_undo_steps);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell.app().assistant_context()["validation"]["state"],
        "failed"
    );
}

#[test]
fn assistant_context_finds_transitively_supported_and_floating_parts_without_mutation() {
    let mut shell = Shell::new();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Floor base".to_owned(),
                    size_mm: [100.0, 100.0, 10.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Supported shelf".to_owned(),
                    size_mm: [80.0, 80.0, 10.0],
                    origin_mm: [10.0, 10.0, 10.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Supported load".to_owned(),
                    size_mm: [60.0, 60.0, 10.0],
                    origin_mm: [20.0, 20.0, 20.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Floating load".to_owned(),
                    size_mm: [20.0, 20.0, 20.0],
                    origin_mm: [200.0, 0.0, 50.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let context = shell.app().assistant_context();
    let gravity = &context["validation"]["gravity_support"];
    assert_eq!(gravity["state"], "failed", "{context:#}");
    assert_eq!(gravity["complete"], true);
    assert_eq!(gravity["checked_occurrence_count"], 4);
    assert_eq!(gravity["unsupported_count"], 1);
    assert_eq!(gravity["issues"][0]["code"], "gravity.unsupported");
    assert_eq!(gravity["issues"][0]["name"], "Floating load");
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let floating_id = gravity["issues"][0]["occurrence_id"].as_u64().unwrap();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: vec![AssistantTranslationIntent {
                occurrence_id: floating_id,
                delta_mm: [0.0, 0.0, -50.0],
            }],
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let supported = shell.app().assistant_context();
    assert_eq!(
        supported["validation"]["gravity_support"]["state"],
        "passed"
    );
    assert_eq!(
        supported["validation"]["gravity_support"]["unsupported_count"],
        0
    );
}

#[test]
fn assistant_chat_selects_validation_scope_and_rejects_unknown_names_without_mutation() {
    let queries = [
        "Skontroluj všetky validátory",
        "Iba kolízie a podopretie",
        "Všetky okrem podopretia",
        "Iba kolízie a vibrácie",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new(queries.map(|query| {
        (
            query.to_owned(),
            AssistantChatResult {
                message: "Validation scope received".to_owned(),
                model_intent: None,
            },
        )
    })));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let input_label = shell.catalog().text("assistant-input-hint");
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    for (index, query) in queries.iter().enumerate() {
        shell.focus_text_input(&input_label);
        shell.type_text(query);
        shell.press_key(egui::Key::Enter);
        for _ in 0..100 {
            shell.step();
            if transport.contexts().len() > index
                && shell.app().assistant_messages().len() == (index + 1) * 2
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 4);
    let all = &contexts[0]["validation"];
    assert_eq!(all["selection_mode"], "all");
    assert_eq!(
        all["executed"],
        serde_json::json!([
            "collision",
            "gravity_support",
            "shelf_deflection",
            "tipping",
            "anchoring",
            "hardware_manufacturing",
            "room_placement",
            "passage_clearance",
            "static_load"
        ])
    );
    assert_eq!(all["skipped"], serde_json::json!([]));

    let only = &contexts[1]["validation"];
    assert_eq!(only["selection_mode"], "only");
    assert_eq!(
        only["requested"],
        serde_json::json!(["collision", "gravity_support"])
    );
    assert_eq!(only["executed"], only["requested"]);

    let except = &contexts[2]["validation"];
    assert_eq!(except["selection_mode"], "all_except");
    assert_eq!(
        except["executed"],
        serde_json::json!([
            "collision",
            "shelf_deflection",
            "tipping",
            "anchoring",
            "hardware_manufacturing",
            "room_placement",
            "passage_clearance",
            "static_load"
        ])
    );
    assert_eq!(except["skipped"], serde_json::json!(["gravity_support"]));
    assert_eq!(except["gravity_support"]["state"], "skipped");

    let unknown = &contexts[3]["validation"];
    assert_eq!(unknown["requested"], serde_json::json!(["collision"]));
    assert_eq!(unknown["executed"], serde_json::json!([]));
    assert_eq!(
        unknown["skipped"],
        serde_json::json!([
            "collision",
            "gravity_support",
            "shelf_deflection",
            "tipping",
            "anchoring",
            "hardware_manufacturing",
            "room_placement",
            "passage_clearance",
            "static_load"
        ])
    );
    assert_eq!(unknown["not_evaluated"][0]["validator"], "vibrácie");
    assert_eq!(unknown["not_evaluated"][0]["reason"], "unknown_validator");
    assert_eq!(unknown["collision"]["state"], "skipped");
    assert_eq!(unknown["gravity_support"]["state"], "skipped");
    assert_eq!(
        unknown["selection_error"],
        "unknown_or_empty_validator_selection"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_reports_shelf_deflection_tipping_and_anchoring_with_explicit_limits() {
    let query = "Iba priehyb, prevrátenie a kotvenie";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Furniture validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Weak shelf".to_owned(),
                    size_mm: [1_000.0, 300.0, 12.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Tall cabinet".to_owned(),
                    size_mm: [300.0, 400.0, 1_800.0],
                    origin_mm: [1_500.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Low cabinet".to_owned(),
                    size_mm: [800.0, 500.0, 600.0],
                    origin_mm: [2_500.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(
        validation["requested"],
        serde_json::json!(["shelf_deflection", "tipping", "anchoring"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(
        validation["skipped"],
        serde_json::json!([
            "collision",
            "gravity_support",
            "hardware_manufacturing",
            "room_placement",
            "passage_clearance",
            "static_load"
        ])
    );
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 3);

    let shelf = &validation["shelf_deflection"];
    assert_eq!(shelf["state"], "failed");
    assert_eq!(shelf["complete"], true);
    assert_eq!(shelf["applicable_count"], 1);
    assert_eq!(shelf["issue_count"], 1);
    assert_eq!(
        shelf["issues"][0]["code"],
        "furniture.shelf_deflection_exceeded"
    );
    assert_eq!(shelf["issues"][0]["name"], "Weak shelf");
    assert_eq!(shelf["inputs"]["design_load_n"], 500.0);
    assert_eq!(shelf["inputs"]["elastic_modulus_n_mm2"], 2_500.0);
    assert!(
        shelf["issues"][0]["predicted_deflection_mm"]
            .as_f64()
            .unwrap()
            > shelf["issues"][0]["allowable_deflection_mm"]
                .as_f64()
                .unwrap()
    );

    let tipping = &validation["tipping"];
    assert_eq!(tipping["state"], "failed");
    assert_eq!(tipping["applicable_count"], 2);
    assert_eq!(tipping["issue_count"], 1);
    assert_eq!(
        tipping["issues"][0]["code"],
        "furniture.tip_angle_below_limit"
    );
    assert_eq!(tipping["issues"][0]["name"], "Tall cabinet");
    assert_eq!(tipping["limit"]["minimum_tip_angle_degrees"], 15.0);

    let anchoring = &validation["anchoring"];
    assert_eq!(anchoring["state"], "failed");
    assert_eq!(anchoring["applicable_count"], 2);
    assert_eq!(anchoring["required_count"], 1);
    assert_eq!(anchoring["issues"][0]["code"], "furniture.anchor_required");
    assert_eq!(anchoring["issues"][0]["name"], "Tall cabinet");
    assert_eq!(
        anchoring["issues"][0]["anchor_declaration"],
        "not_available_in_current_document_schema"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_reports_hardware_and_manufacturing_rules_with_named_elements_and_limits() {
    let query = "Iba pánty, výsuvy, diery a hrany";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Manufacturing validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Door panel".to_owned(),
                    size_mm: [600.0, 500.0, 18.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Hinge cup upper".to_owned(),
                    size_mm: [30.0, 30.0, 10.0],
                    origin_mm: [1.0, 50.0, 4.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Drill hole mounting".to_owned(),
                    size_mm: [10.0, 10.0, 18.0],
                    origin_mm: [28.0, 50.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Left drawer slide".to_owned(),
                    size_mm: [500.0, 12.0, 45.0],
                    origin_mm: [0.0, 600.0, 100.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Right drawer slide".to_owned(),
                    size_mm: [480.0, 12.0, 45.0],
                    origin_mm: [0.0, 700.0, 105.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Thin back panel".to_owned(),
                    size_mm: [600.0, 500.0, 4.0],
                    origin_mm: [1_000.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(
        validation["requested"],
        serde_json::json!(["hardware_manufacturing"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    let manufacturing = &validation["hardware_manufacturing"];
    assert_eq!(manufacturing["state"], "failed");
    assert_eq!(manufacturing["complete"], true);
    assert_eq!(manufacturing["issue_count"], 5);
    assert_eq!(manufacturing["not_evaluated"], serde_json::json!([]));
    assert_eq!(
        manufacturing["limits"]["minimum_hole_edge_material_mm"],
        5.0
    );
    assert_eq!(
        manufacturing["limits"]["minimum_hole_spacing_material_mm"],
        3.0
    );
    assert_eq!(
        manufacturing["limits"]["minimum_hinge_cup_diameter_mm"],
        35.0
    );
    assert_eq!(manufacturing["limits"]["minimum_hinge_cup_depth_mm"], 12.0);
    assert_eq!(manufacturing["limits"]["minimum_panel_thickness_mm"], 6.0);
    let issues = manufacturing["issues"].as_array().unwrap();
    let issue = |code: &str| {
        issues
            .iter()
            .find(|issue| issue["code"] == code)
            .unwrap_or_else(|| panic!("missing {code} in {issues:#?}"))
    };
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["name"],
        "Thin back panel"
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["name"],
        "Hinge cup upper"
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["host_name"],
        "Door panel"
    );
    assert_eq!(
        issue("manufacturing.hinge_cup_envelope_below_minimum")["name"],
        "Hinge cup upper"
    );
    assert_eq!(
        issue("manufacturing.hole_spacing_below_minimum")["left_name"],
        "Hinge cup upper"
    );
    assert_eq!(
        issue("manufacturing.drawer_slide_pair_misaligned")["left_name"],
        "Left drawer slide"
    );
    assert_eq!(
        issue("manufacturing.drawer_slide_pair_misaligned")["right_name"],
        "Right drawer slide"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_reports_room_placement_and_blocked_passages_from_named_envelopes() {
    let query = "Iba umiestnenie v miestnosti a priechodnosť";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Room validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Room envelope living room".to_owned(),
                    size_mm: [4_000.0, 3_000.0, 2_500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Inside cabinet".to_owned(),
                    size_mm: [600.0, 400.0, 1_800.0],
                    origin_mm: [200.0, 200.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Outside table".to_owned(),
                    size_mm: [300.0, 600.0, 800.0],
                    origin_mm: [3_900.0, 100.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Passage to door".to_owned(),
                    size_mm: [800.0, 2_500.0, 1_900.0],
                    origin_mm: [1_000.0, 250.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Sofa obstacle".to_owned(),
                    size_mm: [500.0, 800.0, 900.0],
                    origin_mm: [1_200.0, 1_000.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(
        validation["requested"],
        serde_json::json!(["room_placement", "passage_clearance"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 3);

    let placement = &validation["room_placement"];
    assert_eq!(placement["state"], "failed");
    assert_eq!(placement["complete"], true);
    assert_eq!(placement["applicable_count"], 3);
    assert_eq!(placement["issue_count"], 1);
    assert_eq!(
        placement["issues"][0]["code"],
        "room.furniture_outside_boundary"
    );
    assert_eq!(placement["issues"][0]["name"], "Outside table");
    assert_eq!(
        placement["issues"][0]["room_name"],
        "Room envelope living room"
    );
    assert_eq!(placement["issues"][0]["outside_by_mm"]["right"], 200.0);

    let passage = &validation["passage_clearance"];
    assert_eq!(passage["state"], "failed");
    assert_eq!(passage["complete"], true);
    assert_eq!(passage["applicable_count"], 1);
    assert_eq!(passage["issue_count"], 2);
    assert_eq!(passage["limits"]["minimum_width_mm"], 900.0);
    assert_eq!(passage["limits"]["minimum_headroom_mm"], 2_000.0);
    let issues = passage["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue["code"] == "room.passage_envelope_below_minimum" && issue["name"] == "Passage to door"
    }));
    assert!(issues.iter().any(|issue| {
        issue["code"] == "room.passage_blocked" && issue["obstacle_name"] == "Sofa obstacle"
    }));
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_does_not_claim_room_validation_without_named_envelopes() {
    let query = "Iba umiestnenie v miestnosti a priechodnosť";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Room inputs unavailable".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(validation["complete"], false);
    assert_eq!(
        validation["room_placement"]["not_evaluated"][0]["reason"],
        "named_room_envelope_not_found"
    );
    assert_eq!(
        validation["passage_clearance"]["not_evaluated"][0]["reason"],
        "named_passage_envelope_not_found"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_calculates_static_load_from_explicit_canonical_physics_inputs() {
    let query = "Iba statika a sily";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Static load validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Loaded machine".to_owned(),
                    size_mm: [500.0, 500.0, 500.0],
                    origin_mm: [0.0, 0.0, 500.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Steel support".to_owned(),
                    size_mm: [500.0, 500.0, 500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    let scene = shell.app().document_snapshot().scene_query();
    let loaded_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Loaded machine")
        .unwrap()
        .occurrence_id
        .0;
    let support_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Steel support")
        .unwrap()
        .occurrence_id
        .0;
    let mass_name = format!("physics.mass_kg.occurrence.{loaded_id}");
    let load_name = format!("physics.applied_load_n.occurrence.{loaded_id}");
    let link_name = format!("physics.support_link.load.{loaded_id}.support.{support_id}");
    let capacity_name = format!("physics.support_capacity_n.occurrence.{support_id}");
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
            (&mass_name, 100.0),
            (&load_name, 200.0),
            (&link_name, 1.0),
            (&capacity_name, 1_000.0),
        ],
    );
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(validation["requested"], serde_json::json!(["static_load"]));
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 1);
    let report = &validation["static_load"];
    assert_eq!(report["state"], "failed");
    assert_eq!(report["complete"], true);
    assert_eq!(report["applicable_count"], 1);
    assert_eq!(report["issue_count"], 1);
    let evaluation = &report["evaluations"][0];
    assert_eq!(evaluation["occurrence_id"], loaded_id);
    assert_eq!(evaluation["name"], "Loaded machine");
    assert_eq!(evaluation["mass"]["value_kg"], 100.0);
    assert_eq!(evaluation["applied_load"]["value_n"], 200.0);
    assert_eq!(
        evaluation["gravity"]["vector_m_s2"],
        serde_json::json!([0.0, 0.0, -9.81])
    );
    assert_eq!(
        evaluation["gravity"]["direction"],
        serde_json::json!([0.0, 0.0, -1.0])
    );
    assert_eq!(evaluation["weight_force_n"], 981.0);
    assert_eq!(evaluation["resultant_force_n"], 1_181.0);
    assert_eq!(evaluation["total_support_capacity_n"], 1_000.0);
    assert_eq!(evaluation["capacity_margin_n"], -181.0);
    assert_eq!(evaluation["supports"][0]["occurrence_id"], support_id);
    assert_eq!(evaluation["supports"][0]["name"], "Steel support");
    assert_eq!(
        report["issues"][0]["code"],
        "physics.support_capacity_exceeded"
    );
    assert_eq!(report["issues"][0]["capacity_shortfall_n"], 181.0);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_does_not_estimate_static_load_without_explicit_physics_inputs() {
    let query = "Iba statika a sily";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Physics inputs unavailable".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(query);
    shell.press_key(egui::Key::Enter);
    for _ in 0..100 {
        shell.step();
        if transport.contexts().len() == 1 && shell.app().assistant_messages().len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let contexts = transport.contexts();
    assert_eq!(contexts.len(), 1);
    let validation = &contexts[0]["validation"];
    assert_eq!(validation["requested"], serde_json::json!(["static_load"]));
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(validation["complete"], false);
    assert_eq!(validation["issue_count"], 0);
    assert_eq!(validation["static_load"]["state"], "not_evaluated");
    assert_eq!(
        validation["static_load"]["not_evaluated"][0]["reason"],
        "missing_or_ambiguous_gravity_vector"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_repairs_a_collision_through_preview_confirmation_revalidation_and_one_undo() {
    let mut shell = Shell::new();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Left cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Right cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [50.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert_eq!(
        shell.app().assistant_context()["validation"]["collision"]["issue_count"],
        1
    );

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text("Oprav iba kolízie");
    shell.press_key(egui::Key::Enter);
    shell.settle();

    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-repair-preview-title")));

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    let verification = shell
        .app()
        .assistant_verification()
        .expect("repair confirmation returns validation evidence");
    assert_eq!(verification.repair_validator.as_deref(), Some("collision"));
    let before = verification.validation_before.as_ref().unwrap();
    let after = verification.validation_after.as_ref().unwrap();
    assert_eq!(before["requested"], serde_json::json!(["collision"]));
    assert_eq!(before["collision"]["issue_count"], 1);
    assert_eq!(before["gravity_support"]["state"], "skipped");
    assert_eq!(after["requested"], before["requested"]);
    assert_eq!(after["collision"]["issue_count"], 0);
    assert_eq!(after["gravity_support"]["state"], "skipped");
    assert_eq!(after["state"], "passed");
    assert!(shell.app().assistant_change_can_undo());

    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    shell.settle();
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(
        shell.app().assistant_context()["validation"]["collision"]["issue_count"],
        1
    );
    assert!(!shell.app().assistant_change_can_undo());
}

#[test]
fn assistant_repairs_an_unsupported_part_and_reruns_only_gravity_support() {
    let mut shell = Shell::new();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "Floating shelf".to_owned(),
                size_mm: [100.0, 30.0, 18.0],
                origin_mm: [0.0, 0.0, 50.0],
                subtract_boxes: Vec::new(),
            }],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert_eq!(
        shell.app().assistant_context()["validation"]["gravity_support"]["unsupported_count"],
        1
    );

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text("Oprav iba podopretie");
    shell.press_key(egui::Key::Enter);
    shell.settle();

    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    let verification = shell
        .app()
        .assistant_verification()
        .expect("support repair returns validation evidence");
    assert_eq!(
        verification.repair_validator.as_deref(),
        Some("gravity_support")
    );
    let before = verification.validation_before.as_ref().unwrap();
    let after = verification.validation_after.as_ref().unwrap();
    assert_eq!(before["requested"], serde_json::json!(["gravity_support"]));
    assert_eq!(before["collision"]["state"], "skipped");
    assert_eq!(before["gravity_support"]["unsupported_count"], 1);
    assert_eq!(after["requested"], before["requested"]);
    assert_eq!(after["collision"]["state"], "skipped");
    assert_eq!(after["gravity_support"]["unsupported_count"], 0);
    assert_eq!(after["state"], "passed");
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);

    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    shell.settle();
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(
        shell.app().assistant_context()["validation"]["gravity_support"]["unsupported_count"],
        1
    );
}

#[test]
fn assistant_repairs_all_safe_collision_and_support_findings_in_one_confirmed_batch() {
    let mut shell = Shell::new();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Left cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Right cabinet".to_owned(),
                    size_mm: [100.0, 100.0, 100.0],
                    origin_mm: [50.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Floating shelf".to_owned(),
                    size_mm: [100.0, 30.0, 18.0],
                    origin_mm: [300.0, 0.0, 50.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ));
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    let validation = &shell.app().assistant_context()["validation"];
    assert_eq!(validation["collision"]["issue_count"], 1);
    assert_eq!(validation["gravity_support"]["unsupported_count"], 1);

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text("Oprav iba kolízie a podopretie");
    shell.press_key(egui::Key::Enter);
    shell.settle();

    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    let verification = shell
        .app()
        .assistant_verification()
        .expect("batch repair returns validation evidence");
    assert_eq!(
        verification.repair_validator.as_deref(),
        Some("collision + gravity_support")
    );
    assert_eq!(verification.verified_write_count, 2);
    let before = verification.validation_before.as_ref().unwrap();
    let after = verification.validation_after.as_ref().unwrap();
    assert_eq!(before["collision"]["issue_count"], 1);
    assert_eq!(before["gravity_support"]["unsupported_count"], 1);
    assert_eq!(after["collision"]["issue_count"], 0);
    assert_eq!(after["gravity_support"]["unsupported_count"], 0);
    assert_eq!(after["state"], "passed");
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);

    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    shell.settle();
    assert_eq!(shell.app().canonical_digest(), digest);
    let validation = &shell.app().assistant_context()["validation"];
    assert_eq!(validation["collision"]["issue_count"], 1);
    assert_eq!(validation["gravity_support"]["unsupported_count"], 1);
}

#[test]
fn assistant_advanced_tool_applies_one_verified_undoable_batch_without_confirmation() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let original_height = shell.app().document_height_mm();
    let advanced = shell.catalog().text("assistant-advanced-tools");
    let apply = shell.catalog().text("assistant-preview");

    shell.click_row(&advanced);
    shell.click_row(&apply);

    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().document_height_mm(), 35.0);
    assert!(shell.app().assistant_proposal().is_none());
    let verification = shell
        .app()
        .assistant_verification()
        .expect("immediate application returns a verification receipt")
        .clone();
    assert_eq!(verification.revision_id, revision + 1);
    assert_eq!(verification.verified_write_count, 1);
    assert!(shell.app().assistant_change_can_undo());
    shell.settle();
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-result-title")));
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assistant-verification",
        &BTreeMap::from([
            ("revision", verification.revision_id.to_string()),
            ("writes", verification.verified_write_count.to_string()),
        ]),
    )));
    let undo_change = shell.catalog().text("assistant-undo-change");
    shell.click_row(&undo_change);
    assert_eq!(shell.app().document_height_mm(), original_height);
    assert!(!shell.app().assistant_change_can_undo());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().document_height_mm(), 35.0);
}

#[test]
fn assistant_occurrence_visibility_applies_immediately_and_is_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    assert!(shell.app().occurrence_box_geometry(1).is_some());

    assert!(
        shell
            .app_mut()
            .apply_assistant_intent(WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(1),
                visible: false,
            })
    );
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert!(shell.app().occurrence_box_geometry(1).is_none());
    assert!(shell.app_mut().undo());
    assert!(shell.app().occurrence_box_geometry(1).is_some());
}

#[test]
fn furniture_tag_visibility_uses_accessible_review_without_changing_geometry_ownership() {
    let mut shell = Shell::new();
    let tag = TagId(700);
    let confirm = shell.catalog().text("assistant-confirm");

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::CreateTag {
                target: tag,
                name: "Furniture".to_owned(),
                visible: true,
            })
    );
    shell.settle();
    shell.click_row(&confirm);
    assert_eq!(
        shell.app().document_snapshot().tag(tag).unwrap().name(),
        "Furniture"
    );

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                target: OccurrenceId(1),
                tag: Some(tag),
            })
    );
    shell.settle();
    shell.click_row(&confirm);

    let before = shell.app().document_snapshot();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let before_definition = before.occurrence(OccurrenceId(1)).unwrap().definition_id();
    let before_entity_counts = (
        before.definitions().count(),
        before.occurrences().count(),
        before.features().count(),
        before.tags().count(),
    );
    assert!(shell.app().occurrence_box_geometry(1).is_some());

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetTagVisibility {
                target: tag,
                visible: false,
            })
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.settle();
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-title")));
    shell.click_row(&confirm);

    let hidden_digest = shell.app().canonical_digest();
    let hidden = shell.app().document_snapshot();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert!(!hidden.tag(tag).unwrap().visible());
    assert_eq!(
        hidden.occurrence(OccurrenceId(1)).unwrap().definition_id(),
        before_definition
    );
    assert_eq!(
        (
            hidden.definitions().count(),
            hidden.occurrences().count(),
            hidden.features().count(),
            hidden.tags().count(),
        ),
        before_entity_counts
    );
    assert!(shell.app().occurrence_box_geometry(1).is_none());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().occurrence_box_geometry(1).is_some());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), hidden_digest);
    assert!(shell.app().occurrence_box_geometry(1).is_none());
}

#[test]
fn public_apply_helpers_cannot_bypass_review_for_non_whitelisted_changes() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    assert!(
        !shell
            .app_mut()
            .apply_assistant_intent(WorkflowIntent::RenameDefinition {
                target: DefinitionId(1),
                name: "Needs review".to_owned(),
            },)
    );
    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert!(shell.app_mut().cancel_assistant_proposal());

    assert!(
        !shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "Replacement".to_owned(),
                    size_mm: [10.0, 10.0, 10.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                }],
                translations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            })
    );
    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn assistant_dimension_review_preserves_source_and_normalized_value_in_both_locales() {
    for catalog in [LocaleCatalog::english(), LocaleCatalog::slovak()] {
        let mut shell = Shell::with_catalog(catalog);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                    target: FeatureId(2),
                    value_text: "20.0".to_owned(),
                })
        );
        shell.settle();
        let before = shell.catalog().format(
            "assistant-value-dimension",
            &BTreeMap::from([("source", "20".to_owned()), ("value", "20".to_owned())]),
        );
        let after = shell.catalog().format(
            "assistant-value-dimension",
            &BTreeMap::from([("source", "20.0".to_owned()), ("value", "20".to_owned())]),
        );
        let target = shell.catalog().format(
            "assistant-target-identified",
            &BTreeMap::from([
                ("kind", shell.catalog().text("assistant-entity-feature")),
                ("id", "2".to_owned()),
            ]),
        );
        let diff = shell.catalog().format(
            "assistant-diff-changed",
            &BTreeMap::from([("target", target), ("before", before), ("after", after)]),
        );
        assert!(shell.has_visible_label(&diff));
        let cancel = shell.catalog().text("assistant-cancel");
        shell.click_row(&cancel);

        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateEvaluatorInput {
                    target: NodeId(99),
                    name: "Reviewed depth".to_owned(),
                    value_text: "42.50".to_owned(),
                })
        );
        shell.settle();
        let after = shell.catalog().format(
            "assistant-value-evaluator-input-state",
            &BTreeMap::from([
                ("name", "Reviewed depth".to_owned()),
                ("source", "42.50".to_owned()),
                ("value", "42.5".to_owned()),
                ("dependencies", String::new()),
            ]),
        );
        let target = shell.catalog().format(
            "assistant-target-identified",
            &BTreeMap::from([
                ("kind", shell.catalog().text("assistant-entity-evaluator")),
                ("id", "99".to_owned()),
            ]),
        );
        let diff = shell.catalog().format(
            "assistant-diff-created",
            &BTreeMap::from([
                ("target", target),
                ("before", shell.catalog().text("assistant-value-missing")),
                ("after", after),
            ]),
        );
        assert!(shell.has_visible_label(&diff));
        shell.click_row(&cancel);
    }
}

#[test]
fn assistant_occurrence_translation_review_is_exact_observational_and_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let geometry_before = shell.app().occurrence_box_geometry(1).unwrap();

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTranslation {
                target: OccurrenceId(1),
                x_mm_text: "12.5".to_owned(),
                y_mm_text: "-4".to_owned(),
                z_mm_text: "8.25".to_owned(),
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceTranslation(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(Transform::identity())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(Transform::from_translation(12.5, -4.0, 8.25).unwrap())
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap(),
        geometry_before
    );

    shell.settle();
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-title")));
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-observational")));
    let target = shell.catalog().format(
        "assistant-target-identified",
        &BTreeMap::from([
            ("kind", shell.catalog().text("assistant-entity-occurrence")),
            ("id", "1".to_owned()),
        ]),
    );
    let before = shell.catalog().format(
        "assistant-value-transform",
        &BTreeMap::from([(
            "matrix",
            "1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1".to_owned(),
        )]),
    );
    let after = shell.catalog().format(
        "assistant-value-transform",
        &BTreeMap::from([(
            "matrix",
            "1, 0, 0, 12.5, 0, 1, 0, -4, 0, 0, 1, 8.25, 0, 0, 0, 1".to_owned(),
        )]),
    );
    let diff = shell.catalog().format(
        "assistant-diff-changed",
        &BTreeMap::from([("target", target), ("before", before), ("after", after)]),
    );
    assert!(shell.has_visible_label(&diff));
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        Vec3::new(12.5, -4.0, 8.25)
    );
    assert!(shell.app_mut().undo());
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap(),
        geometry_before
    );
}

#[test]
fn canonical_t18_move_tag_and_rename_is_one_accessible_atomic_undo_step() {
    let mut shell = Shell::new();
    let target = OccurrenceId(1);
    let tag = TagId(718);
    let confirm = shell.catalog().text("assistant-confirm");

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::CreateTag {
                target: tag,
                name: "T18 Furniture".to_owned(),
                visible: true,
            })
    );
    shell.settle();
    shell.click_row(&confirm);

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo_steps = shell.app().undo_step_count();
    let (baseline_name, baseline_transform, baseline_tag) = {
        let snapshot = shell.app().document_snapshot();
        let occurrence = snapshot.occurrence(target).unwrap();
        (
            occurrence.name().to_owned(),
            occurrence.transform(),
            occurrence.tag(),
        )
    };
    let expected_transform = Transform::from_translation(25.0, -10.0, 5.0).unwrap();

    assert!(
        !shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::AtomicMultiCommandEdit {
                target,
                x_mm_text: "100".to_owned(),
                y_mm_text: "200".to_owned(),
                z_mm_text: "300".to_owned(),
                tag: TagId(999_718),
                name: "Must not leak".to_owned(),
            })
    );
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
    let rejected = shell.app().document_snapshot();
    let occurrence = rejected.occurrence(target).unwrap();
    assert_eq!(occurrence.name(), baseline_name);
    assert_eq!(occurrence.transform(), baseline_transform);
    assert_eq!(occurrence.tag(), baseline_tag);

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::AtomicMultiCommandEdit {
                target,
                x_mm_text: "25".to_owned(),
                y_mm_text: "-10".to_owned(),
                z_mm_text: "5".to_owned(),
                tag,
                name: "Moved tagged box".to_owned(),
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::AtomicMultiCommandEdit(target)
    );
    assert_eq!(proposal.batch().commands().len(), 3);
    assert!(matches!(
        &proposal.batch().commands()[0],
        CanonicalCommand::SetOccurrenceTransform { id, transform }
            if *id == target && *transform == expected_transform
    ));
    assert!(matches!(
        &proposal.batch().commands()[1],
        CanonicalCommand::SetOccurrenceTag { id, tag: Some(actual) }
            if *id == target && *actual == tag
    ));
    assert!(matches!(
        &proposal.batch().commands()[2],
        CanonicalCommand::RenameEntity { id, name }
            if *id == target && name == "Moved tagged box"
    ));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    shell.settle();
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-title")));
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().document_snapshot();
    let occurrence = committed.occurrence(target).unwrap();
    assert_eq!(occurrence.name(), "Moved tagged box");
    assert_eq!(occurrence.transform(), expected_transform);
    assert_eq!(occurrence.tag(), Some(tag));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
    let undone = shell.app().document_snapshot();
    let occurrence = undone.occurrence(target).unwrap();
    assert_eq!(occurrence.name(), baseline_name);
    assert_eq!(occurrence.transform(), baseline_transform);
    assert_eq!(occurrence.tag(), baseline_tag);

    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
    let redone = shell.app().document_snapshot();
    let occurrence = redone.occurrence(target).unwrap();
    assert_eq!(occurrence.name(), "Moved tagged box");
    assert_eq!(occurrence.transform(), expected_transform);
    assert_eq!(occurrence.tag(), Some(tag));
}

#[test]
fn assistant_definition_rename_review_is_textual_observational_and_undoable() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: ketchup_core::document::DefinitionId(1),
                name: "Housing".to_owned(),
            })
    );
    let proposal = shell.app().assistant_proposal().unwrap();
    let original_name = proposal.authoritative_diff()[0].before.clone();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RenameDefinition(ketchup_core::document::DefinitionId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Housing".to_owned())
    );
    assert_eq!(shell.app().document_revision(), revision);

    shell.settle();
    let cancel = shell.catalog().text("assistant-cancel");
    shell.click_row(&cancel);
    assert_eq!(shell.app().document_revision(), revision);
    assert!(shell.app().assistant_proposal().is_none());
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: ketchup_core::document::DefinitionId(1),
                name: "Housing".to_owned(),
            })
    );
    assert!(shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert!(shell.app_mut().undo());
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: ketchup_core::document::DefinitionId(1),
                name: "Housing".to_owned(),
            })
    );
    assert_eq!(
        shell
            .app()
            .assistant_proposal()
            .unwrap()
            .authoritative_diff()[0]
            .before,
        original_name
    );
}

#[test]
fn assistant_confirmation_fails_closed_after_an_unrelated_canonical_revision() {
    let mut shell = Shell::new();
    let base_revision = shell.app().document_revision();
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: DefinitionId(1),
                name: "Stale rename".to_owned(),
            },)
    );
    assert!(shell.app().assistant_proposal().is_some());

    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-view", AppCommand::Hide);
    let intervening_revision = shell.app().document_revision();
    let intervening_digest = shell.app().canonical_digest();
    assert_eq!(intervening_revision, base_revision + 1);
    assert!(shell.app().assistant_proposal().is_some());

    shell.settle();
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), intervening_revision);
    assert_eq!(shell.app().canonical_digest(), intervening_digest);
}

#[test]
fn new_open_and_new_chat_cancel_reviewed_work_through_accessible_shell_commands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-cancel.ketchup");
    let mut source = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&path));
    source.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    let opened_digest = source.app().canonical_digest();
    drop(source);

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let prepare_review = |shell: &mut Shell| {
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Reviewed name".to_owned(),
                },)
        );
        assert!(shell.app().assistant_proposal().is_some());
    };

    prepare_review(&mut shell);
    let new_chat = shell.catalog().text("assistant-new-chat");
    shell.click_row(&new_chat);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);

    prepare_review(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().assistant_proposal().is_none());

    prepare_review(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().canonical_digest(), opened_digest);
}

#[test]
fn assistant_creates_a_rectangular_prism_as_one_reviewed_batch_and_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_definitions = shell.app().definition_count();
    let initial_occurrences = shell.app().active_box_count();

    assert!(
        shell
            .app_mut()
            .prepare_assistant_model_intent(AssistantModelIntent {
                replace_scene: false,
                boxes: vec![AssistantBoxIntent {
                    name: "AI rectangular prism".to_owned(),
                    size_mm: [120.0, 80.0, 10.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                }],
                translations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            })
    );
    assert_eq!(shell.app().document_revision(), initial_revision);
    shell.settle();
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().definition_count(), initial_definitions + 1);
    assert_eq!(shell.app().active_box_count(), initial_occurrences + 1);
    let completed_digest = shell.app().canonical_digest();
    assert_ne!(completed_digest, initial_digest);

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().definition_count(), initial_definitions);
    assert_eq!(shell.app().active_box_count(), initial_occurrences);

    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), completed_digest);
    assert_eq!(shell.app().definition_count(), initial_definitions + 1);
    assert_eq!(shell.app().active_box_count(), initial_occurrences + 1);
}

#[test]
fn assistant_replacement_review_hides_internal_digests_and_describes_removals() {
    let mut shell = Shell::new();
    assert!(
        shell
            .app_mut()
            .prepare_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "Replacement".to_owned(),
                    size_mm: [10.0, 20.0, 30.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                }],
                translations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
            })
    );
    let removed = shell
        .app()
        .assistant_proposal()
        .unwrap()
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target
                == ketchup_core::document::AuthoritativeDependency::Definition(DefinitionId(1))
        })
        .unwrap();
    let ProposalValue::Digest(digest) = &removed.before else {
        panic!("replacement review must bind the removed definition digest");
    };
    assert_eq!(removed.after, ProposalValue::Missing);
    let target = shell.catalog().format(
        "assistant-target-identified",
        &BTreeMap::from([
            ("kind", shell.catalog().text("assistant-entity-definition")),
            ("id", "1".to_owned()),
        ]),
    );
    let friendly = shell.catalog().format(
        "assistant-diff-removed",
        &BTreeMap::from([
            ("target", target),
            ("before", String::new()),
            ("after", String::new()),
        ]),
    );
    let internal = shell.catalog().format(
        "assistant-diff-row",
        &BTreeMap::from([
            (
                "before",
                shell.catalog().format(
                    "assistant-value-digest",
                    &BTreeMap::from([("digest", digest.clone())]),
                ),
            ),
            ("after", shell.catalog().text("assistant-value-missing")),
        ]),
    );

    shell.settle();
    assert!(shell.has_visible_label(&friendly));
    assert!(!shell.has_visible_label(&internal));
}

#[test]
fn assistant_model_intent_applies_real_3d_boxes_immediately_as_one_undoable_batch() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Column".to_owned(),
                    size_mm: [400.0, 400.0, 2_500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Beam".to_owned(),
                    size_mm: [5_000.0, 300.0, 500.0],
                    origin_mm: [0.0, 50.0, 2_500.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_ne!(shell.app().canonical_digest(), initial_digest);
    assert!(shell.app().assistant_proposal().is_none());
    assert!(shell.app().assistant_verification().is_some());
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(400.0, 400.0, 2_500.0)))
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(3),
        Some((
            Vec3::new(0.0, 50.0, 2_500.0),
            Vec3::new(5_000.0, 300.0, 500.0)
        ))
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_teapot_intent_creates_smooth_hollow_saved_model_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_features = shell.app().feature_count();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: vec![AssistantBottleIntent {
                name: "Rounded tea pot".to_owned(),
                body_radius_mm: 70.0,
                body_height_mm: 105.0,
                shoulder_rise_mm: 22.0,
                neck_radius_mm: 42.0,
                neck_height_mm: 14.0,
                wall_thickness_mm: 3.0,
                finish_kind: AssistantBottleFinishKind::Fillet,
                finish_amount_mm: 4.0,
                origin_mm: [0.0, 0.0, 0.0],
                teapot: Some(AssistantTeapotIntent {
                    handle_clearance_mm: 52.0,
                    handle_tube_radius_mm: 9.0,
                    spout_length_mm: 105.0,
                    spout_radius_mm: 14.0,
                    lid_height_mm: 18.0,
                    lid_knob_radius_mm: 10.0,
                }),
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert!(shell.app().feature_count() > initial_features);
    assert!(shell.app().occurrence_count() >= 1);
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot
        .features()
        .find(|feature| feature.name() == "Rounded tea pot smooth hollow vessel")
        .expect("teapot mesh must exist");
    let FeatureKind::MeshBody(mesh) = feature.kind() else {
        panic!("teapot must be one canonical mesh body");
    };
    assert!(mesh.vertices_mm.len() > 1_200);
    assert!(mesh.triangles.len() > 2_000);
    assert!(mesh.vertices_mm.contains(&[42.0, 0.0, 141.0]));
    assert!(mesh.vertices_mm.contains(&[39.0, 0.0, 141.0]));
    assert!(mesh.vertices_mm.iter().any(|vertex| vertex[0] > 170.0));
    assert!(mesh.vertices_mm.iter().any(|vertex| vertex[0] < -110.0));
    assert!(matches!(
        &mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-rounded-teapot-v1"
    ));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-rounded-teapot.ketchup");
    persistence::save_atomic(&path, &snapshot).unwrap();
    let reopened = persistence::load_file(&path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    assert!(
        reopened
            .features()
            .any(|feature| feature.name() == "Rounded tea pot smooth hollow vessel")
    );
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assistant-rounded-teapot.ketchup");
    let fixture = persistence::load_file(&fixture_path).unwrap().snapshot();
    let fixture_feature = fixture
        .features()
        .find(|feature| feature.name() == "Rounded tea pot smooth hollow vessel")
        .expect("saved teapot fixture must remain openable");
    let FeatureKind::MeshBody(fixture_mesh) = fixture_feature.kind() else {
        panic!("saved teapot fixture must retain its canonical mesh body");
    };
    assert_eq!(fixture_mesh.vertices_mm.len(), mesh.vertices_mm.len());
    assert_eq!(fixture_mesh.triangles.len(), mesh.triangles.len());
    assert_eq!(fixture_mesh.authority, mesh.authority);

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_bottle_intent_creates_editable_feature_chain_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_features = shell.app().feature_count();
    let initial_occurrences = shell.app().occurrence_count();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: vec![AssistantBottleIntent {
                name: "AI ketchup bottle".to_owned(),
                body_radius_mm: 30.0,
                body_height_mm: 110.0,
                shoulder_rise_mm: 20.0,
                neck_radius_mm: 12.0,
                neck_height_mm: 25.0,
                wall_thickness_mm: 2.0,
                finish_kind: AssistantBottleFinishKind::Chamfer,
                finish_amount_mm: 2.0,
                origin_mm: [90.0, 0.0, 0.0],
                teapot: None,
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().feature_count(), initial_features + 5);
    assert_eq!(shell.app().occurrence_count(), initial_occurrences + 1);
    let snapshot = shell.app().document_snapshot();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "AI ketchup bottle occurrence")
        .unwrap();
    let transform = occurrence.transform();
    let matrix = transform.matrix();
    assert_eq!([matrix[3], matrix[7], matrix[11]], [90.0, 0.0, 0.0]);
    let kinds = snapshot
        .features()
        .filter(|feature| feature.definition_id() == occurrence.definition_id())
        .map(|feature| feature.kind())
        .collect::<Vec<_>>();
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::Profile { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::BottleProfileControl { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::Revolve { .. }))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::Shell { .. }))
    );
    assert!(kinds.iter().any(|kind| matches!(
        kind,
        FeatureKind::BottleEdgeFinish {
            kind: ketchup_core::document::BottleEdgeFinishKind::Chamfer,
            ..
        }
    )));

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_builds_gable_roof_floor_opening_and_staircase_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "Attic floor with stair opening".to_owned(),
                size_mm: [5_500.0, 3_800.0, 200.0],
                origin_mm: [0.0, 0.0, 3_200.0],
                subtract_boxes: vec![AssistantSubtractionIntent {
                    size_mm: [900.0, 1_400.0, 200.0],
                    origin_mm: [3_900.0, 1_900.0, 0.0],
                }],
            }],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: vec![AssistantGableRoofIntent {
                name: "True gable roof".to_owned(),
                length_mm: 5_900.0,
                span_mm: 4_200.0,
                rise_mm: 1_400.0,
                thickness_mm: 180.0,
                origin_mm: [-200.0, -200.0, 3_400.0],
            }],
            staircases: vec![AssistantStaircaseIntent {
                name: "Attic staircase".to_owned(),
                run_mm: 3_000.0,
                width_mm: 800.0,
                rise_mm: 3_000.0,
                step_count: 15,
                origin_mm: [1_800.0, 2_200.0, 200.0],
            }],
            oriented_beams: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().occurrence_count(), 3);
    assert_eq!(shell.app().active_box_count(), 3);
    let snapshot = shell.app().document_snapshot();
    let roof = snapshot
        .features()
        .find(|feature| feature.name() == "True gable roof solid")
        .expect("gable roof mesh must exist");
    let FeatureKind::MeshBody(roof) = roof.kind() else {
        panic!("gable roof must be one canonical mesh body");
    };
    assert_eq!(roof.vertices_mm.len(), 12);
    assert_eq!(roof.triangles.len(), 20);
    let floor = snapshot
        .features()
        .find(|feature| feature.name() == "Attic floor with stair opening solid")
        .expect("floor opening mesh must exist");
    let FeatureKind::MeshBody(floor) = floor.kind() else {
        panic!("floor opening must be one canonical mesh body");
    };
    assert!(floor.vertices_mm.contains(&[3_900.0, 1_900.0, 0.0]));
    assert!(floor.vertices_mm.contains(&[4_800.0, 3_300.0, 200.0]));
    let stairs = snapshot
        .features()
        .find(|feature| feature.name() == "Attic staircase solid")
        .expect("staircase mesh must exist");
    let FeatureKind::MeshBody(stairs) = stairs.kind() else {
        panic!("staircase must be one canonical mesh body");
    };
    assert!(stairs.vertices_mm.contains(&[200.0, 0.0, 200.0]));
    assert!(stairs.vertices_mm.contains(&[3_000.0, 800.0, 3_000.0]));

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_builds_sloped_rafters_with_real_notches_and_central_purlin() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let rafter = |name: &str, x: f64, start_y: f64, end_y: f64| AssistantOrientedBeamIntent {
        name: name.to_owned(),
        start_mm: [x, start_y, 3_044.067_796_610_17],
        end_mm: [x, end_y, 4_800.0],
        up_hint: [0.0, 0.0, 1.0],
        width_mm: 100.0,
        depth_mm: 180.0,
        bottom_notches: vec![AssistantBeamNotchIntent {
            from_start_mm: 600.0,
            length_mm: 160.0,
            depth_mm: 50.0,
        }],
    };

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: vec![
                rafter("Left rafter 1", 0.0, -483.050_847_457_627_1, 1_900.0),
                rafter("Right rafter 1", 0.0, 4_283.050_847_457_627, 1_900.0),
                rafter("Left rafter 2", 600.0, -483.050_847_457_627_1, 1_900.0),
                rafter("Right rafter 2", 600.0, 4_283.050_847_457_627, 1_900.0),
                AssistantOrientedBeamIntent {
                    name: "Central purlin".to_owned(),
                    start_mm: [-200.0, 1_900.0, 4_600.0],
                    end_mm: [5_700.0, 1_900.0, 4_600.0],
                    up_hint: [0.0, 0.0, 1.0],
                    width_mm: 160.0,
                    depth_mm: 240.0,
                    bottom_notches: Vec::new(),
                },
            ],
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().occurrence_count(), 5);
    let snapshot = shell.app().document_snapshot();
    let rafter_feature = snapshot
        .features()
        .find(|feature| feature.name() == "Left rafter 1 solid")
        .expect("rafter mesh must exist");
    let FeatureKind::MeshBody(rafter_mesh) = rafter_feature.kind() else {
        panic!("rafter must be one canonical mesh body");
    };
    assert!(rafter_mesh.vertices_mm.contains(&[600.0, -50.0, -90.0]));
    assert!(rafter_mesh.vertices_mm.contains(&[600.0, -50.0, -40.0]));
    assert!(rafter_mesh.vertices_mm.contains(&[760.0, 50.0, -40.0]));
    let left = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Left rafter 1")
        .expect("left rafter occurrence must exist");
    let right = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Right rafter 1")
        .expect("right rafter occurrence must exist");
    assert!(left.transform().matrix()[8] > 0.5);
    assert!(right.transform().matrix()[8] > 0.5);
    assert!(left.transform().matrix()[4] > 0.5);
    assert!(right.transform().matrix()[4] < -0.5);

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_subtractions_create_one_real_grooved_body_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let groove_starts = (0..17).map(|index| 300.0 + index as f64 * 583.75);
    let subtract_boxes = groove_starts
        .map(|x| AssistantSubtractionIntent {
            size_mm: [60.0, 160.0, 20.0],
            origin_mm: [x, 0.0, 140.0],
        })
        .collect();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "10 m grooved beam".to_owned(),
                size_mm: [10_000.0, 160.0, 160.0],
                origin_mm: [0.0, 0.0, 0.0],
                subtract_boxes,
            }],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let snapshot = shell.app().document_snapshot();
    assert_eq!(snapshot.occurrences().count(), 1);
    assert_eq!(snapshot.definitions().count(), 1);
    let mesh = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            ketchup_core::document::FeatureKind::MeshBody(mesh) => Some(mesh),
            _ => None,
        })
        .expect("the grooved beam must be one canonical mesh body");
    assert!(
        mesh.vertices_mm
            .iter()
            .any(|point| point == &[300.0, 0.0, 140.0])
    );
    assert!(!mesh.triangles.is_empty());

    let open_tab = shell.catalog().text("assistant-open-tab");
    shell.click_row(&open_tab);
    shell.settle();
    shell.app_mut().zoom_fit();
    shell.settle();
    let rect = shell.viewport_rect();
    let groove_corner = Vec3::new(300.0, 0.0, 140.0);
    let pointer = shell.app().project_to_screen(groove_corner, rect);
    shell.app_mut().zoom_at_screen(pointer, rect, 320.0);
    let after_zoom = shell.app().project_to_screen(groove_corner, rect);
    assert!(
        after_zoom.distance(pointer) < 0.01,
        "wheel zoom must keep the groove corner under the pointer"
    );
    let measured = shell
        .app()
        .measurement_point_at_screen(pointer + eframe::egui::Vec2::new(8.0, 0.0), rect, 140.0)
        .expect("Measure must resolve a nearby grooved-body vertex");
    assert!(measured.distance(groove_corner) < 1.0e-9);

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_moves_existing_grooved_body_without_rebuilding_its_geometry() {
    let mut shell = Shell::new();
    let subtract_boxes = (0..17)
        .map(|index| AssistantSubtractionIntent {
            size_mm: [60.0, 160.0, 20.0],
            origin_mm: [300.0 + index as f64 * 583.75, 0.0, 140.0],
        })
        .collect();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "10 m grooved beam".to_owned(),
                size_mm: [10_000.0, 160.0, 160.0],
                origin_mm: [0.0, 0.0, 0.0],
                subtract_boxes,
            }],
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));
    let occurrence_id = shell
        .app()
        .document_snapshot()
        .occurrences()
        .next()
        .unwrap()
        .id();
    let before = shell.app().document_snapshot();
    let feature_before = before.features().next().unwrap().clone();
    let revision = before.revision_id();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            translations: vec![AssistantTranslationIntent {
                occurrence_id: occurrence_id.0,
                delta_mm: [100.0, 0.0, 0.0],
            }],
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(occurrence_id)
            .unwrap()
            .transform()
            .matrix()[3],
        100.0
    );
    assert_eq!(
        shell.app().document_snapshot().features().next().unwrap(),
        &feature_before
    );
    assert!(shell.app_mut().undo());
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(occurrence_id)
            .unwrap()
            .transform()
            .matrix()[3],
        0.0
    );
}

#[test]
fn assistant_context_keeps_all_17_plain_and_7_grooved_parts_copyable_with_bounds() {
    let mut shell = Shell::new();
    let mut boxes = (0..17)
        .map(|index| AssistantBoxIntent {
            name: format!("Plain beam {}", index + 1),
            size_mm: [10_000.0, 60.0, 160.0],
            origin_mm: [0.0, f64::from(index) * 200.0, 0.0],
            subtract_boxes: Vec::new(),
        })
        .collect::<Vec<_>>();
    boxes.extend((0..7).map(|index| AssistantBoxIntent {
        name: format!("Grooved beam {}", index + 1),
        size_mm: [10_000.0, 160.0, 160.0],
        origin_mm: [0.0, f64::from(index) * 500.0, 120.0],
        subtract_boxes: vec![AssistantSubtractionIntent {
            size_mm: [60.0, 160.0, 20.0],
            origin_mm: [300.0, 0.0, 140.0],
        }],
    }));
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes,
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    let context = shell.app().assistant_context();
    let occurrences = context["occurrences"].as_array().unwrap();
    assert_eq!(context["occurrence_count"], 24);
    assert_eq!(context["occurrences_complete"], true);
    assert!(
        context["selected_occurrence_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(occurrences.len(), 24);
    assert_eq!(context["boxes"].as_array().unwrap().len(), 17);
    assert!(occurrences.iter().all(|item| item["copyable"] == true));
    assert!(occurrences.iter().all(|item| item["bounds_mm"].is_object()));
    assert_eq!(
        occurrences
            .iter()
            .filter(|item| item["name"].as_str().unwrap().starts_with("Grooved beam"))
            .count(),
        7
    );
}

#[test]
fn assistant_stacks_24_existing_parts_into_20_layers_as_shared_occurrences_in_one_undo_step() {
    let mut shell = Shell::new();
    let boxes = (0..24)
        .map(|index| AssistantBoxIntent {
            name: format!("Part {}", index + 1),
            size_mm: [100.0, 100.0, 280.0],
            origin_mm: [f64::from(index) * 120.0, 0.0, 0.0],
            subtract_boxes: Vec::new(),
        })
        .collect();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes,
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));
    let before = shell.app().document_snapshot();
    let revision = before.revision_id();
    let source_ids = before
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    let definition_ids = before
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: vec![AssistantLinearArrayIntent {
                occurrence_ids: source_ids,
                instances: 20,
                step_mm: [0.0, 0.0, 280.0],
            }],
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        }
    ));

    let stacked = shell.app().document_snapshot();
    assert_eq!(stacked.revision_id(), revision + 1);
    assert_eq!(stacked.occurrences().count(), 24 * 20);
    assert_eq!(stacked.definitions().count(), 24);
    let context = shell.app().assistant_context();
    assert_eq!(context["occurrence_count"], 480);
    assert_eq!(context["occurrences_complete"], false);
    assert_eq!(context["occurrences"].as_array().unwrap().len(), 100);

    let worker_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated_worker = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(worker_name);
    let worker = if colocated_worker.is_file() {
        colocated_worker
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(worker_name)
    };
    shell.app_mut().connect_exact_worker(&worker).unwrap();

    let first_frame = Instant::now();
    shell.settle();
    assert!(
        first_frame.elapsed() < Duration::from_secs(2),
        "the first rendered frame of a 480-occurrence scene took {:?}",
        first_frame.elapsed()
    );
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 24 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shell.app().exact_render_body_count(), 24);
    shell.app_mut().enable_headless_instanced_scene();
    let pointer = shell.viewport_rect().center();
    shell.move_pointer(pointer);
    let repeated_frames = Instant::now();
    for offset in 0..10 {
        shell.move_pointer(pointer + egui::Vec2::new(offset as f32, 0.0));
    }
    assert!(
        repeated_frames.elapsed() < Duration::from_secs(2),
        "ten cached hovered frames of a 480-occurrence exact scene took {:?}",
        repeated_frames.elapsed()
    );

    let orbit_frames = Instant::now();
    shell.orbit_drag(pointer, egui::Vec2::new(3.0, -2.0), 20);
    assert!(
        orbit_frames.elapsed() < Duration::from_secs(1),
        "twenty orbit frames of a 480-occurrence exact scene took {:?}",
        orbit_frames.elapsed()
    );

    assert_eq!(
        stacked
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>(),
        definition_ids
    );
    let mut layer_counts = BTreeMap::new();
    for occurrence in stacked.occurrences() {
        *layer_counts
            .entry(occurrence.transform().matrix()[11] as i64)
            .or_insert(0usize) += 1;
    }
    assert_eq!(layer_counts.len(), 20);
    assert!((0..20).all(|layer| layer_counts.get(&(layer * 280)).copied() == Some(24)));

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 24);
    assert_eq!(shell.app().document_snapshot().definitions().count(), 24);
}

#[test]
fn assistant_workspace_exposes_provider_filtered_models_and_new_chat() {
    let mut shell = Shell::new();

    assert_eq!(
        shell.app().assistant_workspace_mode(),
        AssistantWorkspaceMode::Dock
    );
    #[cfg(not(feature = "private-oauth"))]
    assert!(
        shell
            .app()
            .assistant_models()
            .iter()
            .all(|model| model.starts_with("claude-"))
    );
    #[cfg(feature = "private-oauth")]
    assert!(
        shell
            .app()
            .assistant_models()
            .iter()
            .all(|model| model.starts_with("gpt-"))
    );
    shell
        .app_mut()
        .select_assistant_provider(AssistantProvider::OpenAiApi);
    let models = shell.app().assistant_models();
    assert!(models.contains(&"gpt-5.2".to_owned()));
    assert!(models.iter().all(|model| model.starts_with("gpt-")));
    assert!(models.iter().all(|model| !model.contains('[')));

    let open_tab = shell.catalog().text("assistant-open-tab");
    shell.click_row(&open_tab);
    assert_eq!(
        shell.app().assistant_workspace_mode(),
        AssistantWorkspaceMode::Tab
    );
    let dock_right = shell.catalog().text("assistant-dock-right");
    shell.click_row(&dock_right);
    assert_eq!(
        shell.app().assistant_workspace_mode(),
        AssistantWorkspaceMode::Dock
    );
    shell.app_mut().new_assistant_chat();
    assert!(shell.app().assistant_messages().is_empty());
}

#[test]
fn assistant_provider_selection_builds_an_exact_model_bound_handshake() {
    let mut shell = Shell::new();

    #[cfg(not(feature = "private-oauth"))]
    {
        assert_eq!(
            shell.app().assistant_provider(),
            AssistantProvider::AnthropicApi
        );
        assert_eq!(shell.app().assistant_model(), "claude-sonnet-5");
        let anthropic = shell.app().assistant_handshake();
        assert_eq!(anthropic.protocol_version, ASSISTANT_PROTOCOL_VERSION);
        assert_eq!(anthropic.distribution, AssistantDistribution::PublicApi);
        assert_eq!(anthropic.provider, "anthropic-api");
        assert_eq!(anthropic.model, "claude-sonnet-5");
        anthropic.validate().unwrap();
    }
    #[cfg(feature = "private-oauth")]
    {
        assert_eq!(
            shell.app().assistant_provider(),
            AssistantProvider::CodexOauth
        );
        assert_eq!(shell.app().assistant_model(), "gpt-5.6-sol");
        let codex = shell.app().assistant_handshake();
        assert_eq!(codex.protocol_version, ASSISTANT_PROTOCOL_VERSION);
        assert_eq!(codex.distribution, AssistantDistribution::PrivateOauth);
        assert_eq!(codex.provider, "codex-oauth");
        assert_eq!(codex.model, "gpt-5.6-sol");
        codex.validate().unwrap();
    }

    shell
        .app_mut()
        .select_assistant_provider(AssistantProvider::OpenAiApi);
    assert_eq!(shell.app().assistant_model(), "gpt-5.2");
    let openai = shell.app().assistant_handshake();
    assert_eq!(openai.distribution, AssistantDistribution::PublicApi);
    assert_eq!(openai.provider, "openai-api");
    assert_eq!(openai.model, "gpt-5.2");
    openai.validate().unwrap();

    shell.app_mut().set_assistant_model("gpt-5.2-mini");
    let custom = shell.app().assistant_handshake();
    assert_eq!(custom.model, "gpt-5.2-mini");
    custom.validate().unwrap();

    shell.app_mut().set_assistant_model("../invalid model");
    assert!(shell.app().assistant_handshake().validate().is_err());
}

#[cfg(feature = "private-oauth")]
#[test]
fn private_build_exposes_only_model_compatible_oauth_selections() {
    let mut shell = Shell::new();
    assert_eq!(shell.app().assistant_model(), "gpt-5.6-sol");
    let initial = shell.app().assistant_handshake();
    assert_eq!(initial.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(initial.provider, "codex-oauth");
    assert_eq!(initial.model, "gpt-5.6-sol");
    initial.validate().unwrap();

    shell
        .app_mut()
        .select_assistant_provider(AssistantProvider::ClaudeCodeOauth);
    let claude = shell.app().assistant_handshake();
    assert_eq!(claude.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(claude.provider, "claude-code-oauth");
    assert_eq!(claude.model, "claude-sonnet-5");
    claude.validate().unwrap();

    shell
        .app_mut()
        .select_assistant_provider(AssistantProvider::CodexOauth);
    assert_eq!(shell.app().assistant_model(), "gpt-5.6-sol");
    let codex = shell.app().assistant_handshake();
    assert_eq!(codex.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(codex.provider, "codex-oauth");
    assert_eq!(codex.model, "gpt-5.6-sol");
    codex.validate().unwrap();
}

#[test]
fn assistant_rejects_invalid_and_stale_intents_without_an_extra_mutation() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    assert!(
        !shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(999),
                value_text: "30".to_owned(),
            })
    );
    assert_eq!(shell.app().document_revision(), initial_revision);

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(2),
                value_text: "40".to_owned(),
            })
    );
    let top_face = shell.top_face_centre(1);
    shell.click_at(top_face);
    shell.click_command(AppCommand::PushPull);
    shell.type_text("5");
    shell.press_key(egui::Key::Enter);
    let changed_revision = shell.app().document_revision();
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}
