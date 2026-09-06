//! End-to-end AI-native acceptance proof: a timber-frame house built from an
//! empty document through the generic Assistant CAD edit program only.
//!
//! No named-shape intent, no fixture, no mesh shortcut. Every step is a natural
//! language request that the host compiles into canonical commands, reviews as
//! a proposal, and commits as exactly one undo step.
//!
//! The second test pins the two generality limits this scenario hit, so that a
//! later change which lifts them fails loudly instead of silently.
mod harness;

use eframe::egui;
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::AssistantMessageRole;
use ketchup_application::validation::{
    AssistantValidationSelection, assistant_validation_context_with_worker,
};
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadClassificationCategory,
    AssistantCadDeletePolicy, AssistantCadEditOperation, AssistantCadEditProgram,
    AssistantCadEntitySelector, AssistantCadFeatureReference, AssistantCadPartFeature,
    AssistantCadProgramFeatureOutput, AssistantCadProgramFeatureReference, AssistantCadRotation,
    AssistantChatResult, AssistantDistribution, AssistantPrincipalPlane, AssistantSketchConstraint,
    AssistantSketchEntity, AssistantWorkplaneSpec,
};
use ketchup_core::document::{
    DefinitionId, EdgeFinishKind, FeatureId, FeatureKind, InstancePath, Snapshot,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V13, ExactBRepGraph, ExactBRepGraphError,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactResultRegistry,
};
use ketchup_core::exact_validation::{
    BuiltinGeneralBodyValidator, BuiltinGravitySupportValidator, GeneralBodyParticipant,
    GeneralClearanceCase, GravitySupportInput, GravitySupportParticipant, general_body_input_bytes,
    general_body_validation_policy, gravity_support_input_bytes, gravity_support_validation_policy,
};
use ketchup_core::fabrication::{
    FABRICATION_ROLE_DIMENSION_V1, GeneralManufacturingKind, ProjectionStatus, TIMBER_MATERIAL_V1,
    TIMBER_MEMBER_ROLE_V1,
};
use ketchup_core::persistence::{self, ContainerData};
use ketchup_core::prismatic::TolerancePolicy;
use ketchup_core::validation::{
    HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationReport,
    ValidationState,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Structural dimensions of the proof house, in millimetres.
const HOUSE_LENGTH_MM: f64 = 6_000.0;
const HOUSE_WIDTH_MM: f64 = 4_000.0;
const WALL_HEIGHT_MM: f64 = 2_600.0;
const TIMBER_WIDTH_MM: f64 = 100.0;
const TIMBER_DEPTH_MM: f64 = 60.0;
const PLATE_THICKNESS_MM: f64 = 60.0;
const STUD_SPACING_MM: f64 = 625.0;
const STUD_INSTANCES: u32 = 9;
const SHEATHING_THICKNESS_MM: f64 = 18.0;
const RIDGE_RISE_MM: f64 = 1_200.0;

/// A closed axis-aligned rectangle as four constrained sketch lines.
fn rectangle(
    width_mm: f64,
    height_mm: f64,
) -> (Vec<AssistantSketchEntity>, Vec<AssistantSketchConstraint>) {
    let corners = [
        [0.0, 0.0],
        [width_mm, 0.0],
        [width_mm, height_mm],
        [0.0, height_mm],
    ];
    let entities = (0..4)
        .map(|index| AssistantSketchEntity::Line {
            id: index as u64 + 1,
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect::<Vec<_>>();
    let constraints = (0..4)
        .map(|index| {
            let id = index as u64 + 1;
            if index % 2 == 0 {
                AssistantSketchConstraint::Horizontal { id, entity_id: id }
            } else {
                AssistantSketchConstraint::Vertical { id, entity_id: id }
            }
        })
        .collect::<Vec<_>>();
    (entities, constraints)
}

/// One extruded timber member placed by translation and optional rotation.
fn timber(
    name: &str,
    plan_width_mm: f64,
    plan_height_mm: f64,
    extrusion_mm: f64,
    translation_mm: [f64; 3],
    rotation: Option<AssistantCadRotation>,
) -> AssistantCadEditOperation {
    let (entities, constraints) = rectangle(plan_width_mm, plan_height_mm);
    AssistantCadEditOperation::CreatePart {
        name: name.to_owned(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities,
        constraints,
        feature: AssistantCadPartFeature::Extrusion {
            distance_mm: extrusion_mm,
        },
        translation_mm,
        rotation,
    }
}

fn wait_for_assistant_proposal(shell: &mut Shell) {
    let confirm = shell.catalog().text("assistant-confirm");
    for _ in 0..200 {
        shell.step();
        if shell.app().assistant_proposal().is_some() && shell.has_visible_label(&confirm) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "scripted assistant response did not reach accessible proposal review: {:?}",
        shell.app().assistant_messages()
    );
}

fn json_string_ending_with<'a>(value: &'a serde_json::Value, suffix: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::String(text) => text.ends_with(suffix).then_some(text),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| json_string_ending_with(value, suffix)),
        serde_json::Value::Object(values) => values
            .values()
            .find_map(|value| json_string_ending_with(value, suffix)),
        _ => None,
    }
}

fn wait_for_live_assistant_proposal(shell: &mut Shell) {
    let confirm = shell.catalog().text("assistant-confirm");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        shell.step();
        if shell.app().assistant_proposal().is_some() && shell.has_visible_label(&confirm) {
            return;
        }
        assert!(
            !shell
                .app()
                .assistant_messages()
                .iter()
                .any(|message| message.role == AssistantMessageRole::Error),
            "live assistant returned an error: {:?}",
            shell.app().assistant_messages()
        );
        assert!(
            Instant::now() < deadline,
            "live assistant did not reach proposal review: {:?}",
            shell.app().assistant_messages()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Send one natural-language request, prove preview mutates nothing, confirm it,
/// and prove the commit is exactly one revision and one undo step.
fn build_step(
    shell: &mut Shell,
    transport: &Arc<ScriptedAssistantTransport>,
    request: &str,
    program: AssistantCadEditProgram,
) {
    transport.queue_cad_edit_program(request, program);
    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(shell);

    assert_eq!(
        shell.app().document_revision(),
        revision_before,
        "{request}: preview must not change the revision"
    );
    assert_eq!(
        shell.app().canonical_digest(),
        digest_before,
        "{request}: preview must not change the canonical digest"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        undo_before,
        "{request}: preview must not add an undo step"
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(
        shell.app().document_revision(),
        revision_before + 1,
        "{request}: confirmation must add exactly one revision"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        undo_before + 1,
        "{request}: confirmation must add exactly one undo step"
    );
    assert_ne!(
        shell.app().canonical_digest(),
        digest_before,
        "{request}: confirmation must change the canonical digest"
    );
}

fn definition_id_of(shell: &Shell, name: &str) -> DefinitionId {
    shell
        .app()
        .document_snapshot()
        .definitions()
        .find(|definition| definition.name() == name)
        .unwrap_or_else(|| panic!("definition {name} must exist"))
        .id()
}

fn body_feature_id_of(shell: &Shell, name: &str) -> FeatureId {
    let snapshot = shell.app().document_snapshot();
    let definition = snapshot
        .definitions()
        .find(|definition| definition.name() == name)
        .unwrap_or_else(|| panic!("definition {name} must exist"));
    definition
        .feature_ids()
        .iter()
        .copied()
        .find(|id| matches!(snapshot.feature(*id).unwrap().kind(), FeatureKind::Pad(_)))
        .unwrap_or_else(|| panic!("definition {name} must own a pad"))
}

fn static_metadata_program(snapshot: &Snapshot) -> (AssistantCadEditProgram, usize) {
    let dimension_id = snapshot
        .classification_dimensions()
        .map(|dimension| dimension.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let first_category_id = snapshot
        .classification_dimensions()
        .flat_map(|dimension| dimension.categories())
        .map(|category| category.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let load_category_id = first_category_id;
    let support_category_id = first_category_id + 1;
    let fabrication_dimension_id = dimension_id + 1;
    let timber_category_id = first_category_id + 2;
    let mut next_node_id = snapshot
        .evaluator_nodes()
        .map(|node| node.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let support_ids = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            matches!(
                occurrence.occurrence_name.as_str(),
                "Foundation beam" | "Rear foundation beam"
            )
        })
        .map(|occurrence| occurrence.occurrence_id.0)
        .collect::<Vec<_>>();
    let load_ids = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.visible && !support_ids.contains(&occurrence.occurrence_id.0)
        })
        .map(|occurrence| occurrence.occurrence_id.0)
        .collect::<Vec<_>>();
    assert_eq!(support_ids.len(), 2);
    assert_eq!(load_ids.len(), 10);
    let all_ids = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.occurrence_id.0)
        .collect::<Vec<_>>();
    assert_eq!(all_ids.len(), 12);

    let mut operations = vec![
        AssistantCadEditOperation::UpsertClassificationDimension {
            dimension_id,
            name: "ketchup.validator-role.v1".to_owned(),
            categories: vec![
                AssistantCadClassificationCategory {
                    id: load_category_id,
                    name: "physics.static.load:live-house".to_owned(),
                },
                AssistantCadClassificationCategory {
                    id: support_category_id,
                    name: "physics.static.support:live-house".to_owned(),
                },
            ],
        },
        AssistantCadEditOperation::SetOccurrenceClassification {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: load_ids.clone(),
            },
            dimension_id,
            category_id: Some(load_category_id),
        },
        AssistantCadEditOperation::SetOccurrenceClassification {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: support_ids.clone(),
            },
            dimension_id,
            category_id: Some(support_category_id),
        },
        AssistantCadEditOperation::UpsertClassificationDimension {
            dimension_id: fabrication_dimension_id,
            name: FABRICATION_ROLE_DIMENSION_V1.to_owned(),
            categories: vec![AssistantCadClassificationCategory {
                id: timber_category_id,
                name: TIMBER_MEMBER_ROLE_V1.to_owned(),
            }],
        },
        AssistantCadEditOperation::SetOccurrenceClassification {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: all_ids,
            },
            dimension_id: fabrication_dimension_id,
            category_id: Some(timber_category_id),
        },
    ];
    for (name, value) in [
        ("physics.gravity_x_m_s2".to_owned(), 0.0),
        ("physics.gravity_y_m_s2".to_owned(), 0.0),
        ("physics.gravity_z_m_s2".to_owned(), -9.81),
    ] {
        operations.push(AssistantCadEditOperation::CreateEvaluatorInput {
            node_id: next_node_id,
            name,
            value,
        });
        next_node_id += 1;
    }
    for occurrence_id in &load_ids {
        for (prefix, value) in [
            ("physics.mass_kg.occurrence", 100.0),
            ("physics.applied_load_n.occurrence", 500.0),
        ] {
            operations.push(AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: next_node_id,
                name: format!("{prefix}.{occurrence_id}"),
                value,
            });
            next_node_id += 1;
        }
    }
    for occurrence_id in support_ids {
        operations.push(AssistantCadEditOperation::CreateEvaluatorInput {
            node_id: next_node_id,
            name: format!("physics.support_capacity_n.occurrence.{occurrence_id}"),
            value: 100_000.0,
        });
        next_node_id += 1;
    }
    assert_eq!(operations.len(), 30);
    (AssistantCadEditProgram { operations }, load_ids.len())
}

