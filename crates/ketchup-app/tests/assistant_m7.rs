mod harness;

use harness::Shell;
use ketchup_app::{AssistantProvider, AssistantWorkspaceMode};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBoxIntent, AssistantDistribution,
    AssistantLinearArrayIntent, AssistantModelIntent, AssistantSubtractionIntent,
    AssistantTranslationIntent,
};
use ketchup_core::document::{
    DefinitionId, FeatureId, OccurrenceId, ProposalGoal, ProposalValue, Transform,
};
use ketchup_core::intent::WorkflowIntent;
use ketchup_interaction::Vec3;
use std::collections::BTreeMap;

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
        .expect("immediate application returns a verification receipt");
    assert_eq!(verification.revision_id, revision + 1);
    assert_eq!(verification.verified_write_count, 1);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_height_mm(), original_height);
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

    assert!(shell.app_mut().confirm_assistant_proposal());
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
fn assistant_draws_a_new_rectangle_through_immediate_canonical_steps() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_definitions = shell.app().definition_count();
    let initial_occurrences = shell.app().active_box_count();
    let definition = DefinitionId(20);
    let profile = FeatureId(30);
    let occurrence = OccurrenceId(40);

    for intent in [
        WorkflowIntent::CreateDefinition {
            target: definition,
            name: "AI rectangle".to_owned(),
        },
        WorkflowIntent::CreateProfileFeature {
            target: profile,
            definition,
            name: "AI rectangle profile".to_owned(),
            points_mm: vec![[0.0, 0.0], [120.0, 0.0], [120.0, 80.0], [0.0, 80.0]],
        },
        WorkflowIntent::CreateOccurrence {
            target: occurrence,
            definition,
            name: "AI rectangle occurrence".to_owned(),
        },
    ] {
        let revision = shell.app().document_revision();
        assert!(shell.app_mut().apply_assistant_intent(intent));
        assert!(shell.app().assistant_proposal().is_none());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(
            shell.app().assistant_verification().unwrap().revision_id,
            revision + 1
        );
    }

    assert_eq!(shell.app().definition_count(), initial_definitions + 1);
    assert_eq!(shell.app().active_box_count(), initial_occurrences + 1);
    assert_eq!(
        shell.app().occurrence_definition_id(occurrence),
        Some(definition)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(occurrence.0),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(120.0, 80.0, 0.0)))
    );
    let completed_digest = shell.app().canonical_digest();
    assert_ne!(completed_digest, initial_digest);

    for _ in 0..3 {
        assert!(shell.app_mut().undo());
    }
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().definition_count(), initial_definitions);
    assert_eq!(shell.app().active_box_count(), initial_occurrences);

    for _ in 0..3 {
        assert!(shell.app_mut().redo());
    }
    assert_eq!(shell.app().canonical_digest(), completed_digest);
    assert_eq!(
        shell.app().occurrence_definition_id(occurrence),
        Some(definition)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(occurrence.0),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(120.0, 80.0, 0.0)))
    );
}

#[test]
fn assistant_model_intent_applies_real_3d_boxes_immediately_as_one_undoable_batch() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();

    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
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
            })
    );
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

    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "10 m grooved beam".to_owned(),
                    size_mm: [10_000.0, 160.0, 160.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes,
                }],
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            })
    );

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
    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes: vec![AssistantBoxIntent {
                    name: "10 m grooved beam".to_owned(),
                    size_mm: [10_000.0, 160.0, 160.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes,
                }],
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            })
    );
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

    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: vec![AssistantTranslationIntent {
                    occurrence_id: occurrence_id.0,
                    delta_mm: [100.0, 0.0, 0.0],
                }],
                linear_arrays: Vec::new(),
            })
    );

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
    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes,
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            })
    );

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
    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: true,
                boxes,
                translations: Vec::new(),
                linear_arrays: Vec::new(),
            })
    );
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

    assert!(
        shell
            .app_mut()
            .apply_assistant_model_intent(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: Vec::new(),
                linear_arrays: vec![AssistantLinearArrayIntent {
                    occurrence_ids: source_ids,
                    instances: 20,
                    step_mm: [0.0, 0.0, 280.0],
                }],
            })
    );

    let stacked = shell.app().document_snapshot();
    assert_eq!(stacked.revision_id(), revision + 1);
    assert_eq!(stacked.occurrences().count(), 24 * 20);
    assert_eq!(stacked.definitions().count(), 24);
    let context = shell.app().assistant_context();
    assert_eq!(context["occurrence_count"], 480);
    assert_eq!(context["occurrences_complete"], false);
    assert_eq!(context["occurrences"].as_array().unwrap().len(), 100);
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
    shell.app_mut().set_push_pull_distance_input("5");
    assert!(shell.app_mut().start_preview());
    assert!(shell.app_mut().confirm_preview());
    let changed_revision = shell.app().document_revision();
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_assistant_proposal());
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}
