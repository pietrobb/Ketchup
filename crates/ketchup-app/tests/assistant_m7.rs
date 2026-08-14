mod harness;

use eframe::egui;
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AppCommand, AssistantMessageRole, AssistantProvider, AssistantWorkspaceMode};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBoxIntent, AssistantChatResult, AssistantDistribution,
    AssistantLinearArrayIntent, AssistantModelIntent, AssistantSubtractionIntent,
    AssistantTranslationIntent,
};
use ketchup_core::document::{
    DefinitionId, FeatureId, NodeId, OccurrenceId, ProposalGoal, ProposalValue, Transform,
};
use ketchup_core::intent::WorkflowIntent;
use ketchup_interaction::{LocaleCatalog, Vec3};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn apply_reviewed_model_intent(shell: &mut Shell, intent: AssistantModelIntent) -> bool {
    shell.app_mut().prepare_assistant_model_intent(intent)
        && shell.app_mut().confirm_assistant_proposal()
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
            linear_arrays: Vec::new(),
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
                linear_arrays: Vec::new(),
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
        let diff = shell.catalog().format(
            "assistant-diff-row",
            &BTreeMap::from([("before", before), ("after", after)]),
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
        let diff = shell.catalog().format(
            "assistant-diff-row",
            &BTreeMap::from([
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
        "assistant-diff-row",
        &BTreeMap::from([("before", before), ("after", after)]),
    );
    assert!(shell.has_visible_label(&target));
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
                linear_arrays: Vec::new(),
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
            linear_arrays: Vec::new(),
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
            linear_arrays: Vec::new(),
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
            linear_arrays: Vec::new(),
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
            translations: vec![AssistantTranslationIntent {
                occurrence_id: occurrence_id.0,
                delta_mm: [100.0, 0.0, 0.0],
            }],
            linear_arrays: Vec::new(),
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
            linear_arrays: Vec::new(),
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
            linear_arrays: Vec::new(),
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
            linear_arrays: vec![AssistantLinearArrayIntent {
                occurrence_ids: source_ids,
                instances: 20,
                step_mm: [0.0, 0.0, 280.0],
            }],
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
    assert!(
        shell
            .app()
            .assistant_models()
            .iter()
            .all(|model| model.starts_with("claude-"))
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
    assert_eq!(shell.app().assistant_model(), "gpt-5.5");
    let codex = shell.app().assistant_handshake();
    assert_eq!(codex.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(codex.provider, "codex-oauth");
    assert_eq!(codex.model, "gpt-5.5");
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