/// Drive the whole frame from an empty document through the Assistant, and
/// return the shell plus the baseline the build started from.
fn build_timber_frame_house() -> (Shell, Arc<ScriptedAssistantTransport>, u64, String) {
    let requests = [
        "Clear the document so the site is empty",
        "Lay the sill plate ring for a 6000 by 4000 mm timber frame house",
        "Raise the first stud on the front wall",
        "Repeat that stud every 625 mm and cap the wall with a top plate",
        "Sheath the front wall with an 18 mm panel",
        "Stand the gable posts, carry the ridge beam on them and add a rafter sloping up to it",
        "Mark every completed house occurrence as a timber member for fabrication",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new(requests.map(|request| {
        (
            request.to_owned(),
            AssistantChatResult {
                message: "Review the timber frame step.".to_owned(),
                model_intent: None,
            },
        )
    })));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();

    // 0. Clear whatever the new document opened with, so the site is empty.
    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    assert!(!existing.is_empty(), "a new document must start occupied");
    build_step(
        &mut shell,
        &transport,
        requests[0],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 0);

    // 1. Sill plate ring closing the whole footprint.
    let inner_width = HOUSE_WIDTH_MM - 2.0 * TIMBER_WIDTH_MM;
    build_step(
        &mut shell,
        &transport,
        requests[1],
        AssistantCadEditProgram {
            operations: vec![
                timber(
                    "Sill plate front",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, 0.0, 0.0],
                    None,
                ),
                timber(
                    "Sill plate back",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, HOUSE_WIDTH_MM - TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
                timber(
                    "Sill plate left",
                    TIMBER_WIDTH_MM,
                    inner_width,
                    PLATE_THICKNESS_MM,
                    [0.0, TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
                timber(
                    "Sill plate right",
                    TIMBER_WIDTH_MM,
                    inner_width,
                    PLATE_THICKNESS_MM,
                    [HOUSE_LENGTH_MM - TIMBER_WIDTH_MM, TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
            ],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 4);

    // 2. The first stud, standing on the sill.
    build_step(
        &mut shell,
        &transport,
        requests[2],
        AssistantCadEditProgram {
            operations: vec![timber(
                "Front stud",
                TIMBER_WIDTH_MM,
                TIMBER_DEPTH_MM,
                WALL_HEIGHT_MM,
                [0.0, SHEATHING_THICKNESS_MM, PLATE_THICKNESS_MM],
                None,
            )],
        },
    );
    let stud_occurrence = shell
        .app()
        .document_snapshot()
        .occurrences()
        .find(|occurrence| occurrence.name() == "Front stud")
        .expect("stud occurrence must exist")
        .id();

    // 3. A real linear pattern of studs, plus the top plate, as one step.
    let before_pattern = shell.app().document_snapshot().occurrences().count();
    build_step(
        &mut shell,
        &transport,
        requests[3],
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::LinearPattern {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![stud_occurrence.0],
                    },
                    instances: STUD_INSTANCES,
                    step_mm: [STUD_SPACING_MM, 0.0, 0.0],
                },
                timber(
                    "Top plate front",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, 0.0, PLATE_THICKNESS_MM + WALL_HEIGHT_MM],
                    None,
                ),
            ],
        },
    );
    let after_pattern = shell.app().document_snapshot().occurrences().count();
    assert!(
        after_pattern > before_pattern + 1,
        "the linear pattern must add real stud instances"
    );
    let stud_definition = definition_id_of(&shell, "Front stud");
    let patterned_studs = shell
        .app()
        .document_snapshot()
        .occurrences()
        .filter(|occurrence| occurrence.definition_id() == stud_definition)
        .count();
    assert_eq!(
        patterned_studs, STUD_INSTANCES as usize,
        "every stud instance must reuse the same parametric definition"
    );

    // 4. Front sheathing standing on the XZ plane.
    let (sheathing_entities, sheathing_constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    build_step(
        &mut shell,
        &transport,
        requests[4],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: sheathing_entities,
                constraints: sheathing_constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [0.0, 0.0, PLATE_THICKNESS_MM],
                rotation: None,
            }],
        },
    );

    // 5. Gable posts, the ridge beam they carry, and one rafter placed by an
    //    arbitrary finite rotation. The rafter runs from the outer top arris of
    //    the plate up to the near face of the ridge, so every roof member is
    //    carried by something that reaches the ground.
    let eaves_height = PLATE_THICKNESS_MM + WALL_HEIGHT_MM + PLATE_THICKNESS_MM;
    let ridge_near_face_mm = HOUSE_WIDTH_MM / 2.0 - TIMBER_WIDTH_MM / 2.0;
    let ridge_soffit_mm = eaves_height + RIDGE_RISE_MM;
    let rafter_run = ridge_near_face_mm;
    let rafter_length = rafter_run.hypot(RIDGE_RISE_MM);
    let rafter_pitch_degrees = RIDGE_RISE_MM.atan2(rafter_run).to_degrees();
    build_step(
        &mut shell,
        &transport,
        requests[5],
        AssistantCadEditProgram {
            operations: vec![
                timber(
                    "Gable post left",
                    TIMBER_WIDTH_MM,
                    TIMBER_WIDTH_MM,
                    ridge_soffit_mm - PLATE_THICKNESS_MM,
                    [0.0, ridge_near_face_mm, PLATE_THICKNESS_MM],
                    None,
                ),
                timber(
                    "Gable post right",
                    TIMBER_WIDTH_MM,
                    TIMBER_WIDTH_MM,
                    ridge_soffit_mm - PLATE_THICKNESS_MM,
                    [
                        HOUSE_LENGTH_MM - TIMBER_WIDTH_MM,
                        ridge_near_face_mm,
                        PLATE_THICKNESS_MM,
                    ],
                    None,
                ),
                timber(
                    "Ridge beam",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    2.0 * TIMBER_WIDTH_MM,
                    [0.0, ridge_near_face_mm, ridge_soffit_mm],
                    None,
                ),
                timber(
                    "Rafter",
                    TIMBER_WIDTH_MM,
                    rafter_length,
                    TIMBER_DEPTH_MM,
                    [0.0, 0.0, eaves_height],
                    Some(AssistantCadRotation {
                        pivot_mm: [0.0, 0.0, eaves_height],
                        axis: [1.0, 0.0, 0.0],
                        angle_degrees: rafter_pitch_degrees,
                    }),
                ),
            ],
        },
    );

    let snapshot = shell.app().document_snapshot();
    let dimension_id = snapshot
        .classification_dimensions()
        .map(|dimension| dimension.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let category_id = snapshot
        .classification_dimensions()
        .flat_map(|dimension| dimension.categories())
        .map(|category| category.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let occurrence_ids = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.occurrence_id.0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        &transport,
        requests[6],
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::UpsertClassificationDimension {
                    dimension_id,
                    name: FABRICATION_ROLE_DIMENSION_V1.to_owned(),
                    categories: vec![AssistantCadClassificationCategory {
                        id: category_id,
                        name: TIMBER_MEMBER_ROLE_V1.to_owned(),
                    }],
                },
                AssistantCadEditOperation::SetOccurrenceClassification {
                    selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
                    dimension_id,
                    category_id: Some(category_id),
                },
            ],
        },
    );

    assert_eq!(transport.remaining_responses(), 0);
    (shell, transport, baseline_revision, baseline_digest)
}

/// Every structural member the Assistant authored, by name.
const HOUSE_MEMBERS: [&str; 11] = [
    "Sill plate front",
    "Sill plate back",
    "Sill plate left",
    "Sill plate right",
    "Front stud",
    "Top plate front",
    "Front sheathing",
    "Gable post left",
    "Gable post right",
    "Ridge beam",
    "Rafter",
];

/// Prove the production OAuth Assistant can author and extend real exact
/// structure across revision-bound turns instead of replaying a Rust-authored
/// program, including a pitched roof authored as general prismatic parts.
#[test]
#[ignore = "requires the production OAuth binary, login and live GPT-5.6 requests"]
fn live_oauth_assistant_builds_a_roofed_house_frame_across_turns() {
    let mut shell = Shell::new();
    shell.app_mut().set_assistant_diagnostics_enabled(true);
    let handshake = shell.app().assistant_handshake();
    assert_eq!(handshake.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(handshake.provider, "codex-oauth");
    assert_eq!(handshake.model, "gpt-5.6-sol");

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    assert!(shell.app().document_snapshot().occurrences().count() > 0);

    let request = "Use one typed cad_edit_program, not model_intent and not prose alone. First delete every existing occurrence listed in the current document context with remove_references so the site is empty. Then create exactly four separate rectangular extruded parts on the XY plane, all dimensions in millimetres: 'Foundation beam' is 4000 by 400 and extruded 300 at translation [0,0,0]; 'Left post' is 200 by 200 and extruded 2500 at [0,0,300]; 'Right post' is 200 by 200 and extruded 2500 at [3800,0,300]; 'Header beam' is 4000 by 200 and extruded 200 at [0,0,2800]. Omit the optional rotation field entirely for every part. Draw the Foundation beam rectangle with corners [0,0], [4000,0], [4000,400], [0,400]; both post rectangles with corners [0,0], [200,0], [200,200], [0,200]; and the Header beam rectangle with corners [0,0], [4000,0], [4000,200], [0,200]. Close each rectangle with four line entities and set constraints to an empty array for every part. Do not emit any constraint object. Do not add any other part and do not approximate with mesh geometry.";
    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert!(shell.app().assistant_messages().iter().any(|message| {
        message.role == AssistantMessageRole::Assistant
            && message.source.contains("gpt-5.6-sol")
            && !message.text.trim().is_empty()
    }));
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the live provider response must retain bounded diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the captured provider response must be JSON");
    assert_eq!(
        provider_response["cad_edit_program"]["operations"]
            .as_array()
            .expect("GPT-5.6 must return a typed CAD program")
            .len(),
        5
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), 4);
    let expected = [
        ("Foundation beam", [4_000.0, 400.0, 300.0], [0.0, 0.0, 0.0]),
        ("Left post", [200.0, 200.0, 2_500.0], [0.0, 0.0, 300.0]),
        ("Right post", [200.0, 200.0, 2_500.0], [3_800.0, 0.0, 300.0]),
        ("Header beam", [4_000.0, 200.0, 200.0], [0.0, 0.0, 2_800.0]),
    ];
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    for &(name, size, translation) in &expected {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist"));
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!([matrix[3], matrix[7], matrix[11]], translation);
        assert_eq!(
            [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );

        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("live-authored {name} must compile exactly: {error}"));
        let package = worker
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("live-authored {name} must evaluate in OCCT: {error}"));
        let expected_volume = size.iter().product::<f64>();
        assert!((package.volume_mm3 - expected_volume).abs() <= expected_volume * 1.0e-9);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], size]);
        assert_eq!(package.topology_counts, [8, 12, 6, 1, 1]);
    }

    let first_identities = expected.map(|(name, _, _)| {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist before turn two"));
        (
            name,
            occurrence.id(),
            occurrence.definition_id(),
            occurrence.transform(),
        )
    });
    let first_revision = shell.app().document_revision();
    let first_digest = shell.app().canonical_digest();
    let second_request = "Use one typed cad_edit_program, not model_intent and not prose alone. Extend the existing front portal into a supported 4000 by 4000 mm house frame by creating exactly six additional separate rectangular extruded parts on the XY plane. Keep every existing occurrence and do not modify or delete it. 'Rear foundation beam' is 4000 by 400 and extruded 300 at translation [0,3600,0]. 'Rear left post' and 'Rear right post' are each 200 by 200 and extruded 2500 at [0,3800,300] and [3800,3800,300]. 'Rear header beam' is 4000 by 200 and extruded 200 at [0,3800,2800]. 'Left top tie' and 'Right top tie' are each 200 by 3600 and extruded 200 at [0,200,2800] and [3800,200,2800]. Draw each 4000 by 400 rectangle with corners [0,0], [4000,0], [4000,400], [0,400]; each 200 by 200 rectangle with corners [0,0], [200,0], [200,200], [0,200]; the 4000 by 200 rectangle with corners [0,0], [4000,0], [4000,200], [0,200]; and each 200 by 3600 rectangle with corners [0,0], [200,0], [200,3600], [0,3600]. Close every rectangle with four line entities whose objects use exactly the keys type, id, start_mm and end_mm; never use start or end. Set constraints to an empty array and omit the optional rotation field entirely for every part. Do not emit any constraint object, add any other part, or approximate with mesh geometry.";
    shell.focus_text_input(&input);
    shell.type_text(second_request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), first_revision);
    assert_eq!(shell.app().canonical_digest(), first_digest);
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the second live provider response must retain bounded diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let second_provider_message =
        json_string_ending_with(&diagnostics.request_payload, second_request)
            .expect("the provider payload must contain the second request");
    let (document_context, provider_prompt) = second_provider_message
        .strip_prefix("<document-context>")
        .and_then(|message| message.split_once("</document-context>\n\n"))
        .expect("the second request must carry a serialized document context");
    assert_eq!(provider_prompt, second_request);
    let document_context: serde_json::Value = serde_json::from_str(document_context)
        .expect("the second provider context must remain valid JSON");
    assert_eq!(document_context["revision"], first_revision);
    assert_eq!(document_context["canonical_digest"], first_digest);
    assert_eq!(document_context["occurrence_count"], 4);
    let conversation = document_context["conversation"]
        .as_array()
        .expect("the second context must retain the first app turn");
    assert!(
        conversation.iter().any(|message| {
            message["role"] == "user" && message["text"].as_str() == Some(request)
        })
    );
    assert!(conversation.iter().any(|message| {
        message["role"] == "assistant"
            && message["text"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
    }));
    for (name, _, _) in expected {
        assert!(
            document_context["occurrences"]
                .as_array()
                .expect("the context must list the first-turn occurrences")
                .iter()
                .any(|occurrence| occurrence["name"] == name),
            "the second provider context must expose {name}"
        );
    }

    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the second captured provider response must be JSON");
    let operations = provider_response["cad_edit_program"]["operations"]
        .as_array()
        .expect("GPT-5.6 must return a second typed CAD program");
    assert_eq!(operations.len(), 6);
    assert!(
        operations
            .iter()
            .all(|operation| operation["operation"] == "create_part"),
        "turn two may only append the six requested parts"
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), first_revision + 1);
    assert_ne!(shell.app().canonical_digest(), first_digest);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), 10);
    for (name, occurrence_id, definition_id, transform) in first_identities {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("turn two must preserve {name}"));
        assert_eq!(occurrence.id(), occurrence_id);
        assert_eq!(occurrence.definition_id(), definition_id);
        assert_eq!(occurrence.transform(), transform);
    }
    let extension = [
        (
            "Rear foundation beam",
            [4_000.0, 400.0, 300.0],
            [0.0, 3_600.0, 0.0],
        ),
        (
            "Rear left post",
            [200.0, 200.0, 2_500.0],
            [0.0, 3_800.0, 300.0],
        ),
        (
            "Rear right post",
            [200.0, 200.0, 2_500.0],
            [3_800.0, 3_800.0, 300.0],
        ),
        (
            "Rear header beam",
            [4_000.0, 200.0, 200.0],
            [0.0, 3_800.0, 2_800.0],
        ),
        (
            "Left top tie",
            [200.0, 3_600.0, 200.0],
            [0.0, 200.0, 2_800.0],
        ),
        (
            "Right top tie",
            [200.0, 3_600.0, 200.0],
            [3_800.0, 200.0, 2_800.0],
        ),
    ];
    for (name, size, translation) in extension {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist"));
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!([matrix[3], matrix[7], matrix[11]], translation);
        assert_eq!(
            [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("live-authored {name} must compile exactly: {error}"));
        let package = worker
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("live-authored {name} must evaluate in OCCT: {error}"));
        let expected_volume = size.iter().product::<f64>();
        assert!((package.volume_mm3 - expected_volume).abs() <= expected_volume * 1.0e-9);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], size]);
        assert_eq!(package.topology_counts, [8, 12, 6, 1, 1]);
    }

    let frame_identities = committed
        .occurrences()
        .map(|occurrence| {
            (
                occurrence.name().to_owned(),
                occurrence.id(),
                occurrence.definition_id(),
                occurrence.transform(),
            )
        })
        .collect::<Vec<_>>();
    let frame_graphs = frame_identities
        .iter()
        .map(|(name, _, definition_id, _)| {
            let body = body_feature_id_of(&shell, name);
            let graph = ExactBRepGraph::from_snapshot(&committed, *definition_id, body)
                .unwrap_or_else(|error| {
                    panic!("{name} must compile before the roof turn: {error}")
                });
            (
                name.clone(),
                graph.definition_id,
                graph.producer_feature_id,
                graph.profiles,
                graph.nodes,
            )
        })
        .collect::<Vec<_>>();
    let frame_revision = shell.app().document_revision();
    let frame_digest = shell.app().canonical_digest();
    let roof_request = "Use one typed cad_edit_program, not model_intent and not prose alone. Add a pitched roof to the existing 4000 by 4000 mm house frame by creating exactly two separate prismatic parts. Keep every existing occurrence unchanged and do not delete it. Create both parts on the YZ principal plane, extruded 4000 mm through the house length, translated [0,0,3000], with constraints as an empty array and with the optional rotation field omitted. 'Left roof plane' has one closed four-line profile with corners [0,0], [2000,1000], [2000,1100], [0,100]. 'Right roof plane' has one closed four-line profile with corners [2000,1000], [4000,0], [4000,100], [2000,1100]. For each profile use four line entity objects with exactly the keys type, id, start_mm and end_mm, where each end_mm equals the next line's start_mm and the fourth end_mm closes at the first start_mm. Use extrusion as the part feature with distance_mm 4000. Do not emit any constraint object, add any other part, or approximate with mesh geometry.";
    shell.focus_text_input(&input);
    shell.type_text(roof_request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), frame_revision);
    assert_eq!(shell.app().canonical_digest(), frame_digest);
    let proposal = shell
        .app()
        .assistant_proposal()
        .expect("the roof response must create a reviewable proposal");
    assert_eq!(proposal.provenance_revision(), frame_revision);
    assert_eq!(proposal.provenance_digest(), frame_digest);
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the roof response must retain bounded provider diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let roof_provider_message = json_string_ending_with(&diagnostics.request_payload, roof_request)
        .expect("the provider payload must contain the roof request");
    let (document_context, provider_prompt) = roof_provider_message
        .strip_prefix("<document-context>")
        .and_then(|message| message.split_once("</document-context>\n\n"))
        .expect("the roof request must carry serialized frame context");
    assert_eq!(provider_prompt, roof_request);
    let document_context: serde_json::Value = serde_json::from_str(document_context)
        .expect("the roof provider context must remain valid JSON");
    assert_eq!(document_context["revision"], frame_revision);
    assert_eq!(document_context["canonical_digest"], frame_digest);
    assert_eq!(document_context["occurrence_count"], 10);
    assert!(
        document_context["conversation"]
            .as_array()
            .expect("the roof context must retain earlier turns")
            .iter()
            .any(|message| {
                message["role"] == "user" && message["text"].as_str() == Some(second_request)
            })
    );
    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the captured roof provider response must be JSON");
    let operations = provider_response["cad_edit_program"]["operations"]
        .as_array()
        .expect("GPT-5.6 must return a typed roof CAD program");
    assert!(
        operations
            .iter()
            .all(|operation| operation.get("rotation").is_none()),
        "the provider must omit rotation instead of changing the roof placement"
    );
    let profile = |corners: [[f64; 2]; 4]| {
        (0..4)
            .map(|index| AssistantSketchEntity::Line {
                id: index as u64 + 1,
                start_mm: corners[index],
                end_mm: corners[(index + 1) % 4],
            })
            .collect::<Vec<_>>()
    };
    let expected_roof_program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreatePart {
                name: "Left roof plane".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Yz,
                },
                entities: profile([
                    [0.0, 0.0],
                    [2_000.0, 1_000.0],
                    [2_000.0, 1_100.0],
                    [0.0, 100.0],
                ]),
                constraints: Vec::new(),
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: 4_000.0,
                },
                translation_mm: [0.0, 0.0, 3_000.0],
                rotation: None,
            },
            AssistantCadEditOperation::CreatePart {
                name: "Right roof plane".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Yz,
                },
                entities: profile([
                    [2_000.0, 1_000.0],
                    [4_000.0, 0.0],
                    [4_000.0, 100.0],
                    [2_000.0, 1_100.0],
                ]),
                constraints: Vec::new(),
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: 4_000.0,
                },
                translation_mm: [0.0, 0.0, 3_000.0],
                rotation: None,
            },
        ],
    };
    let roof_program = serde_json::from_value::<AssistantCadEditProgram>(
        provider_response["cad_edit_program"].clone(),
    )
    .expect("the roof response must deserialize through the public typed contract");
    assert_eq!(roof_program, expected_roof_program);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), frame_revision + 1);
    assert_ne!(shell.app().canonical_digest(), frame_digest);
    let roof_revision = shell.app().document_revision();
    let roof_digest = shell.app().canonical_digest();
    let roof_snapshot = shell.app().document_snapshot();
    let (expected_static_program, static_load_count) = static_metadata_program(&roof_snapshot);
    let static_program_json = serde_json::to_string(&expected_static_program).unwrap();
    let static_request = format!(
        "Use one typed cad_edit_program, not model_intent and not prose alone. Keep all 12 existing occurrences and their geometry unchanged. Add the canonical classification roles and explicit numeric evaluator inputs for a complete static-load check by returning exactly this cad_edit_program object: {static_program_json} Do not add, delete, transform or recolor any occurrence and do not emit any additional operation."
    );
    shell.focus_text_input(&input);
    shell.type_text(&static_request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), roof_revision);
    assert_eq!(shell.app().canonical_digest(), roof_digest);
    let proposal = shell
        .app()
        .assistant_proposal()
        .expect("the static metadata response must create a reviewable proposal");
    assert_eq!(proposal.provenance_revision(), roof_revision);
    assert_eq!(proposal.provenance_digest(), roof_digest);
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the static metadata response must retain provider diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let static_provider_message =
        json_string_ending_with(&diagnostics.request_payload, &static_request)
            .expect("the provider payload must contain the static metadata request");
    let (document_context, provider_prompt) = static_provider_message
        .strip_prefix("<document-context>")
        .and_then(|message| message.split_once("</document-context>\n\n"))
        .expect("the static metadata request must carry serialized roof context");
    assert_eq!(provider_prompt, static_request);
    let document_context: serde_json::Value = serde_json::from_str(document_context)
        .expect("the static metadata provider context must remain valid JSON");
    assert_eq!(document_context["revision"], roof_revision);
    assert_eq!(document_context["canonical_digest"], roof_digest);
    assert_eq!(document_context["occurrence_count"], 12);
    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the captured static metadata provider response must be JSON");
    let static_program = serde_json::from_value::<AssistantCadEditProgram>(
        provider_response["cad_edit_program"].clone(),
    )
    .expect("the static metadata response must deserialize through the public typed contract");
    assert_eq!(static_program, expected_static_program);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), roof_revision + 1);
    assert_ne!(shell.app().canonical_digest(), roof_digest);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), 12);
    for (name, occurrence_id, definition_id, transform) in frame_identities {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("the roof turn must preserve {name}"));
        assert_eq!(occurrence.id(), occurrence_id);
        assert_eq!(occurrence.definition_id(), definition_id);
        assert_eq!(occurrence.transform(), transform);
    }
    for (name, definition_id, producer_feature_id, profiles, nodes) in frame_graphs {
        let body = body_feature_id_of(&shell, &name);
        let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(definition_id), body)
            .unwrap_or_else(|error| panic!("{name} must compile after the roof turn: {error}"));
        assert_eq!(graph.producer_feature_id, producer_feature_id);
        assert_eq!(graph.profiles, profiles);
        assert_eq!(graph.nodes, nodes);
    }
    for (name, expected_bounds) in [
        (
            "Left roof plane",
            [[0.0, 0.0, 0.0], [4_000.0, 2_000.0, 1_100.0]],
        ),
        (
            "Right roof plane",
            [[0.0, 2_000.0, 0.0], [4_000.0, 4_000.0, 1_100.0]],
        ),
    ] {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist"));
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!([matrix[3], matrix[7], matrix[11]], [0.0, 0.0, 3_000.0]);
        assert_eq!(
            [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("live-authored {name} must compile exactly: {error}"));
        let package = worker
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("live-authored {name} must evaluate in OCCT: {error}"));
        assert!((package.volume_mm3 - 800_000_000.0).abs() <= 0.8);
        for axis in 0..3 {
            assert!((package.bounds_mm[0][axis] - expected_bounds[0][axis]).abs() <= 1.0e-6);
            assert!((package.bounds_mm[1][axis] - expected_bounds[1][axis]).abs() <= 1.0e-6);
        }
        let support_y_mm = if name == "Left roof plane" {
            0.0
        } else {
            4_000.0
        };
        for support_x_mm in [0.0, 4_000.0] {
            assert!(
                package.vertices.iter().any(|vertex| {
                    (vertex.position_mm[0] - support_x_mm).abs() <= 1.0e-6
                        && (vertex.position_mm[1] - support_y_mm).abs() <= 1.0e-6
                        && vertex.position_mm[2].abs() <= 1.0e-6
                }),
                "{name} must have an exact lower OCCT edge on its supporting header"
            );
        }
        assert_eq!(package.topology_counts, [8, 12, 6, 1, 1]);
    }

    let tolerance = TolerancePolicy::default();
    let packages = committed
        .occurrences()
        .map(|occurrence| {
            let name = occurrence.name();
            let body = body_feature_id_of(&shell, name);
            let graph = ExactBRepGraph::from_snapshot(&committed, occurrence.definition_id(), body)
                .unwrap_or_else(|error| {
                    panic!("live-authored {name} must compile for validation: {error}")
                });
            let package = worker
                .evaluate_exact_brep_graph(&graph)
                .unwrap_or_else(|error| {
                    panic!("live-authored {name} must evaluate for validation: {error}")
                });
            Arc::new(ExactBodyPackage::from(package))
        })
        .collect::<Vec<_>>();
    let registry = ExactResultRegistry::accept(&committed, packages).unwrap();
    let participants = committed
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                &committed,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be an accepted live validation body: {error:?}",
                    occurrence.occurrence_name
                )
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(participants.len(), 12);

    let mut collision_cases = Vec::new();
    for left in 0..participants.len() {
        for right in (left + 1)..participants.len() {
            collision_cases.push(
                GeneralClearanceCase::new(
                    participants[left].clone(),
                    participants[right].clone(),
                    0.0,
                )
                .unwrap(),
            );
        }
    }
    assert_eq!(collision_cases.len(), 66);
    let collision_report = general_report(&committed, &collision_cases, tolerance);
    assert!(collision_report.invocation.is_current(&committed));
    assert_eq!(collision_report.diagnostics.len(), collision_cases.len());
    assert!(
        collision_report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "collision.none"),
        "{:#?}",
        collision_report.diagnostics
    );
    assert_eq!(
        collision_report.state,
        ValidationState::Passed,
        "{:#?}",
        collision_report.diagnostics
    );

    let grounded_foundations = committed
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.visible
                && matches!(
                    occurrence.occurrence_name.as_str(),
                    "Foundation beam" | "Rear foundation beam"
                )
        })
        .map(|occurrence| occurrence.occurrence_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(grounded_foundations.len(), 2);
    let gravity_participants = participants
        .iter()
        .cloned()
        .map(|body| {
            let occurrence_id = body.instance_path().root_occurrence();
            let occurrence = committed
                .occurrence(occurrence_id)
                .expect("every validation body must retain its final occurrence identity");
            assert!(
                matches!(
                    occurrence.name(),
                    "Foundation beam" | "Rear foundation beam"
                ) == grounded_foundations.contains(&occurrence_id)
            );
            GravitySupportParticipant::new(
                body,
                "live-house",
                grounded_foundations.contains(&occurrence_id),
            )
        })
        .collect::<Vec<_>>();
    let gravity_input = GravitySupportInput::new(gravity_participants, [0.0, 0.0, -9.81]).unwrap();
    let gravity_validator = BuiltinGravitySupportValidator::new(tolerance);
    let gravity_policy = gravity_support_validation_policy();
    let gravity_bytes = gravity_support_input_bytes(&gravity_input);
    let gravity_invocation = ValidationInvocation::bind(
        &committed,
        gravity_validator.descriptor(),
        &gravity_policy,
        Vec::new(),
        &gravity_bytes,
    );
    let gravity_report = gravity_validator.invoke(ValidationExecution {
        snapshot: &committed,
        invocation: gravity_invocation,
        policy: &gravity_policy,
        input: &gravity_input,
    });
    assert!(gravity_report.invocation.is_current(&committed));
    assert_eq!(gravity_report.diagnostics.len(), participants.len());
    assert!(
        gravity_report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "gravity.unsupported"),
        "{:#?}",
        gravity_report.diagnostics
    );
    assert_eq!(
        gravity_report.state,
        ValidationState::Passed,
        "{:#?}",
        gravity_report.diagnostics
    );

    let static_validation = assistant_validation_context_with_worker(
        &committed,
        &registry,
        &AssistantValidationSelection::only(&["static_load"]),
        &ContainerData::default(),
        Some(exact_worker_path()),
        Duration::from_secs(30),
    );
    assert_eq!(static_validation["revision"], committed.revision_id());
    assert_eq!(
        static_validation["canonical_digest"],
        committed.canonical_digest()
    );
    assert_eq!(
        static_validation["state"], "passed",
        "{static_validation:#}"
    );
    assert_eq!(static_validation["complete"], true, "{static_validation:#}");
    assert_eq!(
        static_validation["static_load"]["applicable_count"],
        static_load_count
    );
    assert_eq!(static_validation["static_load"]["issue_count"], 0);

    let fabrication = ketchup_core::fabrication::project_general_fabrication(
        &committed,
        &registry,
        &collision_cases,
        &collision_report,
        tolerance,
    )
    .expect("the final live-authored revision must project a manufacturing handoff");
    assert_eq!(fabrication.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(
        fabrication.drawings.envelope.status,
        ProjectionStatus::Complete
    );
    assert_eq!(
        fabrication.manufacturing.envelope.status,
        ProjectionStatus::Complete
    );
    assert!(fabrication.bom.envelope.is_current(&committed));
    assert!(fabrication.drawings.envelope.is_current(&committed));
    assert!(fabrication.manufacturing.envelope.is_current(&committed));
    assert_eq!(fabrication.bom.rows.len(), participants.len());
    assert!(
        fabrication
            .bom
            .rows
            .iter()
            .all(|row| row.material_key == TIMBER_MATERIAL_V1)
    );
    assert_eq!(fabrication.drawings.drawings.len(), participants.len());
    assert_eq!(
        fabrication.manufacturing.operations.len(),
        participants.len()
    );
    assert!(fabrication.manufacturing.unresolved_sources.is_empty());
    assert!(
        fabrication
            .manufacturing
            .operations
            .iter()
            .all(|operation| operation.kind == GeneralManufacturingKind::Stock)
    );
    let bom = fabrication.bom_export(&committed).unwrap();
    let drawings = fabrication.drawing_svg(&committed).unwrap();
    let manufacturing = fabrication.manufacturing_export(&committed).unwrap();
    let bom_text = String::from_utf8_lossy(&bom);
    assert!(bom_text.contains("ketchup.general-bom-export.v1"));
    assert!(bom_text.contains(&format!("source_revision={}", committed.revision_id())));
    assert!(bom_text.contains(&format!("source_digest={}", committed.canonical_digest())));
    assert!(String::from_utf8_lossy(&drawings).contains("ketchup.general-drawing-svg.v1"));
    assert!(String::from_utf8_lossy(&manufacturing).contains("kind=stock"));
    publish_artifact("live-oauth-roofed-house-bom.txt", &bom);
    publish_artifact("live-oauth-roofed-house-drawings.svg", &drawings);
    publish_artifact("live-oauth-roofed-house-manufacturing.txt", &manufacturing);

    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("live-oauth-roofed-house-frame.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    assert_eq!(
        outcome.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    publish_artifact(
        "live-oauth-roofed-house-frame.ketchup",
        &std::fs::read(path).unwrap(),
    );

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), roof_revision);
    assert_eq!(shell.app().canonical_digest(), roof_digest);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), frame_revision);
    assert_eq!(shell.app().canonical_digest(), frame_digest);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), first_revision);
    assert_eq!(shell.app().canonical_digest(), first_digest);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

/// Prove the built frame is exact, editable, savable and undoable.
#[test]
fn assistant_builds_a_timber_frame_house_from_an_empty_document() {
    let (mut shell, _transport, baseline_revision, baseline_digest) = build_timber_frame_house();

    // The rafter must really be tilted, not axis-aligned like every other member.
    let snapshot = shell.app().document_snapshot();
    let rafter_transform = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Rafter")
        .expect("rafter occurrence must exist")
        .transform();
    let rafter = rafter_transform.matrix();
    assert!(
        rafter[6].abs() > 0.1 && rafter[10].abs() > 0.1,
        "the rafter must carry a real out-of-plane rotation, got {rafter:?}"
    );

    // Every structural member must stay an editable exact chain, never a mesh.
    let committed = shell.app().document_snapshot();
    for definition in committed.definitions() {
        for feature_id in definition.feature_ids() {
            assert!(
                !matches!(
                    committed.feature(*feature_id).unwrap().kind(),
                    FeatureKind::MeshBody(_)
                ),
                "{} must stay an editable feature chain",
                definition.name()
            );
        }
    }

    // Every generated member must compile into the general exact BRep graph.
    for name in HOUSE_MEMBERS {
        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("{name} must compile into an exact graph: {error}"));
    }

    // Save and reopen: the whole house must round-trip losslessly and stay editable.
    let committed_digest = committed.canonical_digest();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("timber-frame-house.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    assert_eq!(outcome.snapshot().canonical_digest(), committed_digest);
    publish_artifact("timber-frame-house.ketchup", &std::fs::read(&path).unwrap());

    // The whole build must unwind step by step back to the empty document.
    while shell.app().document_revision() > baseline_revision {
        assert!(shell.app_mut().undo());
    }
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

/// Copy a produced artifact next to the operator when `KETCHUP_HOUSE_OUT` is set,
/// so the proof house can be opened and inspected instead of only asserted on.
fn publish_artifact(name: &str, bytes: &[u8]) {
    let Some(directory) = std::env::var_os("KETCHUP_HOUSE_OUT") else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(name), bytes).unwrap();
}

fn exact_worker_path() -> PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
}

fn general_report(
    snapshot: &Snapshot,
    cases: &[GeneralClearanceCase],
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let input = general_body_input_bytes(cases);
    let invocation =
        ValidationInvocation::bind(snapshot, validator.descriptor(), &policy, vec![], &input);
    validator.invoke(ValidationExecution {
        snapshot,
        invocation,
        policy: &policy,
        input: cases,
    })
}

/// The Assistant-built house must carry all the way to a manufacturable
/// handoff: a bill of materials, per-piece drawings and machining operations,
/// each exportable only while it still matches the document it came from.
#[test]
fn the_timber_frame_house_projects_a_manufacturable_handoff() {
    let (shell, _transport, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

    // Solve every member exactly, through the same worker the app uses.
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let packages = HOUSE_MEMBERS
        .iter()
        .map(|name| {
            let definition_id = definition_id_of(&shell, name);
            let request = ExactFeatureChainRequest::from_snapshot(&snapshot, definition_id)
                .unwrap_or_else(|error| panic!("{name} must yield an exact request: {error}"));
            let package = worker
                .evaluate_rectangle(&request)
                .unwrap_or_else(|error| panic!("{name} must solve exactly: {error}"));
            Arc::new(ExactBodyPackage::from(package))
        })
        .collect::<Vec<_>>();
    let registry = ExactResultRegistry::accept(&snapshot, packages).unwrap();

    // Cover every visible member with a clearance case; members touch, so the
    // required minimum is zero and interference would still be caught.
    let participants = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be an accepted general body: {error:?}",
                    occurrence.occurrence_name
                )
            })
        })
        .collect::<Vec<_>>();
    let visible_definitions = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.definition_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        participants.len() >= 8 + STUD_INSTANCES as usize,
        "every house member must be an accepted general body"
    );
    let cases = (0..participants.len())
        .map(|index| {
            GeneralClearanceCase::new(
                participants[index].clone(),
                participants[(index + 1) % participants.len()].clone(),
                0.0,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let report = general_report(&snapshot, &cases, tolerance);
    assert_eq!(
        report.state,
        ValidationState::Passed,
        "{:#?}",
        report.diagnostics
    );

    let projection = ketchup_core::fabrication::project_general_fabrication(
        &snapshot, &registry, &cases, &report, tolerance,
    )
    .unwrap();
    assert_eq!(
        projection,
        ketchup_core::fabrication::project_general_fabrication(
            &snapshot, &registry, &cases, &report, tolerance,
        )
        .unwrap(),
        "the manufacturable handoff must regenerate deterministically"
    );

    // One bill-of-materials row per distinct member, with the studs pooled.
    assert_eq!(projection.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(projection.bom.rows.len(), visible_definitions.len());
    assert!(
        projection
            .bom
            .rows
            .iter()
            .all(|row| row.material_key == TIMBER_MATERIAL_V1)
    );
    for name in HOUSE_MEMBERS {
        let definition_id = definition_id_of(&shell, name);
        assert!(
            projection
                .bom
                .rows
                .iter()
                .any(|row| row.definition_id == definition_id),
            "{name} must appear in the bill of materials"
        );
    }
    let stud_definition = definition_id_of(&shell, "Front stud");
    let stud_row = projection
        .bom
        .rows
        .iter()
        .find(|row| row.definition_id == stud_definition)
        .expect("the studs must appear as one pooled row");
    assert_eq!(stud_row.quantity, STUD_INSTANCES as usize);
    assert_eq!(stud_row.dimensions.height_mm, WALL_HEIGHT_MM);
    for row in &projection.bom.rows {
        assert_eq!(row.validation_state, ValidationState::Passed);
    }

    // Every member must get a drawing, and every drawing three views.
    assert_eq!(
        projection.drawings.drawings.len(),
        visible_definitions.len()
    );
    for drawing in &projection.drawings.drawings {
        assert_eq!(drawing.views.len(), 3);
        assert_eq!(drawing.dimensions.len(), 3);
    }

    // Every member must resolve to a machining operation, none left unresolved.
    assert_eq!(
        projection.manufacturing.operations.len(),
        visible_definitions.len()
    );
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    for operation in &projection.manufacturing.operations {
        assert_eq!(operation.kind, GeneralManufacturingKind::Stock);
    }

    // All three exports must succeed against the document they describe.
    let bom = String::from_utf8(projection.bom_export(&snapshot).unwrap()).unwrap();
    assert!(bom.contains(&format!("quantity={}", STUD_INSTANCES)));
    let drawings = String::from_utf8(projection.drawing_svg(&snapshot).unwrap()).unwrap();
    assert!(drawings.contains("ketchup.general-drawing-svg.v1"));
    let manufacturing =
        String::from_utf8(projection.manufacturing_export(&snapshot).unwrap()).unwrap();
    assert!(manufacturing.contains("kind=stock"));

    publish_artifact("timber-frame-house-bom.txt", bom.as_bytes());
    publish_artifact("timber-frame-house-drawings.svg", drawings.as_bytes());
    publish_artifact(
        "timber-frame-house-manufacturing.txt",
        manufacturing.as_bytes(),
    );
}

/// Manufacturability is not structural sanity. The same house must also be
/// held up by gravity: every member either sits on the ground or rests on
/// something that does. This is the check the acceptance proof was missing —
/// it is what catches a ridge beam hanging in mid-air.
#[test]
fn the_timber_frame_house_must_stand_up_under_gravity() {
    let (shell, _transport, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let packages = HOUSE_MEMBERS
        .iter()
        .map(|name| {
            let definition_id = definition_id_of(&shell, name);
            let request = ExactFeatureChainRequest::from_snapshot(&snapshot, definition_id)
                .unwrap_or_else(|error| panic!("{name} must yield an exact request: {error}"));
            let package = worker
                .evaluate_rectangle(&request)
                .unwrap_or_else(|error| panic!("{name} must solve exactly: {error}"));
            Arc::new(ExactBodyPackage::from(package))
        })
        .collect::<Vec<_>>();
    let registry = ExactResultRegistry::accept(&snapshot, packages).unwrap();

    // Only the sill plates are founded on the ground. Everything else has to
    // earn its support through real contact with something already supported.
    let sill_definitions = [
        "Sill plate front",
        "Sill plate back",
        "Sill plate left",
        "Sill plate right",
    ]
    .map(|name| definition_id_of(&shell, name));
    let participants = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            let body = GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be an accepted general body: {error:?}",
                    occurrence.occurrence_name
                )
            });
            let grounded = occurrence.occurrence_name.starts_with("Sill plate")
                || sill_definitions.contains(&occurrence.definition_id);
            GravitySupportParticipant::new(body, "house", grounded)
        })
        .collect::<Vec<_>>();

    let input = GravitySupportInput::new(participants, [0.0, 0.0, -9.81]).unwrap();
    let validator = BuiltinGravitySupportValidator::new(tolerance);
    let policy = gravity_support_validation_policy();
    let bytes = gravity_support_input_bytes(&input);
    let invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        Vec::new(),
        &bytes,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation,
        policy: &policy,
        input: &input,
    });

    let unsupported = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "gravity.unsupported")
        .map(|diagnostic| {
            let evidence = diagnostic.evidence.clone();
            match snapshot.scene_query().into_iter().find(|occurrence| {
                evidence.starts_with(&format!("body=occurrence:{};", occurrence.occurrence_id.0))
            }) {
                Some(occurrence) => occurrence.occurrence_name,
                None => evidence,
            }
        })
        .collect::<Vec<_>>();
    assert!(
        unsupported.is_empty(),
        "the Assistant-built house has {} member(s) floating in mid-air: {:#?}",
        unsupported.len(),
        unsupported
    );
    assert_eq!(
        report.state,
        ValidationState::Passed,
        "{:#?}",
        report.diagnostics
    );
}

/// The window opening this scenario needs was blocked by two generality limits.
/// The first one is gone: a Pocket now consumes a Sketch profile authored by the
/// same generic program, so the Assistant can cut an opening into a part it just
/// created. The second still fails closed with a specific machine code and no
/// mutation, and this test pins that split.
#[test]
fn assistant_cuts_an_opening_but_cannot_yet_boolean_two_parts_it_created() {
    let requests = [
        "Sheath the front wall",
        "Cut a window opening into that sheathing",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new(requests.map(|request| {
        (
            request.to_owned(),
            AssistantChatResult {
                message: "Review the sheathing.".to_owned(),
                model_intent: None,
            },
        )
    })));
    let (entities, constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    build_step(
        &mut shell,
        &transport,
        requests[0],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [0.0, 0.0, 0.0],
                rotation: None,
            }],
        },
    );

    let sheathing = definition_id_of(&shell, "Front sheathing");
    let sheathing_pad = body_feature_id_of(&shell, "Front sheathing");

    // Lifted limit: a Pocket now accepts the Sketch profile the same program
    // authored, so the opening goes all the way through review to commit.
    let (window_entities, window_constraints) = rectangle(1_200.0, 1_400.0);
    let sketch_then_pocket = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreateSketch {
                definition_id: sheathing.0,
                name: "Window opening".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: window_entities,
                constraints: window_constraints,
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: sheathing.0,
                name: "Window pocket".to_owned(),
                feature: AssistantCadBodyFeature::Pocket {
                    target_feature_id: sheathing_pad.0,
                    // The sketch this same program just authored.
                    profile_feature_id: sheathing_pad.0 + 2,
                    depth_mm: SHEATHING_THICKNESS_MM,
                },
            },
        ],
    };
    build_step(&mut shell, &transport, requests[1], sketch_then_pocket);

    // The committed definition really owns the pocket, driven by that sketch and
    // targeting the pad, and the whole part stays an editable exact chain.
    let committed = shell.app().document_snapshot();
    let pocket_id = committed
        .definitions()
        .find(|definition| definition.id() == sheathing)
        .expect("the sheathing definition must survive the cut")
        .feature_ids()
        .iter()
        .copied()
        .find(|id| {
            matches!(
                committed.feature(*id).unwrap().kind(),
                FeatureKind::Pocket { .. }
            )
        })
        .expect("the sheathing must own the window pocket");
    let FeatureKind::Pocket {
        target, profile, ..
    } = committed.feature(pocket_id).unwrap().kind()
    else {
        unreachable!("the pocket kind was just matched")
    };
    assert_eq!(*target, sheathing_pad);
    assert_eq!(profile.0, sheathing_pad.0 + 2);
    ExactBRepGraph::from_snapshot(&committed, sheathing, pocket_id)
        .expect("the pocketed sheathing must compile into an exact graph");

    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    // CreatePart still opens a new definition, so its symbolically referenced
    // body is visible to the later operation but cannot bypass canonical
    // same-definition Boolean ownership.
    let second_part = AssistantCadEditProgram {
        operations: vec![
            timber("Window block", 1_200.0, 1_400.0, 100.0, [0.0; 3], None),
            AssistantCadEditOperation::AppendFeature {
                definition_id: sheathing.0,
                name: "Window cut".to_owned(),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: AssistantCadBooleanOperation::Cut,
                    target_feature_id: sheathing_pad.0.into(),
                    tool_feature_id: AssistantCadFeatureReference::ProgramOutput(
                        AssistantCadProgramFeatureReference {
                            operation_index: 0,
                            output: AssistantCadProgramFeatureOutput::BodyFeature,
                        },
                    ),
                },
            },
        ],
    };
    let rejection = shell
        .app()
        .plan_assistant_cad_edit_program(&second_part)
        .expect_err("a cross-definition Boolean must fail closed");
    assert_eq!(
        rejection.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    assert_eq!(rejection.operation, "append_feature");

    // Neither rejection may touch the document.
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert!(shell.app().assistant_proposal().is_none());
}

/// The generic Assistant can feed its own closed-profile and open-path Sketches
/// into the same reviewed exact Sweep path used by canonical fixtures.
#[test]
fn assistant_authored_sketches_feed_a_reviewed_exact_sweep() {
    let clear_request = "Clear the document for a sweep test";
    let profile_request = "Create a square sweep profile part";
    let path_request = "Create an open straight sweep path in that part";
    let sweep_request = "Sweep the authored square profile along the authored path";
    let chat_result = || AssistantChatResult {
        message: "Review the generic sweep step.".to_owned(),
        model_intent: None,
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (clear_request.to_owned(), chat_result()),
        (profile_request.to_owned(), chat_result()),
        (path_request.to_owned(), chat_result()),
        (sweep_request.to_owned(), chat_result()),
        (sweep_request.to_owned(), chat_result()),
    ]));
    let mut shell = Shell::with_assistant_transport(transport.clone());

    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        &transport,
        clear_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 0);

    let (entities, constraints) = rectangle(20.0, 20.0);
    build_step(
        &mut shell,
        &transport,
        profile_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Sweep profile".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Yz,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion { distance_mm: 5.0 },
                translation_mm: [0.0; 3],
                rotation: None,
            }],
        },
    );
    let definition_id = definition_id_of(&shell, "Sweep profile");

    build_step(
        &mut shell,
        &transport,
        path_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateSketch {
                definition_id: definition_id.0,
                name: "Sweep path".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![AssistantSketchEntity::Line {
                    id: 1,
                    start_mm: [0.0, 0.0],
                    end_mm: [200.0, 0.0],
                }],
                constraints: vec![AssistantSketchConstraint::Horizontal {
                    id: 1,
                    entity_id: 1,
                }],
            }],
        },
    );

    let authored = shell.app().document_snapshot();
    let profile_id = authored
        .features()
        .find(|feature| feature.name() == "Sweep profile sketch")
        .expect("the generic part must own its Assistant-authored profile sketch")
        .id();
    let path_id = authored
        .features()
        .find(|feature| feature.name() == "Sweep path")
        .expect("the generic CreateSketch operation must author the path")
        .id();
    assert!(matches!(
        authored.feature(profile_id).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));
    assert!(matches!(
        authored.feature(path_id).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));

    let sweep_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: definition_id.0,
            name: "Assistant sketch sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: profile_id.0,
                path_feature_id: path_id.0,
            },
        }],
    };
    build_step(&mut shell, &transport, sweep_request, sweep_program);

    let committed = shell.app().document_snapshot();
    let sweep_id = committed
        .features()
        .find(|feature| feature.name() == "Assistant sketch sweep")
        .expect("the reviewed Assistant step must create the Sweep")
        .id();
    assert!(matches!(
        committed.feature(sweep_id).unwrap().kind(),
        FeatureKind::Sweep { profile, path }
            if *profile == profile_id && *path == path_id
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, sweep_id).unwrap();
    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), definition_id, sweep_id).unwrap(),
        graph
    );
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V13);
    let mut downgraded = graph.clone();
    downgraded.schema = EXACT_BREP_GRAPH_SCHEMA_V12.to_owned();
    assert!(matches!(
        downgraded.to_bytes(),
        Err(ExactBRepGraphError::InvalidGraph)
    ));
    assert_eq!(graph.profiles.len(), 2);
    assert_eq!(graph.profiles[0].source_feature_id, profile_id.0);
    assert_eq!(graph.profiles[1].source_feature_id, path_id.0);
    let expected_bounds = [[0.0, 0.0, 0.0], [200.0, 20.0, 20.0]];
    assert_eq!(graph.producer_bounds_mm().unwrap(), Some(expected_bounds));
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!((package.volume_mm3 - 80_000.0).abs() <= 1.0e-6);
    assert_eq!(package.bounds_mm, expected_bounds);
}

/// Unlike fixture-backed geometry, this starts from an Assistant-authored body.
/// The headless hook publishes the worker result deterministically; this test
/// isolates the generic Assistant context/planner/commit path for a real fillet.
#[test]
fn assistant_authored_part_accepts_a_host_issued_topology_fillet() {
    let clear_request = "Clear the document for a fillet test";
    let part_request = "Create a small rectangular part";
    let fillet_request = "Fillet one current edge of that part by 2 mm";
    let chat_result = || AssistantChatResult {
        message: "Review the generic fillet step.".to_owned(),
        model_intent: None,
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (clear_request.to_owned(), chat_result()),
        (part_request.to_owned(), chat_result()),
        (fillet_request.to_owned(), chat_result()),
    ]));
    let mut shell = Shell::with_assistant_transport(transport.clone());

    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        &transport,
        clear_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );

    let (entities, constraints) = rectangle(40.0, 30.0);
    build_step(
        &mut shell,
        &transport,
        part_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Fillet block".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion { distance_mm: 20.0 },
                translation_mm: [0.0; 3],
                rotation: None,
            }],
        },
    );

    let definition_id = definition_id_of(&shell, "Fillet block");
    let body_id = body_feature_id_of(&shell, "Fillet block");
    let base_graph =
        ExactBRepGraph::from_snapshot(&shell.app().document_snapshot(), definition_id, body_id)
            .expect("the Assistant-authored block must compile into an exact graph");
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();
    let base_volume_mm3 = base_package.volume_mm3;
    let edge_reference = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference
                .producer_element_id
                .starts_with("generated-result/edge/")
        })
        .expect("the exact block must publish host-issued edge references")
        .lineage_digest
        .clone();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    assert!(
        shell.app().assistant_context()["topology_edge_references"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["reference_id"] == edge_reference)
    );

    build_step(
        &mut shell,
        &transport,
        fillet_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: definition_id.0,
                name: "Rounded edge".to_owned(),
                feature: AssistantCadBodyFeature::TopologyFillet {
                    target_feature_id: body_id.0,
                    edge_reference_ids: vec![edge_reference.clone()],
                    radius_mm: 2.0,
                },
            }],
        },
    );

    let committed = shell.app().document_snapshot();
    let fillet_id = committed
        .features()
        .find(|feature| feature.name() == "Rounded edge")
        .expect("the reviewed fillet must be canonical")
        .id();
    assert!(matches!(
        committed.feature(fillet_id).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish {
            target,
            edges,
            kind: EdgeFinishKind::Fillet,
            amount,
        } if *target == body_id
            && edges.len() == 1
            && edges[0].lineage_digest == edge_reference
            && amount.millimetres() == 2.0
    ));
    let fillet_graph = ExactBRepGraph::from_snapshot(&committed, definition_id, fillet_id)
        .expect("the Assistant fillet must remain an exact graph");
    let fillet_package = worker.evaluate_exact_brep_graph(&fillet_graph).unwrap();
    assert!(fillet_package.volume_mm3 > 0.0);
    assert!(
        fillet_package.volume_mm3 < base_volume_mm3,
        "a convex edge fillet must remove exact material from the block"
    );
    assert_eq!(fillet_package.topology_counts[4], 1);
    assert_eq!(transport.remaining_responses(), 0);
}
