mod harness;

use eframe::egui::{self, accesskit::Role};
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{
    ASSISTANT_REPAIR_PROGRAM_SCHEMA_V1, AppCommand, AssistantMessageRole, AssistantProvider,
    AssistantRepairOperation, AssistantRepairProgram, AssistantWorkspaceMode,
};
use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantApiDiagnostics, AssistantBalloonTextIntent,
    AssistantBeamNotchIntent, AssistantBottleFinishKind, AssistantBottleIntent, AssistantBoxIntent,
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadPartFeature, AssistantCadRotation, AssistantChatResult, AssistantDistribution,
    AssistantGableRoofIntent, AssistantKetchupBottleIntent, AssistantLinearArrayIntent,
    AssistantModelIntent, AssistantOrientedBeamIntent, AssistantParameterEditIntent,
    AssistantPrincipalPlane, AssistantProfileTranslationIntent, AssistantRotationIntent,
    AssistantSketchConstraint, AssistantSketchEntity, AssistantSketchPointKind,
    AssistantSketchPointRef, AssistantStaircaseIntent, AssistantSubtractionIntent,
    AssistantTeapotIntent, AssistantTranslationIntent, AssistantWorkplaneSpec,
};
use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, ClassificationCategoryId,
    ClassificationDimensionId, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, GroupId, NodeId, OccurrenceId, ProfileSegment, ProposalGoal, ProposalValue, TagId,
    Transform,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchSpec, WorkplaneSpec,
};
use ketchup_core::state_view::encode_semantic_state;
use ketchup_core::validation::VALIDATOR_ROLE_DIMENSION_V1;
use ketchup_interaction::{LocaleCatalog, Vec3};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn exact_worker_path() -> std::path::PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
}

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

fn assign_validator_roles(shell: &mut Shell, assignments: &[(&str, &str, &str)]) {
    let first_role = assignments.first().unwrap().2;
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, first_role)
    );
    let dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in assignments
        .iter()
        .map(|(_, _, role)| *role)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|role| *role != first_role)
    {
        assert!(
            shell
                .app_mut()
                .add_classification_category(dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    shell.settle();
    for (name, dimensions, role) in assignments {
        let row = shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", (*name).to_owned()),
                ("dimensions", (*dimensions).to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        );
        shell.click_row(&row);
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(dimension_id, Some(role_ids[*role]),)
        );
    }
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
    shell.settle();
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

fn write_assistant_boolean_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boolean inputs".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Target profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Target extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Tool profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[40.0, 0.0], [120.0, 0.0], [120.0, 60.0], [40.0, 60.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Boolean inputs".to_owned(),
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

fn write_assistant_sweep_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Sweep inputs".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Curved sweep profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [-2.0, -3.0],
                            end_mm: [2.0, -3.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [2.0, -3.0],
                            end_mm: [2.0, 3.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [2.0, 3.0],
                            end_mm: [-2.0, 3.0],
                            center_mm: [0.0, 3.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [-2.0, 3.0],
                            end_mm: [-2.0, -3.0],
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Oblique sweep path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [10.0, -5.0],
                        end_mm: [24.0, 17.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Sweep inputs".to_owned(),
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

fn write_assistant_rotation_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Arbitrary profile body".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Non-box profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [31.0, 4.0], [23.0, 29.0], [2.0, 18.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Non-box extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("17").unwrap(),
                },
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(2),
                name: "Rotated parent assembly".to_owned(),
                transform: Transform::from_matrix([
                    0.0, -1.0, 0.0, 100.0, 1.0, 0.0, 0.0, 50.0, 0.0, 0.0, 1.0, 10.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Arbitrary assembly".to_owned(),
                transform: Transform::identity(),
                parent: Some(GroupId(2)),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Grouped arbitrary body".to_owned(),
                transform: Transform::identity(),
                parent: Some(GroupId(1)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Root arbitrary body".to_owned(),
                transform: Transform::from_translation(60.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn assert_point_near(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!((actual[axis] - expected[axis]).abs() < 1.0e-9);
    }
}

fn rotate_point_about_axis(
    point: [f64; 3],
    pivot: [f64; 3],
    axis: [f64; 3],
    angle_degrees: f64,
) -> [f64; 3] {
    let axis_length = axis.iter().map(|value| value * value).sum::<f64>().sqrt();
    let unit = axis.map(|value| value / axis_length);
    let vector = [
        point[0] - pivot[0],
        point[1] - pivot[1],
        point[2] - pivot[2],
    ];
    let dot = unit[0] * vector[0] + unit[1] * vector[1] + unit[2] * vector[2];
    let cross = [
        unit[1] * vector[2] - unit[2] * vector[1],
        unit[2] * vector[0] - unit[0] * vector[2],
        unit[0] * vector[1] - unit[1] * vector[0],
    ];
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    [
        pivot[0] + vector[0] * cos + cross[0] * sin + unit[0] * dot * (1.0 - cos),
        pivot[1] + vector[1] * cos + cross[1] * sin + unit[1] * dot * (1.0 - cos),
        pivot[2] + vector[2] * cos + cross[2] * sin + unit[2] * dot * (1.0 - cos),
    ]
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
    panic!(
        "scripted assistant response did not reach accessible proposal review: {:?}",
        shell.app().assistant_messages()
    );
}

#[test]
fn assistant_rotates_arbitrary_occurrences_and_groups_around_arbitrary_world_axes() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory
        .path()
        .join("assistant-arbitrary-rotation.ketchup");
    write_assistant_rotation_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);

    let before_snapshot = shell.app().document_snapshot();
    let first_before = before_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    let first_before_world = before_snapshot
        .world_transform_for_occurrence(OccurrenceId(1))
        .unwrap();
    let group_before_world = before_snapshot
        .world_transform_for_group(GroupId(1))
        .unwrap();
    let second_before = before_snapshot
        .occurrence(OccurrenceId(2))
        .unwrap()
        .transform();
    let before_digest = shell.app().canonical_digest();
    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let first_pivot = [12.5, -3.0, 40.0];
    let first_axis = [1.0, 2.0, 3.0];

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: vec![
                AssistantRotationIntent {
                    occurrence_id: Some(1),
                    group_id: None,
                    pivot_mm: first_pivot,
                    axis: first_axis,
                    angle_degrees: 37.25,
                },
                AssistantRotationIntent {
                    occurrence_id: Some(2),
                    group_id: None,
                    pivot_mm: [-8.0, 11.0, 6.0],
                    axis: [-2.0, 5.0, 1.0],
                    angle_degrees: -61.5,
                },
            ],
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    ));
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    let rotated_snapshot = shell.app().document_snapshot();
    let first_rotated = rotated_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    let first_rotated_world = rotated_snapshot
        .world_transform_for_occurrence(OccurrenceId(1))
        .unwrap();
    let second_rotated = rotated_snapshot
        .occurrence(OccurrenceId(2))
        .unwrap()
        .transform();
    assert_ne!(first_rotated, first_before);
    assert_ne!(second_rotated, second_before);
    let first_local_point = [7.0, -4.0, 3.0];
    assert_point_near(
        transform_point(first_rotated_world, first_local_point),
        rotate_point_about_axis(
            transform_point(first_before_world, first_local_point),
            first_pivot,
            first_axis,
            37.25,
        ),
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform(),
        first_before
    );

    let group_pivot = [4.0, 7.0, -2.0];
    let group_axis = [3.0, -1.0, 2.0];
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: vec![AssistantRotationIntent {
                occurrence_id: None,
                group_id: Some(1),
                pivot_mm: group_pivot,
                axis: group_axis,
                angle_degrees: 22.75,
            }],
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    ));
    let group_snapshot = shell.app().document_snapshot();
    let group_rotated = group_snapshot.group(GroupId(1)).unwrap().transform();
    let group_rotated_world = group_snapshot
        .world_transform_for_group(GroupId(1))
        .unwrap();
    let group_local_point = [-2.0, 5.0, 9.0];
    assert_point_near(
        transform_point(group_rotated_world, group_local_point),
        rotate_point_about_axis(
            transform_point(group_before_world, group_local_point),
            group_pivot,
            group_axis,
            22.75,
        ),
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .group(GroupId(1))
            .unwrap()
            .transform(),
        group_rotated
    );
}

#[test]
fn scripted_assistant_rotation_reviews_cancel_stale_confirm_and_undo_through_accesskit() {
    let cancelled_request = "Rotate the first arbitrary body, but let me review it";
    let stale_request = "Prepare the first arbitrary body rotation before another edit";
    let occurrences_request = "Rotate both arbitrary bodies around their requested world axes";
    let group_request = "Rotate the arbitrary assembly around its requested world axis";
    let occurrence_rotations = vec![
        AssistantRotationIntent {
            occurrence_id: Some(1),
            group_id: None,
            pivot_mm: [12.5, -3.0, 40.0],
            axis: [1.0, 2.0, 3.0],
            angle_degrees: 37.25,
        },
        AssistantRotationIntent {
            occurrence_id: Some(2),
            group_id: None,
            pivot_mm: [-8.0, 11.0, 6.0],
            axis: [-2.0, 5.0, 1.0],
            angle_degrees: -61.5,
        },
    ];
    let result = |message: &str, rotations: Vec<AssistantRotationIntent>| AssistantChatResult {
        message: message.to_owned(),
        model_intent: Some(AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations,
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }),
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            cancelled_request.to_owned(),
            result(
                "Review the cancelled rotation.",
                vec![occurrence_rotations[0].clone()],
            ),
        ),
        (
            stale_request.to_owned(),
            result(
                "Review the stale rotation.",
                vec![occurrence_rotations[0].clone()],
            ),
        ),
        (
            occurrences_request.to_owned(),
            result(
                "Review both occurrence rotations.",
                occurrence_rotations.clone(),
            ),
        ),
        (
            group_request.to_owned(),
            result(
                "Review the group rotation.",
                vec![AssistantRotationIntent {
                    occurrence_id: None,
                    group_id: Some(1),
                    pivot_mm: [4.0, 7.0, -2.0],
                    axis: [3.0, -1.0, 2.0],
                    angle_degrees: 22.75,
                }],
            ),
        ),
    ]));
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("assistant-rotation-review.ketchup");
    write_assistant_rotation_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport.clone());
    shell.click_menu_command("menu-file", AppCommand::Open);
    let input = shell.catalog().text("assistant-input-hint");
    let cancel = shell.catalog().text("assistant-cancel");
    let confirm = shell.catalog().text("assistant-confirm");
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_snapshot = shell.app().document_snapshot();
    let first_before = baseline_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    let first_before_world = baseline_snapshot
        .world_transform_for_occurrence(OccurrenceId(1))
        .unwrap();
    let second_before = baseline_snapshot
        .occurrence(OccurrenceId(2))
        .unwrap()
        .transform();
    let second_before_world = baseline_snapshot
        .world_transform_for_occurrence(OccurrenceId(2))
        .unwrap();

    shell.focus_text_input(&input);
    shell.type_text(cancelled_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    shell.click_row(&cancel);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    shell.focus_text_input(&input);
    shell.type_text(stale_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-view", AppCommand::Hide);
    let intervening_revision = shell.app().document_revision();
    let intervening_digest = shell.app().canonical_digest();
    let intervening_undo = shell.app().undo_step_count();
    shell.settle();
    shell.click_row(&confirm);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), intervening_revision);
    assert_eq!(shell.app().canonical_digest(), intervening_digest);
    assert_eq!(shell.app().undo_step_count(), intervening_undo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform(),
        first_before
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    shell.focus_text_input(&input);
    shell.type_text(occurrences_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    shell.click_row(&confirm);
    assert!(shell.app().document_revision() > baseline_revision);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let rotated_snapshot = shell.app().document_snapshot();
    let first_rotated = rotated_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    let second_rotated = rotated_snapshot
        .occurrence(OccurrenceId(2))
        .unwrap()
        .transform();
    let first_rotated_world = rotated_snapshot
        .world_transform_for_occurrence(OccurrenceId(1))
        .unwrap();
    assert_ne!(first_rotated, first_before);
    assert_ne!(second_rotated, second_before);
    let second_rotated_world = rotated_snapshot
        .world_transform_for_occurrence(OccurrenceId(2))
        .unwrap();
    assert_point_near(
        transform_point(first_rotated_world, [7.0, -4.0, 3.0]),
        rotate_point_about_axis(
            transform_point(first_before_world, [7.0, -4.0, 3.0]),
            occurrence_rotations[0].pivot_mm,
            occurrence_rotations[0].axis,
            occurrence_rotations[0].angle_degrees,
        ),
    );
    assert_point_near(
        transform_point(second_rotated_world, [-3.0, 8.0, 1.5]),
        rotate_point_about_axis(
            transform_point(second_before_world, [-3.0, 8.0, 1.5]),
            occurrence_rotations[1].pivot_mm,
            occurrence_rotations[1].axis,
            occurrence_rotations[1].angle_degrees,
        ),
    );
    let rotated_digest = shell.app().canonical_digest();
    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), rotated_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);

    let before_group_snapshot = shell.app().document_snapshot();
    let group_before = before_group_snapshot.group(GroupId(1)).unwrap().transform();
    let group_before_world = before_group_snapshot
        .world_transform_for_group(GroupId(1))
        .unwrap();
    let before_group_revision = shell.app().document_revision();
    let before_group_digest = shell.app().canonical_digest();
    let before_group_undo = shell.app().undo_step_count();
    shell.focus_text_input(&input);
    shell.type_text(group_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), before_group_revision);
    assert_eq!(shell.app().canonical_digest(), before_group_digest);
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), before_group_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_group_undo + 1);
    let group_snapshot = shell.app().document_snapshot();
    let group_rotated = group_snapshot.group(GroupId(1)).unwrap().transform();
    assert_ne!(group_rotated, group_before);
    assert_point_near(
        transform_point(
            group_snapshot
                .world_transform_for_group(GroupId(1))
                .unwrap(),
            [-2.0, 5.0, 9.0],
        ),
        rotate_point_about_axis(
            transform_point(group_before_world, [-2.0, 5.0, 9.0]),
            [4.0, 7.0, -2.0],
            [3.0, -1.0, 2.0],
            22.75,
        ),
    );
    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    assert_eq!(shell.app().canonical_digest(), before_group_digest);
    assert_eq!(shell.app().undo_step_count(), before_group_undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let final_snapshot = shell.app().document_snapshot();
    assert_eq!(
        final_snapshot.group(GroupId(1)).unwrap().transform(),
        group_rotated
    );
    let final_digest = final_snapshot.canonical_digest();
    let round_trip_path = directory
        .path()
        .join("assistant-rotation-round-trip.ketchup");
    persistence::save_atomic(&round_trip_path, &final_snapshot).unwrap();
    let reopened_outcome = persistence::load_file(&round_trip_path).unwrap();
    assert!(reopened_outcome.is_editable());
    let reopened = reopened_outcome.snapshot();
    assert_eq!(reopened.canonical_digest(), final_digest);
    for occurrence_id in [OccurrenceId(1), OccurrenceId(2)] {
        assert_eq!(
            reopened.occurrence(occurrence_id).unwrap().transform(),
            final_snapshot
                .occurrence(occurrence_id)
                .unwrap()
                .transform()
        );
        assert_eq!(
            reopened
                .world_transform_for_occurrence(occurrence_id)
                .unwrap(),
            final_snapshot
                .world_transform_for_occurrence(occurrence_id)
                .unwrap()
        );
    }
    assert_eq!(
        reopened.group(GroupId(1)).unwrap().transform(),
        group_rotated
    );
    assert_eq!(
        reopened.world_transform_for_group(GroupId(1)).unwrap(),
        final_snapshot
            .world_transform_for_group(GroupId(1))
            .unwrap()
    );
    assert_eq!(transport.contexts().len(), 4);
    assert_eq!(transport.remaining_responses(), 0);
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
fn assistant_follow_up_move_right_is_not_misclassified_as_validation_repair() {
    let create_request = "Urob mäkčenie na to č v podobnej balónovej geometrii";
    let move_request =
        "Trošku ho posuň doprava, aby bol v strede nad tým céčkom. Teraz je trošku posunutý";
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            create_request.to_owned(),
            AssistantChatResult {
                message: "Pridávam samostatný mäkčeň.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: Vec::new(),
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                    bottles: Vec::new(),
                    balloon_texts: vec![AssistantBalloonTextIntent {
                        name: "Balloon caron".to_owned(),
                        text: "ˇ".to_owned(),
                        height_mm: 40.0,
                        depth_mm: 16.0,
                        stroke_width_mm: 8.0,
                        letter_spacing_mm: 0.0,
                        origin_mm: [40.0, 0.0, 100.0],
                    }],
                    gable_roofs: Vec::new(),
                    staircases: Vec::new(),
                    oriented_beams: Vec::new(),
                }),
            },
        ),
        (
            create_request.to_owned(),
            AssistantChatResult {
                message: "Používam editovateľný všeobecný feature program.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: vec![AssistantBoxIntent {
                        name: "Editable caron proxy".to_owned(),
                        size_mm: [20.0, 8.0, 12.0],
                        origin_mm: [40.0, 0.0, 100.0],
                        subtract_boxes: Vec::new(),
                    }],
                    translations: Vec::new(),
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                    bottles: Vec::new(),
                    balloon_texts: Vec::new(),
                    gable_roofs: Vec::new(),
                    staircases: Vec::new(),
                    oriented_beams: Vec::new(),
                }),
            },
        ),
        (
            move_request.to_owned(),
            AssistantChatResult {
                message: "Posúvam mäkčeň doprava.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![AssistantTranslationIntent {
                        occurrence_id: 1,
                        delta_mm: [5.0, 0.0, 0.0],
                    }],
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                    bottles: Vec::new(),
                    balloon_texts: Vec::new(),
                    gable_roofs: Vec::new(),
                    staircases: Vec::new(),
                    oriented_beams: Vec::new(),
                }),
            },
        ),
    ]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let input_label = shell.catalog().text("assistant-input-hint");
    let confirm = shell.catalog().text("assistant-confirm");

    shell.focus_text_input(&input_label);
    shell.type_text(create_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert!(shell.app().assistant_messages().iter().any(|message| {
        message
            .diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.code == "planning.editable_macro_required")
    }));
    assert_eq!(transport.remaining_responses(), 1);
    shell.click_row(&confirm);
    let before_move = shell
        .app()
        .document_snapshot()
        .occurrence(OccurrenceId(1))
        .expect("the first turn must create the standalone caron")
        .transform();

    shell.focus_text_input(&input_label);
    shell.type_text(move_request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(transport.remaining_responses(), 0);
    shell.click_row(&confirm);

    let after_move = shell
        .app()
        .document_snapshot()
        .occurrence(OccurrenceId(1))
        .expect("the second turn must retain the standalone caron")
        .transform();
    let before_matrix = before_move.matrix();
    let after_matrix = after_move.matrix();
    assert_eq!(after_matrix[3], before_matrix[3] + 5.0);
    assert_eq!(after_matrix[7], before_matrix[7]);
    assert_eq!(after_matrix[11], before_matrix[11]);
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
fn canonical_rejection_reaches_accesskit_without_generic_error_degradation() {
    let request = "Move missing occurrence 999";
    let rejected_result = || AssistantChatResult {
        message: "Moved the requested occurrence.".to_owned(),
        model_intent: Some(AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: vec![AssistantTranslationIntent {
                occurrence_id: 999,
                delta_mm: [10.0, 0.0, 0.0],
            }],
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }),
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (request.to_owned(), rejected_result()),
        (request.to_owned(), rejected_result()),
    ]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let before_undo_steps = shell.app().undo_step_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    for _ in 0..200 {
        shell.step();
        if shell
            .app()
            .assistant_messages()
            .iter()
            .filter(|message| message.diagnostic.is_some())
            .count()
            == 2
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    shell.settle();

    let diagnostic_messages = shell
        .app()
        .assistant_messages()
        .iter()
        .filter(|message| message.diagnostic.is_some())
        .collect::<Vec<_>>();
    assert_eq!(diagnostic_messages.len(), 2);
    for message in &diagnostic_messages {
        let diagnostic = message.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.code, "canonical.occurrence_not_found");
        assert_eq!(diagnostic.operation, "translate_occurrence");
        assert_eq!(diagnostic.target, "occurrence:999");
        assert_eq!(diagnostic.failed_invariant, "occurrence 999 does not exist");
        assert!(message.text.contains("canonical.occurrence_not_found"));
        assert!(message.text.contains("occurrence 999 does not exist"));
    }
    let visible_rejection = &diagnostic_messages.last().unwrap().text;
    assert!(shell.has_visible_label(visible_rejection));
    assert_ne!(
        visible_rejection,
        &shell.catalog().text("assistant-error-rejected-change")
    );
    assert_eq!(transport.contexts().len(), 2);
    assert_eq!(transport.contexts()[1]["assistant_replan"]["attempt"], 1);
    assert_eq!(
        transport.contexts()[1]["assistant_replan"]["max_attempts"],
        1
    );
    assert_eq!(transport.remaining_responses(), 0);
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo_steps);
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
fn scripted_cad_edit_program_reviews_selection_transform_copy_pattern_mirror_delete_and_one_undo() {
    let request = "Transform, copy, pattern, mirror, then delete my selection";
    let selector = AssistantCadEntitySelector::CurrentSelection {};
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::Transform {
                selector: selector.clone(),
                translation_mm: [10.0, 0.0, 0.0],
                rotation: None,
            },
            AssistantCadEditOperation::Copy {
                selector: selector.clone(),
                translation_mm: [0.0, 20.0, 0.0],
            },
            AssistantCadEditOperation::LinearPattern {
                selector: selector.clone(),
                instances: 3,
                step_mm: [30.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::Mirror {
                selector: selector.clone(),
                plane_origin_mm: [0.0, 0.0, 0.0],
                plane_normal: [1.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::Delete {
                selector,
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            },
        ],
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the bounded occurrence edit.".to_owned(),
            model_intent: None,
        },
    )]));
    transport.queue_cad_edit_program(request, program);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo_steps = shell.app().undo_step_count();
    let baseline_snapshot = shell.app().document_snapshot();
    let baseline_occurrences = baseline_snapshot.occurrences().count();
    let baseline_definition = baseline_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();

    let input_label = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input_label);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);

    assert_eq!(
        transport.contexts()[0]["selected_occurrence_ids"],
        serde_json::json!([1])
    );
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        baseline_occurrences
    );
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-title")));

    let proposal = shell.app().assistant_proposal().unwrap();
    assert_eq!(proposal.batch().commands().len(), 6);
    assert!(matches!(
        proposal.batch().commands()[0],
        CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(1),
            ..
        }
    ));
    assert_eq!(
        proposal
            .batch()
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::CreateOccurrence { id, .. } => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(4),
            OccurrenceId(5)
        ]
    );
    assert!(matches!(
        proposal.batch().commands()[5],
        CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1)
        }
    ));

    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
    let committed_digest = shell.app().canonical_digest();
    assert_ne!(committed_digest, baseline_digest);
    let committed = shell.app().document_snapshot();
    assert!(committed.occurrence(OccurrenceId(1)).is_none());
    assert_eq!(committed.occurrences().count(), baseline_occurrences + 3);
    for id in 2..=5 {
        assert_eq!(
            committed
                .occurrence(OccurrenceId(id))
                .unwrap()
                .definition_id(),
            baseline_definition
        );
    }

    let undo_change = shell.catalog().text("assistant-undo-change");
    shell.click_row(&undo_change);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
    let undone = shell.app().document_snapshot();
    assert!(undone.occurrence(OccurrenceId(1)).is_some());
    for id in 2..=5 {
        assert!(undone.occurrence(OccurrenceId(id)).is_none());
    }
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_create_part_program_round_trips_state_view_and_one_step_undo_redo() {
    let request = "Create one editable extruded part";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the editable part.".to_owned(),
            model_intent: None,
        },
    )]));
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Editable prism".to_owned(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 12.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 12.0,
            }],
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
            translation_mm: [5.0, 6.0, 7.0],
            rotation: Some(AssistantCadRotation {
                pivot_mm: [5.0, 6.0, 7.0],
                axis: [0.0, 1.0, 0.0],
                angle_degrees: 45.0,
            }),
        }],
    };
    transport.queue_cad_edit_program(request, program.clone());
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), baseline_occurrences + 1);
    let definition = committed
        .definitions()
        .find(|definition| definition.name() == "Editable prism")
        .unwrap();
    let kinds = definition
        .feature_ids()
        .iter()
        .map(|id| committed.feature(*id).unwrap().kind())
        .collect::<Vec<_>>();
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::Workplane(_)))
    );
    assert!(
        kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::Sketch(_)))
    );
    assert!(kinds.iter().any(|kind| matches!(kind, FeatureKind::Pad(_))));
    assert!(
        !kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::MeshBody(_)))
    );

    let committed_digest = committed.canonical_digest();
    let committed_state = encode_semantic_state(&committed);
    let committed_complete_view = committed_state.complete_v1();
    let committed_agent_view = committed_state.agent_v1();
    assert!(
        committed_complete_view.contains(&format!("source.canonical_digest={committed_digest}"))
    );
    for expected_kind in ["workplane", "sketch", "pad"] {
        assert!(committed_complete_view.contains(&format!(".kind={expected_kind}")));
        assert!(committed_agent_view.contains(&format!("kind:{expected_kind}")));
    }
    assert!(!committed_complete_view.contains(".kind=mesh_body"));
    assert!(!committed_agent_view.contains("kind:mesh_body"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-editable-prism.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    let reopened = outcome.snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_state = encode_semantic_state(&reopened);
    assert_eq!(reopened_state.complete_v1(), committed_complete_view);
    assert_eq!(reopened_state.agent_v1(), committed_agent_view);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        baseline_occurrences
    );

    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
    assert_eq!(
        encode_semantic_state(&shell.app().document_snapshot()).complete_v1(),
        committed_complete_view
    );
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_create_revolved_part_round_trips_state_view_and_one_step_undo_redo() {
    let request = "Create one editable revolved part";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the editable revolved part.".to_owned(),
            model_intent: None,
        },
    )]));
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Editable revolve".to_owned(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [10.0, 0.0],
                radius_mm: 2.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 2.0,
            }],
            feature: AssistantCadPartFeature::Revolve {
                axis_start_mm: [0.0, 0.0],
                axis_end_mm: [0.0, 1.0],
                angle_degrees: 275.0,
            },
            translation_mm: [10.0, 20.0, 30.0],
            rotation: None,
        }],
    };
    transport.queue_cad_edit_program(request, program);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), baseline_occurrences + 1);
    let definition = committed
        .definitions()
        .find(|definition| definition.name() == "Editable revolve")
        .unwrap();
    let revolve = definition
        .feature_ids()
        .iter()
        .find_map(|id| {
            let feature = committed.feature(*id).unwrap();
            match feature.kind() {
                FeatureKind::Revolve {
                    profile,
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                } => Some((*profile, *axis_start_mm, *axis_end_mm, *angle_degrees)),
                _ => None,
            }
        })
        .expect("Assistant must create a canonical revolve feature");
    assert!(matches!(
        committed.feature(revolve.0).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));
    assert_eq!(revolve.1, [0.0, 0.0]);
    assert_eq!(revolve.2, [0.0, 1.0]);
    assert_eq!(revolve.3, 275.0);
    assert!(definition.feature_ids().iter().all(|id| !matches!(
        committed.feature(*id).unwrap().kind(),
        FeatureKind::MeshBody(_)
    )));

    let committed_digest = committed.canonical_digest();
    let committed_state = encode_semantic_state(&committed);
    let committed_complete_view = committed_state.complete_v1();
    let committed_agent_view = committed_state.agent_v1();
    assert!(committed_complete_view.contains(".kind=revolve"));
    assert!(committed_agent_view.contains("kind:revolve"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-editable-revolve.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    let reopened = outcome.snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_state = encode_semantic_state(&reopened);
    assert_eq!(reopened_state.complete_v1(), committed_complete_view);
    assert_eq!(reopened_state.agent_v1(), committed_agent_view);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    assert_eq!(
        shell.app().document_snapshot().occurrences().count(),
        baseline_occurrences
    );

    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
    assert_eq!(
        encode_semantic_state(&shell.app().document_snapshot()).complete_v1(),
        committed_complete_view
    );
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_append_boolean_programs_are_exact_persistent_and_one_step() {
    let requests = [
        "Append an exact cut",
        "Append an exact union",
        "Append an exact intersect",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            requests[0].to_owned(),
            AssistantChatResult {
                message: "Review the exact cut.".to_owned(),
                model_intent: None,
            },
        ),
        (
            requests[1].to_owned(),
            AssistantChatResult {
                message: "Review the exact union.".to_owned(),
                model_intent: None,
            },
        ),
        (
            requests[2].to_owned(),
            AssistantChatResult {
                message: "Review the exact intersect.".to_owned(),
                model_intent: None,
            },
        ),
    ]));
    for (request, operation) in requests.iter().zip([
        AssistantCadBooleanOperation::Cut,
        AssistantCadBooleanOperation::Union,
        AssistantCadBooleanOperation::Intersect,
    ]) {
        transport.queue_cad_edit_program(
            *request,
            AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::AppendFeature {
                    definition_id: 1,
                    name: format!("Assistant {operation:?}"),
                    feature: AssistantCadBodyFeature::Boolean {
                        operation,
                        target_feature_id: 2,
                        tool_feature_id: 4,
                    },
                }],
            },
        );
    }

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-boolean-inputs.ketchup");
    write_assistant_boolean_fixture(&fixture_path);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();
    let mut digest_before_last = String::new();

    for (index, ((request, assistant_operation), canonical_operation)) in requests
        .iter()
        .zip([
            AssistantCadBooleanOperation::Cut,
            AssistantCadBooleanOperation::Union,
            AssistantCadBooleanOperation::Intersect,
        ])
        .zip([
            BooleanOperation::Cut,
            BooleanOperation::Union,
            BooleanOperation::Intersect,
        ])
        .enumerate()
    {
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo = shell.app().undo_step_count();
        if index == 2 {
            digest_before_last.clone_from(&baseline_digest);
        }
        let input = shell.catalog().text("assistant-input-hint");
        shell.focus_text_input(&input);
        shell.type_text(request);
        shell.press_key(egui::Key::Enter);
        wait_for_assistant_proposal(&mut shell);
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo);

        shell.click_row(&shell.catalog().text("assistant-confirm"));
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
        let committed = shell.app().document_snapshot();
        let feature_id = FeatureId(5 + index as u64);
        assert!(matches!(
            committed.feature(feature_id).unwrap().kind(),
            FeatureKind::Boolean {
                operation,
                target: FeatureId(2),
                tool: FeatureId(4),
            } if *operation == canonical_operation
        ));
        let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), feature_id).unwrap();
        assert_eq!(graph.producer_feature_id, feature_id.0);
        assert!(graph.nodes.iter().any(|node| matches!(
            &node.operation,
            ExactBRepOperation::Boolean { operation, .. }
                if *operation == canonical_operation.into()
        )));
        assert_eq!(committed.occurrences().count(), baseline_occurrences);
        assert_eq!(
            assistant_operation,
            match canonical_operation {
                BooleanOperation::Cut => AssistantCadBooleanOperation::Cut,
                BooleanOperation::Union => AssistantCadBooleanOperation::Union,
                BooleanOperation::Intersect => AssistantCadBooleanOperation::Intersect,
                BooleanOperation::Split => unreachable!(),
            }
        );
    }

    let committed = shell.app().document_snapshot();
    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-boolean-results.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    for feature_id in [FeatureId(5), FeatureId(6), FeatureId(7)] {
        assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), feature_id).is_ok());
    }

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), digest_before_last);
    assert!(
        shell
            .app()
            .document_snapshot()
            .feature(FeatureId(7))
            .is_none()
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert!(
        shell
            .app()
            .document_snapshot()
            .feature(FeatureId(7))
            .is_some()
    );
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_append_pocket_is_exact_persistent_and_one_step() {
    let request = "Pocket the existing extrusion";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the exact pocket.".to_owned(),
            model_intent: None,
        },
    )]));
    transport.queue_cad_edit_program(
        request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: 1,
                name: "Assistant pocket".to_owned(),
                feature: AssistantCadBodyFeature::Pocket {
                    target_feature_id: 2,
                    profile_feature_id: 3,
                    depth_mm: 8.0,
                },
            }],
        },
    );

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-pocket-inputs.ketchup");
    write_assistant_boolean_fixture(&fixture_path);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(5)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(2),
            profile: FeatureId(3),
            depth,
        } if depth.millimetres() == 8.0
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(5)).unwrap();
    assert_eq!(graph.producer_feature_id, 5);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(&node.operation, ExactBRepOperation::ProfileCut { .. }))
    );
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-pocket-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(5)).is_ok());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_append_sweep_is_exact_persistent_and_one_step() {
    let request = "Sweep the existing profile along the existing path";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the exact sweep.".to_owned(),
            model_intent: None,
        },
    )]));
    transport.queue_cad_edit_program(
        request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: 1,
                name: "Assistant sweep".to_owned(),
                feature: AssistantCadBodyFeature::Sweep {
                    profile_feature_id: 1,
                    path_feature_id: 2,
                },
            }],
        },
    );

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-sweep-inputs.ketchup");
    write_assistant_sweep_fixture(&fixture_path);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Sweep {
            profile: FeatureId(1),
            path: FeatureId(2),
        }
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(graph.producer_feature_id, 3);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(&node.operation, ExactBRepOperation::Sweep { .. }))
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.volume_mm3 > 0.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-sweep-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(3)).is_ok());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn named_assistant_generators_are_editable_or_fail_closed_with_bounded_macro_inputs() {
    let empty_intent = || AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    let mut shell = Shell::new();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();

    let mut vessel = empty_intent();
    vessel.bottles.push(AssistantBottleIntent {
        name: "Editable vessel".to_owned(),
        body_radius_mm: 30.0,
        body_height_mm: 110.0,
        shoulder_rise_mm: 20.0,
        neck_radius_mm: 12.0,
        neck_height_mm: 25.0,
        wall_thickness_mm: 2.0,
        finish_kind: AssistantBottleFinishKind::Fillet,
        finish_amount_mm: 2.0,
        origin_mm: [0.0, 0.0, 0.0],
        teapot: None,
        ketchup_bottle: None,
    });
    let mut roof = empty_intent();
    roof.gable_roofs.push(AssistantGableRoofIntent {
        name: "Editable roof".to_owned(),
        length_mm: 600.0,
        span_mm: 400.0,
        rise_mm: 120.0,
        thickness_mm: 20.0,
        origin_mm: [100.0, 0.0, 0.0],
    });
    let mut stairs = empty_intent();
    stairs.staircases.push(AssistantStaircaseIntent {
        name: "Editable stairs".to_owned(),
        run_mm: 3_000.0,
        width_mm: 800.0,
        rise_mm: 3_000.0,
        step_count: 15,
        origin_mm: [0.0, 2_200.0, 0.0],
    });
    let mut beam = empty_intent();
    beam.oriented_beams.push(AssistantOrientedBeamIntent {
        name: "Editable beam".to_owned(),
        start_mm: [0.0, 0.0, 300.0],
        end_mm: [500.0, 100.0, 400.0],
        up_hint: [0.0, 0.0, 1.0],
        width_mm: 40.0,
        depth_mm: 60.0,
        bottom_notches: Vec::new(),
    });
    #[cfg(not(feature = "named-product-fixtures"))]
    {
        assert!(!shell.app_mut().prepare_assistant_model_intent(vessel));
        assert!(shell.app().assistant_proposal().is_none());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo);
    }
    #[cfg(feature = "named-product-fixtures")]
    let editable_generators = [
        (vessel, "Editable vessel"),
        (roof, "Editable roof"),
        (stairs, "Editable stairs"),
        (beam, "Editable beam"),
    ];
    #[cfg(not(feature = "named-product-fixtures"))]
    let editable_generators = [
        (roof, "Editable roof"),
        (stairs, "Editable stairs"),
        (beam, "Editable beam"),
    ];
    for (editable, name) in editable_generators {
        assert!(
            apply_reviewed_model_intent(&mut shell, editable),
            "{name} must produce a reviewed editable macro"
        );
        let editable_snapshot = shell.app().document_snapshot();
        let definition = editable_snapshot
            .definitions()
            .find(|definition| definition.name() == name)
            .unwrap();
        assert!(definition.feature_ids().iter().all(|feature_id| {
            !matches!(
                editable_snapshot.feature(*feature_id).unwrap().kind(),
                FeatureKind::MeshBody(_)
            )
        }));
        assert!(shell.app_mut().undo());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo);
    }

    let mut teapot = empty_intent();
    teapot.bottles.push(AssistantBottleIntent {
        name: "Rejected teapot".to_owned(),
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
        ketchup_bottle: None,
    });
    let mut squeeze_bottle = empty_intent();
    squeeze_bottle.replace_scene = true;
    squeeze_bottle.bottles.push(AssistantBottleIntent {
        name: "Rejected squeeze bottle".to_owned(),
        body_radius_mm: 38.0,
        body_height_mm: 145.0,
        shoulder_rise_mm: 28.0,
        neck_radius_mm: 15.0,
        neck_height_mm: 18.0,
        wall_thickness_mm: 2.0,
        finish_kind: AssistantBottleFinishKind::Fillet,
        finish_amount_mm: 2.0,
        origin_mm: [0.0, 0.0, 0.0],
        teapot: None,
        ketchup_bottle: Some(AssistantKetchupBottleIntent {
            body_depth_ratio: 0.68,
            cap_radius_mm: 19.5,
            cap_height_mm: 24.0,
            label_width_mm: 58.0,
            label_height_mm: 72.0,
            label_relief_mm: 2.5,
            grip_rib_count: 20,
        }),
    });
    let mut balloon_text = empty_intent();
    balloon_text.balloon_texts.push(AssistantBalloonTextIntent {
        name: "Rejected balloon text".to_owned(),
        text: "ABC".to_owned(),
        height_mm: 40.0,
        depth_mm: 16.0,
        stroke_width_mm: 8.0,
        letter_spacing_mm: 4.0,
        origin_mm: [0.0, 0.0, 0.0],
    });
    for rejected in [teapot, squeeze_bottle, balloon_text] {
        assert!(!shell.app_mut().prepare_assistant_model_intent(rejected));
        assert!(shell.app().assistant_proposal().is_none());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo);
        assert!(
            shell
                .app()
                .document_snapshot()
                .features()
                .all(|feature| !matches!(feature.kind(), FeatureKind::MeshBody(_)))
        );
    }

    let invalid_reference = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Invalid offset part".to_owned(),
            workplane: AssistantWorkplaneSpec::Offset {
                base_feature_id: 999,
                distance_mm: 10.0,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            }],
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 10.0 },
            translation_mm: [0.0; 3],
            rotation: None,
        }],
    };
    let invalid_reference = shell
        .app()
        .plan_assistant_cad_edit_program(&invalid_reference)
        .expect_err("missing workplane reference must fail closed");
    assert_eq!(
        invalid_reference.code,
        "planning.workplane_base_unavailable"
    );

    let resource_overflow = AssistantCadEditProgram {
        operations: (0..65)
            .map(|_| AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::CurrentSelection {},
                dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
            })
            .collect(),
    };
    let resource_overflow = shell
        .app()
        .plan_assistant_cad_edit_program(&resource_overflow)
        .expect_err("operation budget overflow must fail closed");
    assert_eq!(resource_overflow.code, "intent.cad_edit_program_invalid");
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert!(shell.app().assistant_proposal().is_none());
}

#[test]
fn scripted_sketch_program_reviews_creates_and_edits_workplanes_entities_and_constraints() {
    let create_principal = "Create a reviewed mixed sketch on the XY workplane";
    let create_offset = "Create a reviewed circle sketch on an offset workplane";
    let edit_dimensions = "Edit the offset workplane and circle radius together";
    let chat_result = |message: &str| AssistantChatResult {
        message: message.to_owned(),
        model_intent: None,
    };
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            create_principal.to_owned(),
            chat_result("Review the principal workplane and mixed sketch."),
        ),
        (
            create_offset.to_owned(),
            chat_result("Review the offset workplane and circle sketch."),
        ),
        (
            edit_dimensions.to_owned(),
            chat_result("Review both dimension edits."),
        ),
    ]));
    let point = |entity_id, point| AssistantSketchPointRef { entity_id, point };
    transport.queue_cad_edit_program(
        create_principal,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateSketch {
                definition_id: 1,
                name: "Mixed boundary sketch".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![
                    AssistantSketchEntity::Line {
                        id: 1,
                        start_mm: [-20.0, 0.0],
                        end_mm: [20.0, 0.0],
                    },
                    AssistantSketchEntity::Arc {
                        id: 2,
                        start_mm: [20.0, 0.0],
                        end_mm: [-20.0, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                    AssistantSketchEntity::Circle {
                        id: 3,
                        center_mm: [50.0, 0.0],
                        radius_mm: 5.0,
                    },
                ],
                constraints: vec![
                    AssistantSketchConstraint::Horizontal {
                        id: 1,
                        entity_id: 1,
                    },
                    AssistantSketchConstraint::Radius {
                        id: 2,
                        entity_id: 3,
                        value_mm: 5.0,
                    },
                    AssistantSketchConstraint::FixedPoint {
                        id: 3,
                        point: point(1, AssistantSketchPointKind::Start),
                        position_mm: [-20.0, 0.0],
                    },
                ],
            }],
        },
    );
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let input = shell.catalog().text("assistant-input-hint");
    let confirm = shell.catalog().text("assistant-confirm");

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let before_undo = shell.app().undo_step_count();
    shell.focus_text_input(&input);
    shell.type_text(create_principal);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-review-title")));
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);

    let principal_snapshot = shell.app().document_snapshot();
    let principal_workplane_id = principal_snapshot
        .features()
        .find(|feature| feature.name() == "Mixed boundary sketch workplane")
        .unwrap()
        .id();
    let mixed_sketch_id = principal_snapshot
        .features()
        .find(|feature| feature.name() == "Mixed boundary sketch")
        .unwrap()
        .id();
    let FeatureKind::Sketch(mixed) = principal_snapshot.feature(mixed_sketch_id).unwrap().kind()
    else {
        panic!("expected reviewed canonical mixed sketch")
    };
    assert_eq!(mixed.entities.len(), 3);
    assert_eq!(mixed.constraints.len(), 3);
    assert_eq!(mixed.solved_regions().unwrap().len(), 2);

    transport.queue_cad_edit_program(
        create_offset,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateSketch {
                definition_id: 1,
                name: "Offset circle sketch".to_owned(),
                workplane: AssistantWorkplaneSpec::Offset {
                    base_feature_id: principal_workplane_id.0,
                    distance_mm: 12.0,
                },
                entities: vec![AssistantSketchEntity::Circle {
                    id: 1,
                    center_mm: [8.0, 9.0],
                    radius_mm: 6.0,
                }],
                constraints: vec![AssistantSketchConstraint::Radius {
                    id: 1,
                    entity_id: 1,
                    value_mm: 6.0,
                }],
            }],
        },
    );
    let before_offset_revision = shell.app().document_revision();
    let before_offset_digest = shell.app().canonical_digest();
    shell.focus_text_input(&input);
    shell.type_text(create_offset);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), before_offset_revision);
    assert_eq!(shell.app().canonical_digest(), before_offset_digest);
    shell.click_row(&confirm);

    let offset_snapshot = shell.app().document_snapshot();
    let offset_workplane_id = offset_snapshot
        .features()
        .find(|feature| feature.name() == "Offset circle sketch workplane")
        .unwrap()
        .id();
    let offset_sketch_id = offset_snapshot
        .features()
        .find(|feature| feature.name() == "Offset circle sketch")
        .unwrap()
        .id();
    transport.queue_cad_edit_program(
        edit_dimensions,
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::SetDimension {
                    feature_id: offset_workplane_id.0,
                    constraint_id: None,
                    value_mm: 18.0,
                },
                AssistantCadEditOperation::SetDimension {
                    feature_id: offset_sketch_id.0,
                    constraint_id: Some(1),
                    value_mm: 9.0,
                },
            ],
        },
    );
    let before_edit_revision = shell.app().document_revision();
    let before_edit_digest = shell.app().canonical_digest();
    let before_edit_undo = shell.app().undo_step_count();
    shell.focus_text_input(&input);
    shell.type_text(edit_dimensions);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), before_edit_revision);
    assert_eq!(shell.app().canonical_digest(), before_edit_digest);
    shell.click_row(&confirm);

    assert_eq!(shell.app().document_revision(), before_edit_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_edit_undo + 1);
    let edited = shell.app().document_snapshot();
    let FeatureKind::Workplane(workplane) = edited.feature(offset_workplane_id).unwrap().kind()
    else {
        panic!("expected reviewed offset workplane")
    };
    assert!(matches!(
        &workplane.support,
        ketchup_core::sketch::WorkplaneSupport::Offset { distance, .. }
            if distance.millimetres() == 18.0
    ));
    let FeatureKind::Sketch(sketch) = edited.feature(offset_sketch_id).unwrap().kind() else {
        panic!("expected reviewed offset sketch")
    };
    assert!(matches!(
        &sketch.constraints[0].kind,
        SketchConstraintKind::Radius { value, .. } if value.millimetres() == 9.0
    ));
    assert!(matches!(
        &sketch.solve_geometry().unwrap().entities[0],
        SketchEntity::Circle { radius_mm, .. } if *radius_mm == 9.0
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_edit_digest);
    assert_eq!(transport.remaining_responses(), 0);
}

#[test]
fn scripted_sketch_program_refuses_stale_preview_and_invalid_constraint_without_mutation() {
    let stale_request = "Prepare a reviewed sketch that will become stale";
    let stale_transport = Arc::new(ScriptedAssistantTransport::new([(
        stale_request.to_owned(),
        AssistantChatResult {
            message: "Review the sketch before applying it.".to_owned(),
            model_intent: None,
        },
    )]));
    stale_transport.queue_cad_edit_program(
        stale_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateSketch {
                definition_id: 1,
                name: "Stale sketch".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![AssistantSketchEntity::Circle {
                    id: 1,
                    center_mm: [0.0, 0.0],
                    radius_mm: 4.0,
                }],
                constraints: vec![AssistantSketchConstraint::Radius {
                    id: 1,
                    entity_id: 1,
                    value_mm: 4.0,
                }],
            }],
        },
    );
    let mut stale_shell = Shell::with_assistant_transport(stale_transport.clone());
    let input = stale_shell.catalog().text("assistant-input-hint");
    stale_shell.focus_text_input(&input);
    stale_shell.type_text(stale_request);
    stale_shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut stale_shell);
    assert!(stale_shell.app().assistant_proposal().is_some());

    stale_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    stale_shell.click_menu_command("menu-view", AppCommand::Hide);
    let intervening_revision = stale_shell.app().document_revision();
    let intervening_digest = stale_shell.app().canonical_digest();
    let intervening_undo = stale_shell.app().undo_step_count();
    stale_shell.settle();
    stale_shell.click_row(&stale_shell.catalog().text("assistant-confirm"));

    assert!(stale_shell.app().assistant_proposal().is_none());
    assert_eq!(stale_shell.app().document_revision(), intervening_revision);
    assert_eq!(stale_shell.app().canonical_digest(), intervening_digest);
    assert_eq!(stale_shell.app().undo_step_count(), intervening_undo);
    assert!(
        stale_shell
            .app()
            .document_snapshot()
            .features()
            .all(|feature| feature.name() != "Stale sketch")
    );
    assert_eq!(stale_transport.remaining_responses(), 0);

    let invalid_request = "Create a sketch with an invalid constraint reference";
    let rejected_result = || AssistantChatResult {
        message: "Review the constrained sketch.".to_owned(),
        model_intent: None,
    };
    let invalid_transport = Arc::new(ScriptedAssistantTransport::new([
        (invalid_request.to_owned(), rejected_result()),
        (invalid_request.to_owned(), rejected_result()),
    ]));
    let invalid_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1,
            name: "Rejected sketch".to_owned(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 4.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 999,
                value_mm: 4.0,
            }],
        }],
    };
    invalid_transport.queue_cad_edit_program(invalid_request, invalid_program.clone());
    invalid_transport.queue_cad_edit_program(invalid_request, invalid_program);
    let mut invalid_shell = Shell::with_assistant_transport(invalid_transport.clone());
    let before_revision = invalid_shell.app().document_revision();
    let before_digest = invalid_shell.app().canonical_digest();
    let before_undo = invalid_shell.app().undo_step_count();
    let input = invalid_shell.catalog().text("assistant-input-hint");
    invalid_shell.focus_text_input(&input);
    invalid_shell.type_text(invalid_request);
    invalid_shell.press_key(egui::Key::Enter);
    for _ in 0..200 {
        invalid_shell.step();
        if invalid_shell
            .app()
            .assistant_messages()
            .iter()
            .filter(|message| message.diagnostic.is_some())
            .count()
            == 2
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    invalid_shell.settle();

    assert_eq!(
        invalid_shell
            .app()
            .assistant_messages()
            .iter()
            .filter(|message| message.diagnostic.is_some())
            .count(),
        2
    );
    assert!(invalid_shell.app().assistant_proposal().is_none());
    assert_eq!(invalid_shell.app().document_revision(), before_revision);
    assert_eq!(invalid_shell.app().canonical_digest(), before_digest);
    assert_eq!(invalid_shell.app().undo_step_count(), before_undo);
    assert!(
        invalid_shell
            .app()
            .document_snapshot()
            .features()
            .all(|feature| feature.name() != "Rejected sketch")
    );
    assert_eq!(invalid_transport.remaining_responses(), 0);
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn verbal_bottle_goal_with_dimension_constraint_completes_verified_one_undo_cycle() {
    let request = "Create an editable ketchup bottle with a 30 mm body radius";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the editable bottle.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: Vec::new(),
                rotations: Vec::new(),
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
                    ketchup_bottle: None,
                }],
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
            }),
        },
    )]));
    let mut shell = Shell::with_assistant_transport(transport.clone());
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_undo_steps = shell.app().undo_step_count();
    let initial_features = shell.app().feature_count();
    let initial_occurrences = shell.app().occurrence_count();

    shell.focus_text_input(&shell.catalog().text("assistant-input-hint"));
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(&mut shell);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
    assert_eq!(shell.app().feature_count(), initial_features + 5);
    assert_eq!(shell.app().occurrence_count(), initial_occurrences + 1);
    let snapshot = shell.app().document_snapshot();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Verbally created bottle occurrence")
        .expect("confirmed proposal creates the requested bottle");
    assert!(snapshot.features().any(|feature| {
        feature.definition_id() == occurrence.definition_id()
            && matches!(
                feature.kind(),
                FeatureKind::BottleProfileControl { body_radius, .. }
                    if body_radius.millimetres() == 30.0
            )
    }));
    let verification = shell
        .app()
        .assistant_verification()
        .expect("confirmed proposal returns host verification")
        .clone();
    assert_eq!(verification.revision_id, initial_revision + 1);
    assert_eq!(
        verification.canonical_digest,
        shell.app().canonical_digest()
    );
    assert!(verification.verified_write_count > 0);
    assert!(!verification.command_digest.is_empty());
    assert!(!verification.result_digest.is_empty());
    shell.settle();
    assert!(shell.has_visible_label(&shell.catalog().text("assistant-result-title")));
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assistant-verification",
        &BTreeMap::from([
            ("revision", verification.revision_id.to_string()),
            ("writes", verification.verified_write_count.to_string()),
        ]),
    )));

    shell.click_row(&shell.catalog().text("assistant-undo-change"));
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps);
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
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
                rotations: Vec::new(),
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
                balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
        }
    ));
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(FeatureId(13))
            .unwrap()
            .kind(),
        FeatureKind::Pad(spec) if spec.extent.blind_distance().unwrap().millimetres() == 7.5
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
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
                rotations: Vec::new(),
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
                balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
                    name: "Opaque alpha".to_owned(),
                    size_mm: [100.0, 100.0, 10.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Floating decoy beta".to_owned(),
                    size_mm: [80.0, 80.0, 10.0],
                    origin_mm: [10.0, 10.0, 10.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque gamma".to_owned(),
                    size_mm: [60.0, 60.0, 10.0],
                    origin_mm: [20.0, 20.0, 20.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Supported decoy delta".to_owned(),
                    size_mm: [20.0, 20.0, 20.0],
                    origin_mm: [200.0, 0.0, 50.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Opaque alpha",
                "100 × 100 × 10",
                "physics.gravity.ground:stack",
            ),
            (
                "Floating decoy beta",
                "80 × 80 × 10",
                "physics.gravity.body:stack",
            ),
            ("Opaque gamma", "60 × 60 × 10", "physics.gravity.body:stack"),
            (
                "Supported decoy delta",
                "20 × 20 × 20",
                "physics.gravity.body:isolated",
            ),
        ],
    );
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
        ],
    );
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
    assert_eq!(gravity["issues"][0]["name"], "Supported decoy delta");
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    shell.settle();
    let still_unseeded = shell.app().assistant_context();
    assert_eq!(
        still_unseeded["validation"]["gravity_support"]["state"],
        "failed"
    );
    assert_eq!(
        still_unseeded["validation"]["gravity_support"]["unsupported_count"],
        1
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
                    name: "Opaque panel A".to_owned(),
                    size_mm: [1_000.0, 300.0, 12.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Shelf decoy B".to_owned(),
                    size_mm: [300.0, 400.0, 1_800.0],
                    origin_mm: [1_500.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque case C".to_owned(),
                    size_mm: [800.0, 500.0, 600.0],
                    origin_mm: [2_500.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Tall cabinet unassigned decoy".to_owned(),
                    size_mm: [100.0, 100.0, 2_000.0],
                    origin_mm: [4_000.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    shell.settle();
    let (shelf_occurrence_id, tall_case_occurrence_id, low_case_occurrence_id) = {
        let snapshot = shell.app().document_snapshot();
        let occurrence_id = |name: &str| {
            snapshot
                .occurrences()
                .find(|occurrence| occurrence.name() == name)
                .unwrap()
                .id()
        };
        (
            occurrence_id("Opaque panel A"),
            occurrence_id("Shelf decoy B"),
            occurrence_id("Opaque case C"),
        )
    };

    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, "furniture.shelf.xy",)
    );
    let role_dimension_id: ClassificationDimensionId = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    assert!(
        shell
            .app_mut()
            .add_classification_category(role_dimension_id, "furniture.case.z")
    );
    let (shelf_role_id, case_role_id): (ClassificationCategoryId, ClassificationCategoryId) = {
        let snapshot = shell.app().document_snapshot();
        let dimension = snapshot
            .classification_dimension(role_dimension_id)
            .unwrap();
        (
            dimension
                .categories()
                .find(|category| category.name() == "furniture.shelf.xy")
                .unwrap()
                .id(),
            dimension
                .categories()
                .find(|category| category.name() == "furniture.case.z")
                .unwrap()
                .id(),
        )
    };
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    shell.settle();
    for (occurrence_id, role_id, name, dimensions) in [
        (
            shelf_occurrence_id,
            shelf_role_id,
            "Opaque panel A",
            "1000 × 300 × 12",
        ),
        (
            tall_case_occurrence_id,
            case_role_id,
            "Shelf decoy B",
            "300 × 400 × 1800",
        ),
        (
            low_case_occurrence_id,
            case_role_id,
            "Opaque case C",
            "800 × 500 × 600",
        ),
    ] {
        let row = shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", name.to_owned()),
                ("dimensions", dimensions.to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        );
        shell.click_row(&row);
        assert!(shell.app().occurrence_is_selected(occurrence_id));
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_id))
        );
    }
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
    shell.settle();

    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: vec![
                AssistantRotationIntent {
                    occurrence_id: Some(shelf_occurrence_id.0),
                    group_id: None,
                    pivot_mm: [0.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    angle_degrees: 45.0,
                },
                AssistantRotationIntent {
                    occurrence_id: Some(tall_case_occurrence_id.0),
                    group_id: None,
                    pivot_mm: [1_500.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    angle_degrees: 45.0,
                },
            ],
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
    assert_eq!(shelf["issues"][0]["name"], "Opaque panel A");
    assert_eq!(shelf["issues"][0]["role"], "furniture.shelf.xy");
    assert_eq!(
        shelf["issues"][0]["role_source"],
        "canonical_classification"
    );
    assert_eq!(shelf["issues"][0]["span_mm"], 1_000.0);
    assert_eq!(shelf["issues"][0]["depth_mm"], 300.0);
    assert_eq!(shelf["issues"][0]["thickness_mm"], 12.0);
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
    assert_eq!(tipping["issues"][0]["name"], "Shelf decoy B");
    assert_eq!(tipping["issues"][0]["role"], "furniture.case.z");
    assert_eq!(
        tipping["issues"][0]["role_source"],
        "canonical_classification"
    );
    assert_eq!(tipping["issues"][0]["base_depth_mm"], 300.0);
    assert_eq!(tipping["issues"][0]["height_mm"], 1_800.0);
    assert_eq!(tipping["limit"]["minimum_tip_angle_degrees"], 15.0);

    let anchoring = &validation["anchoring"];
    assert_eq!(anchoring["state"], "failed");
    assert_eq!(anchoring["applicable_count"], 2);
    assert_eq!(anchoring["required_count"], 1);
    assert_eq!(anchoring["issues"][0]["code"], "furniture.anchor_required");
    assert_eq!(anchoring["issues"][0]["name"], "Shelf decoy B");
    assert_eq!(anchoring["issues"][0]["role"], "furniture.case.z");
    assert_eq!(anchoring["issues"][0]["base_depth_mm"], 300.0);
    assert_eq!(anchoring["issues"][0]["height_mm"], 1_800.0);
    assert_eq!(
        anchoring["issues"][0]["anchor_declaration"],
        "not_available_in_current_document_schema"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_reports_hardware_and_manufacturing_from_roles_and_source_geometry() {
    let query = "Iba pánty, výsuvy, diery a hrany";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Manufacturing validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport_at_size(
        transport.clone(),
        egui::Vec2::new(1_280.0, 1_600.0),
    );
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Opaque manufacturing A".to_owned(),
                    size_mm: [600.0, 500.0, 18.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque manufacturing B".to_owned(),
                    size_mm: [30.0, 30.0, 10.0],
                    origin_mm: [1.0, 50.0, 4.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque manufacturing C".to_owned(),
                    size_mm: [10.0, 10.0, 18.0],
                    origin_mm: [28.0, 50.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque manufacturing D".to_owned(),
                    size_mm: [500.0, 12.0, 45.0],
                    origin_mm: [0.0, 600.0, 100.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque manufacturing E".to_owned(),
                    size_mm: [480.0, 12.0, 45.0],
                    origin_mm: [0.0, 700.0, 105.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque manufacturing F".to_owned(),
                    size_mm: [600.0, 500.0, 4.0],
                    origin_mm: [1_000.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    shell.settle();
    let occurrence_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .occurrences()
            .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
            .collect::<BTreeMap<_, _>>()
    };
    let role_names = [
        "manufacturing.panel.xy:door-a",
        "manufacturing.hinge-cup.z:door-a",
        "manufacturing.hole.z:door-a",
        "hardware.linear-pair.x:drawer-a",
        "manufacturing.panel.xy:back-a",
    ];
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, role_names[0],)
    );
    let role_dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in &role_names[1..] {
        assert!(
            shell
                .app_mut()
                .add_classification_category(role_dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(role_dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    shell.settle();
    for (name, dimensions, role) in [
        ("Opaque manufacturing A", "600 × 500 × 18", role_names[0]),
        ("Opaque manufacturing B", "30 × 30 × 10", role_names[1]),
        ("Opaque manufacturing C", "10 × 10 × 18", role_names[2]),
        ("Opaque manufacturing D", "500 × 12 × 45", role_names[3]),
        ("Opaque manufacturing E", "480 × 12 × 45", role_names[3]),
        ("Opaque manufacturing F", "600 × 500 × 4", role_names[4]),
    ] {
        let row = shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", name.to_owned()),
                ("dimensions", dimensions.to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        );
        shell.click_row(&row);
        assert!(
            shell.app().occurrence_is_selected(occurrence_ids[name]),
            "headless outliner did not select {name} with label {row:?}"
        );
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_ids[role]),)
        );
    }
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
    shell.settle();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: occurrence_ids
                .values()
                .map(|occurrence_id| AssistantRotationIntent {
                    occurrence_id: Some(occurrence_id.0),
                    group_id: None,
                    pivot_mm: [0.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    angle_degrees: 45.0,
                })
                .collect(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
    assert_eq!(
        manufacturing["assumptions"],
        serde_json::json!([
            "canonical validator roles declare panels, holes, hinge cups, linear-hardware pairs, source axes, and association groups",
            "a hole or hinge-cup association group must resolve to exactly one explicitly declared host panel",
            "panel thickness and hole depth use declared source-frame axes bound to accepted topology",
            "hole radial envelopes are conservatively treated as circular using their largest source-frame radial extent",
            "each linear-hardware association group must contain exactly two members",
        ])
    );
    let issues = manufacturing["issues"].as_array().unwrap();
    let issue = |code: &str| {
        issues
            .iter()
            .find(|issue| issue["code"] == code)
            .unwrap_or_else(|| panic!("missing {code} in {issues:#?}"))
    };
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["name"],
        "Opaque manufacturing F"
    );
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["role"],
        "manufacturing.panel.xy:back-a"
    );
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["thickness_mm"],
        4.0
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["host_name"],
        "Opaque manufacturing A"
    );
    assert_eq!(
        issue("manufacturing.hinge_cup_envelope_below_minimum")["name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("manufacturing.hole_spacing_below_minimum")["left_name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("hardware.linear_pair_misaligned")["left_name"],
        "Opaque manufacturing D"
    );
    assert_eq!(
        issue("hardware.linear_pair_misaligned")["right_name"],
        "Opaque manufacturing E"
    );
    let evaluations = manufacturing["evaluations"].as_array().unwrap();
    let hole = evaluations
        .iter()
        .find(|evaluation| evaluation["rule"] == "hole_edge_distance")
        .unwrap();
    assert_eq!(hole["role"], "manufacturing.hinge-cup.z:door-a");
    assert_eq!(hole["host_role"], "manufacturing.panel.xy:door-a");
    assert_eq!(hole["topology_source"], "canonical_extrusion_topology");
    let pair = evaluations
        .iter()
        .find(|evaluation| evaluation["rule"] == "linear_hardware_pair_alignment")
        .unwrap();
    assert_eq!(pair["length_axis"], 0);
    assert_eq!(pair["length_mismatch_mm"], 20.0);
    assert_eq!(
        pair["topology_sources"],
        serde_json::json!([
            "canonical_extrusion_topology",
            "canonical_extrusion_topology"
        ])
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_reports_spatial_roles_and_oriented_narrow_phase() {
    let query = "Iba umiestnenie v miestnosti a priechodnosť";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        query.to_owned(),
        AssistantChatResult {
            message: "Room validation received".to_owned(),
            model_intent: None,
        },
    )]));
    let mut shell = Shell::with_assistant_transport_at_size(
        transport.clone(),
        egui::Vec2::new(1_280.0, 1_600.0),
    );
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Passage decoy A".to_owned(),
                    size_mm: [4_000.0, 3_000.0, 2_500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Room decoy B".to_owned(),
                    size_mm: [600.0, 400.0, 1_800.0],
                    origin_mm: [200.0, 200.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Opaque spatial C".to_owned(),
                    size_mm: [300.0, 600.0, 800.0],
                    origin_mm: [3_900.0, 100.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Furniture decoy D".to_owned(),
                    size_mm: [800.0, 2_500.0, 1_900.0],
                    origin_mm: [1_000.0, 250.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Room envelope decoy E".to_owned(),
                    size_mm: [500.0, 800.0, 900.0],
                    origin_mm: [1_200.0, 1_000.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Passage decoy F".to_owned(),
                    size_mm: [300.0, 500.0, 900.0],
                    origin_mm: [1_900.0, 1_000.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    shell.settle();
    let occurrence_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .occurrences()
            .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
            .collect::<BTreeMap<_, _>>()
    };
    let role_names = [
        "spatial.room:living",
        "spatial.furniture:living",
        "spatial.passage.xy:living",
        "spatial.obstacle:living",
    ];
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, role_names[0],)
    );
    let role_dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in &role_names[1..] {
        assert!(
            shell
                .app_mut()
                .add_classification_category(role_dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(role_dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    shell.settle();
    for (name, dimensions, role) in [
        ("Passage decoy A", "4000 × 3000 × 2500", role_names[0]),
        ("Room decoy B", "600 × 400 × 1800", role_names[1]),
        ("Opaque spatial C", "300 × 600 × 800", role_names[1]),
        ("Furniture decoy D", "800 × 2500 × 1900", role_names[2]),
        ("Room envelope decoy E", "500 × 800 × 900", role_names[1]),
        ("Passage decoy F", "300 × 500 × 900", role_names[3]),
    ] {
        let row = shell.catalog().format(
            "outliner-object",
            &BTreeMap::from([
                ("name", name.to_owned()),
                ("dimensions", dimensions.to_owned()),
                ("visibility", "◉".to_owned()),
            ]),
        );
        shell.click_row(&row);
        assert!(shell.app().occurrence_is_selected(occurrence_ids[name]));
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_ids[role]),)
        );
    }
    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
    shell.settle();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: occurrence_ids
                .values()
                .map(|occurrence_id| AssistantRotationIntent {
                    occurrence_id: Some(occurrence_id.0),
                    group_id: None,
                    pivot_mm: [0.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    angle_degrees: 45.0,
                })
                .collect(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
    assert_eq!(placement["issues"][0]["name"], "Opaque spatial C");
    assert_eq!(placement["issues"][0]["room_name"], "Passage decoy A");
    assert!(
        (placement["issues"][0]["outside_by_mm"]["right"]
            .as_f64()
            .unwrap()
            - 200.0)
            .abs()
            < 1.0e-9
    );
    assert_eq!(placement["issues"][0]["role"], "spatial.furniture:living");
    assert_eq!(placement["issues"][0]["room_role"], "spatial.room:living");
    assert_eq!(placement["issues"][0]["evidence_class"], "tolerant");
    assert_eq!(
        placement["issues"][0]["narrow_phase_method"],
        "ketchup.method.general-body-obb-sat.cpu-f64.v1"
    );

    let passage = &validation["passage_clearance"];
    assert_eq!(passage["state"], "failed");
    assert_eq!(passage["complete"], true);
    assert_eq!(passage["applicable_count"], 1);
    assert_eq!(passage["issue_count"], 2);
    assert_eq!(passage["limits"]["minimum_width_mm"], 900.0);
    assert_eq!(passage["limits"]["minimum_headroom_mm"], 2_000.0);
    assert_eq!(
        passage["evaluations"][0]["role"],
        "spatial.passage.xy:living"
    );
    assert_eq!(passage["evaluations"][0]["width_mm"], 800.0);
    assert_eq!(passage["evaluations"][0]["headroom_mm"], 1_900.0);
    let issues = passage["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue["code"] == "room.passage_envelope_below_minimum"
            && issue["name"] == "Furniture decoy D"
    }));
    let blocked = issues
        .iter()
        .find(|issue| issue["code"] == "room.passage_blocked")
        .unwrap();
    assert_eq!(blocked["obstacle_name"], "Room envelope decoy E");
    assert_eq!(blocked["obstacle_role"], "spatial.furniture:living");
    assert_eq!(blocked["narrow_phase_relation"], "intersecting");
    assert_eq!(
        blocked["narrow_phase_method"],
        "ketchup.method.general-body-obb-sat.cpu-f64.v1"
    );
    assert!(
        issues
            .iter()
            .all(|issue| issue["obstacle_name"] != "Passage decoy F"),
        "world-AABB-only overlap must not create a passage blocker: {issues:#?}"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn assistant_chat_does_not_claim_spatial_validation_without_canonical_roles() {
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
        validation["room_placement"]["role_error"],
        "validator role dimension is missing"
    );
    assert_eq!(
        validation["passage_clearance"]["role_error"],
        "validator role dimension is missing"
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
                    name: "Support decoy epsilon".to_owned(),
                    size_mm: [500.0, 500.0, 500.0],
                    origin_mm: [0.0, 0.0, 500.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Load decoy zeta".to_owned(),
                    size_mm: [500.0, 500.0, 500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Support decoy epsilon",
                "500 × 500 × 500",
                "physics.static.load:case-a",
            ),
            (
                "Load decoy zeta",
                "500 × 500 × 500",
                "physics.static.support:case-a",
            ),
        ],
    );
    let scene = shell.app().document_snapshot().scene_query();
    let loaded_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Support decoy epsilon")
        .unwrap()
        .occurrence_id
        .0;
    let support_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Load decoy zeta")
        .unwrap()
        .occurrence_id
        .0;
    let mass_name = format!("physics.mass_kg.occurrence.{loaded_id}");
    let load_name = format!("physics.applied_load_n.occurrence.{loaded_id}");
    let capacity_name = format!("physics.support_capacity_n.occurrence.{support_id}");
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
            (&mass_name, 100.0),
            (&load_name, 200.0),
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
    assert_eq!(evaluation["name"], "Support decoy epsilon");
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
    assert_eq!(evaluation["supports"][0]["name"], "Load decoy zeta");
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
        "missing_or_ambiguous_canonical_roles"
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
    let repair_program = shell
        .app()
        .assistant_repair_program()
        .expect("repair preview exposes its typed program")
        .clone();
    assert_eq!(repair_program.schema, ASSISTANT_REPAIR_PROGRAM_SCHEMA_V1);
    assert_eq!(
        repair_program.document_id,
        shell.app().document_snapshot().document_id().0
    );
    assert_eq!(repair_program.revision_id, revision);
    assert_eq!(repair_program.canonical_digest, digest);
    assert_eq!(repair_program.max_operations, 100);
    assert!(matches!(
        repair_program.operations.as_slice(),
        [AssistantRepairOperation::ResolveCollision { .. }]
    ));
    let encoded = serde_json::to_vec(&repair_program).unwrap();
    let decoded: AssistantRepairProgram = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, repair_program);
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
    assert_eq!(verification.repair_program.as_ref(), Some(&repair_program));
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
            boxes: vec![
                AssistantBoxIntent {
                    name: "Floating shelf".to_owned(),
                    size_mm: [100.0, 30.0, 18.0],
                    origin_mm: [0.0, 0.0, 50.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Ground reference".to_owned(),
                    size_mm: [100.0, 30.0, 10.0],
                    origin_mm: [0.0, 0.0, -10.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Floating shelf",
                "100 × 30 × 18",
                "physics.gravity.body:repair-case",
            ),
            (
                "Ground reference",
                "100 × 30 × 10",
                "physics.gravity.ground:repair-case",
            ),
        ],
    );
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
        ],
    );
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
fn assistant_repairs_gravity_support_along_the_typed_non_world_axis() {
    let mut shell = Shell::new();
    assert!(apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Neutral body alpha".to_owned(),
                    size_mm: [20.0, 30.0, 40.0],
                    origin_mm: [50.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Neutral support beta".to_owned(),
                    size_mm: [10.0, 30.0, 40.0],
                    origin_mm: [-10.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Neutral body alpha",
                "20 × 30 × 40",
                "physics.gravity.body:typed-axis",
            ),
            (
                "Neutral support beta",
                "10 × 30 × 40",
                "physics.gravity.ground:typed-axis",
            ),
        ],
    );
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", -9.81),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", 0.0),
        ],
    );
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

    let program = shell
        .app()
        .assistant_repair_program()
        .expect("typed gravity repair is previewed")
        .clone();
    assert_eq!(program.revision_id, revision);
    assert_eq!(program.canonical_digest, digest);
    match program.operations.as_slice() {
        [
            AssistantRepairOperation::RestoreGravitySupport {
                gravity_direction,
                delta_mm,
                ..
            },
        ] => {
            assert_eq!(*gravity_direction, [-1.0, 0.0, 0.0]);
            assert_eq!(*delta_mm, [-50.0, 0.0, 0.0]);
        }
        operations => panic!("unexpected typed gravity operations: {operations:?}"),
    }
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    let verification = shell.app().assistant_verification().unwrap();
    assert_eq!(verification.repair_program.as_ref(), Some(&program));
    assert_eq!(
        verification.validation_after.as_ref().unwrap()["gravity_support"]["unsupported_count"],
        0
    );
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
                AssistantBoxIntent {
                    name: "Batch ground reference".to_owned(),
                    size_mm: [100.0, 30.0, 10.0],
                    origin_mm: [300.0, 0.0, -10.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ));
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Left cabinet",
                "100 × 100 × 100",
                "physics.gravity.ground:repair-batch",
            ),
            (
                "Right cabinet",
                "100 × 100 × 100",
                "physics.gravity.ground:repair-batch",
            ),
            (
                "Floating shelf",
                "100 × 30 × 18",
                "physics.gravity.body:repair-batch",
            ),
            (
                "Batch ground reference",
                "100 × 30 × 10",
                "physics.gravity.ground:repair-batch",
            ),
        ],
    );
    apply_reviewed_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
        ],
    );
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
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
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
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
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
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
#[cfg(feature = "named-product-fixtures")]
fn assistant_teapot_intent_creates_smooth_hollow_saved_model_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_features = shell.app().feature_count();

    if !apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: Vec::new(),
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
                ketchup_bottle: None,
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ) {
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert!(shell.app().assistant_proposal().is_none());
        assert!(shell.app().action_digest().contains("non-editable mesh"));
        return;
    }

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().feature_count(), initial_features + 2);
    let snapshot = shell.app().document_snapshot();
    let body_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Rounded tea pot body occurrence")
        .expect("separate teapot body occurrence must exist");
    let lid_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Rounded tea pot lid occurrence")
        .expect("separate removable lid occurrence must exist");
    assert_ne!(
        body_occurrence.definition_id(),
        lid_occurrence.definition_id(),
        "the lid must be independently selectable and removable"
    );
    let body_feature = snapshot
        .features()
        .find(|feature| feature.name() == "Rounded tea pot smooth hollow body")
        .expect("teapot body mesh must exist");
    let lid_feature = snapshot
        .features()
        .find(|feature| feature.name() == "Rounded tea pot removable seated lid")
        .expect("separate seated lid mesh must exist");
    let FeatureKind::MeshBody(body_mesh) = body_feature.kind() else {
        panic!("teapot body must be a canonical mesh body");
    };
    let FeatureKind::MeshBody(lid_mesh) = lid_feature.kind() else {
        panic!("teapot lid must be a canonical mesh body");
    };
    let assert_closed_manifold = |mesh: &ketchup_core::document::MeshBodySpec| {
        let mut edges = BTreeMap::<(u32, u32), (usize, i32)>::new();
        let mut signed_volume = 0.0;
        for triangle in &mesh.triangles {
            let [a, b, c] = *triangle;
            assert!(a != b && b != c && c != a);
            let va = mesh.vertices_mm[a as usize];
            let vb = mesh.vertices_mm[b as usize];
            let vc = mesh.vertices_mm[c as usize];
            signed_volume += va[0] * (vb[1] * vc[2] - vb[2] * vc[1])
                + va[1] * (vb[2] * vc[0] - vb[0] * vc[2])
                + va[2] * (vb[0] * vc[1] - vb[1] * vc[0]);
            for (start, end) in [(a, b), (b, c), (c, a)] {
                let key = (start.min(end), start.max(end));
                let edge = edges.entry(key).or_default();
                edge.0 += 1;
                edge.1 += if start < end { 1 } else { -1 };
            }
        }
        assert!(
            edges
                .values()
                .all(|(count, balance)| *count == 2 && *balance == 0)
        );
        assert!(signed_volume.abs() > 1.0);
    };
    assert_closed_manifold(body_mesh);
    assert_closed_manifold(lid_mesh);
    assert!(body_mesh.vertices_mm.len() > 1_800);
    assert!(body_mesh.triangles.len() > 3_000);
    assert!(lid_mesh.vertices_mm.len() > 400);
    assert!(lid_mesh.triangles.len() > 800);
    assert!(body_mesh.vertices_mm.contains(&[42.0, 0.0, 141.0]));
    assert!(body_mesh.vertices_mm.contains(&[39.0, 0.0, 141.0]));
    assert!(body_mesh.vertices_mm.iter().any(|vertex| vertex[0] > 165.0));
    assert!(
        body_mesh
            .vertices_mm
            .iter()
            .any(|vertex| vertex[0] < -110.0)
    );
    let spout_tip_radii = body_mesh
        .vertices_mm
        .iter()
        .filter(|vertex| (vertex[0] - 170.8).abs() < 0.1 && (vertex[2] - 124.08).abs() < 0.1)
        .map(|vertex| vertex[1].abs())
        .collect::<Vec<_>>();
    assert!(spout_tip_radii.iter().copied().fold(0.0, f64::max) > 10.5);
    assert!(
        spout_tip_radii
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
            < 8.0,
        "the tapered spout tip must retain a distinct open inner radius"
    );
    assert!(
        lid_mesh
            .vertices_mm
            .iter()
            .any(|vertex| { vertex[2] < 141.0 && vertex[0].hypot(vertex[1]) > 37.0 })
    );
    assert!(
        lid_mesh
            .vertices_mm
            .iter()
            .any(|vertex| { vertex[2] > 141.0 && vertex[0].hypot(vertex[1]) > 45.0 })
    );
    assert!(matches!(
        &body_mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-rounded-teapot-body-v2"
    ));
    assert!(matches!(
        &lid_mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-removable-teapot-lid-v1"
    ));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-rounded-teapot.ketchup");
    persistence::save_atomic(&path, &snapshot).unwrap();
    let reopened = persistence::load_file(&path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    assert!(
        reopened
            .features()
            .any(|feature| feature.name() == "Rounded tea pot smooth hollow body")
    );
    assert!(
        reopened
            .features()
            .any(|feature| feature.name() == "Rounded tea pot removable seated lid")
    );
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assistant-rounded-teapot.ketchup");
    if std::env::var_os("UPDATE_ASSISTANT_TEAPOT_FIXTURE").is_some() {
        persistence::save_atomic(&fixture_path, &snapshot).unwrap();
    }
    let fixture = persistence::load_file(&fixture_path).unwrap().snapshot();
    for (name, expected) in [
        ("Rounded tea pot smooth hollow body", body_mesh),
        ("Rounded tea pot removable seated lid", lid_mesh),
    ] {
        let fixture_feature = fixture
            .features()
            .find(|feature| feature.name() == name)
            .expect("saved teapot fixture must retain both removable parts");
        let FeatureKind::MeshBody(fixture_mesh) = fixture_feature.kind() else {
            panic!("saved teapot fixture parts must remain canonical mesh bodies");
        };
        assert_eq!(fixture_mesh.vertices_mm.len(), expected.vertices_mm.len());
        assert_eq!(fixture_mesh.triangles.len(), expected.triangles.len());
        assert_eq!(fixture_mesh.authority, expected.authority);
    }
    assert_eq!(
        fixture
            .occurrences()
            .filter(|occurrence| occurrence.name().starts_with("Rounded tea pot"))
            .count(),
        2
    );

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_balloon_text_creates_inflated_letters_with_holes_depth_save_and_undo() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    if !apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            balloon_texts: vec![AssistantBalloonTextIntent {
                name: "Balloon KECUP".to_owned(),
                text: "KECUP".to_owned(),
                height_mm: 120.0,
                depth_mm: 42.0,
                stroke_width_mm: 20.0,
                letter_spacing_mm: 12.0,
                origin_mm: [25.0, 10.0, 5.0],
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ) {
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert!(shell.app().assistant_proposal().is_none());
        assert!(shell.app().action_digest().contains("non-editable mesh"));
        return;
    }
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot
        .features()
        .find(|feature| feature.name() == "Balloon KECUP inflated text")
        .expect("balloon text mesh must exist");
    let FeatureKind::MeshBody(mesh) = feature.kind() else {
        panic!("balloon text must be one canonical mesh body");
    };
    assert!(mesh.vertices_mm.len() > 3_000);
    assert!(mesh.triangles.len() > 6_000);
    assert!(mesh.vertices_mm.iter().any(|point| point[1] == -21.0));
    assert!(mesh.vertices_mm.iter().any(|point| point[1] == 21.0));
    let depth_layers = mesh
        .vertices_mm
        .iter()
        .map(|point| (point[1] * 1_000.0).round() as i64)
        .collect::<BTreeSet<_>>();
    assert!(
        depth_layers.len() > 20,
        "inflated glyphs must have continuously rounded depth, not three beveled slabs"
    );
    assert!(
        !mesh
            .vertices_mm
            .iter()
            .any(|point| (point[0] - 520.8).abs() < 5.0 && (point[2] - 90.0).abs() < 5.0),
        "the bowl of P must remain a through opening"
    );
    assert!(matches!(
        &mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-balloon-text-v2"
    ));
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Balloon KECUP occurrence")
        .expect("balloon text occurrence must exist");
    let transform = occurrence.transform();
    let matrix = transform.matrix();
    assert_eq!([matrix[3], matrix[7], matrix[11]], [25.0, 10.0, 5.0]);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-balloon-letters.ketchup");
    persistence::save_atomic(&path, &snapshot).unwrap();
    let reopened = persistence::load_file(&path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assistant-balloon-letters.ketchup");
    if std::env::var_os("UPDATE_ASSISTANT_BALLOON_FIXTURE").is_some() {
        persistence::save_atomic(&fixture_path, &snapshot).unwrap();
    }
    let fixture = persistence::load_file(&fixture_path).unwrap().snapshot();
    let fixture_feature = fixture
        .features()
        .find(|feature| feature.name() == "Balloon KECUP inflated text")
        .expect("saved balloon text fixture must remain openable");
    let FeatureKind::MeshBody(fixture_mesh) = fixture_feature.kind() else {
        panic!("saved balloon text fixture must retain its canonical mesh body");
    };
    assert_eq!(fixture_mesh.vertices_mm.len(), mesh.vertices_mm.len());
    assert_eq!(fixture_mesh.triangles.len(), mesh.triangles.len());
    assert_eq!(fixture_mesh.authority, mesh.authority);

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn assistant_balloon_text_supports_the_complete_rounded_uppercase_and_digit_alphabet() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    if !apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            balloon_texts: vec![
                AssistantBalloonTextIntent {
                    name: "Complete balloon alphabet".to_owned(),
                    text: "ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_owned(),
                    height_mm: 40.0,
                    depth_mm: 16.0,
                    stroke_width_mm: 8.0,
                    letter_spacing_mm: 4.0,
                    origin_mm: [0.0, 0.0, 0.0],
                },
                AssistantBalloonTextIntent {
                    name: "Complete balloon digits".to_owned(),
                    text: "0123456789".to_owned(),
                    height_mm: 40.0,
                    depth_mm: 16.0,
                    stroke_width_mm: 8.0,
                    letter_spacing_mm: 4.0,
                    origin_mm: [0.0, 0.0, 60.0],
                },
                AssistantBalloonTextIntent {
                    name: "Standalone balloon caron".to_owned(),
                    text: "ˇ".to_owned(),
                    height_mm: 40.0,
                    depth_mm: 16.0,
                    stroke_width_mm: 8.0,
                    letter_spacing_mm: 4.0,
                    origin_mm: [0.0, 0.0, 120.0],
                },
            ],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
        },
    ) {
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert!(shell.app().assistant_proposal().is_none());
        assert!(shell.app().action_digest().contains("non-editable mesh"));
        return;
    }
    let snapshot = shell.app().document_snapshot();
    for name in [
        "Complete balloon alphabet inflated text",
        "Complete balloon digits inflated text",
        "Standalone balloon caron inflated text",
    ] {
        let feature = snapshot
            .features()
            .find(|feature| feature.name() == name)
            .expect("every supported balloon glyph must produce a mesh body");
        let FeatureKind::MeshBody(mesh) = feature.kind() else {
            panic!("every supported balloon glyph must be a canonical mesh body");
        };
        assert!(mesh.vertices_mm.len() < 100_000);
        assert!(mesh.triangles.len() < 200_000);
        assert!(matches!(
            &mesh.authority,
            ketchup_core::document::MeshAuthority::Authored { provenance }
                if provenance == "ketchup-assistant-balloon-text-v2"
        ));
    }
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn assistant_ketchup_bottle_creates_saved_rounded_squeeze_model_as_one_undo_step() {
    let mut shell = Shell::new();
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    if !apply_reviewed_model_intent(
        &mut shell,
        AssistantModelIntent {
            replace_scene: true,
            boxes: Vec::new(),
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: vec![AssistantBottleIntent {
                name: "Kečup squeeze bottle".to_owned(),
                body_radius_mm: 38.0,
                body_height_mm: 145.0,
                shoulder_rise_mm: 28.0,
                neck_radius_mm: 15.0,
                neck_height_mm: 18.0,
                wall_thickness_mm: 2.0,
                finish_kind: AssistantBottleFinishKind::Fillet,
                finish_amount_mm: 2.0,
                origin_mm: [0.0, 0.0, 0.0],
                teapot: None,
                ketchup_bottle: Some(AssistantKetchupBottleIntent {
                    body_depth_ratio: 0.68,
                    cap_radius_mm: 19.5,
                    cap_height_mm: 24.0,
                    label_width_mm: 58.0,
                    label_height_mm: 72.0,
                    label_relief_mm: 2.5,
                    grip_rib_count: 20,
                }),
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        },
    ) {
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert!(shell.app().assistant_proposal().is_none());
        assert!(shell.app().action_digest().contains("non-editable mesh"));
        return;
    }
    let snapshot = shell.app().document_snapshot();
    let body_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Kečup squeeze bottle body occurrence")
        .expect("separate bottle body occurrence must exist");
    let cap_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Kečup squeeze bottle cap occurrence")
        .expect("separate removable cap occurrence must exist");
    assert_ne!(
        body_occurrence.definition_id(),
        cap_occurrence.definition_id(),
        "the cap must be independently selectable and removable"
    );
    let body_feature = snapshot
        .features()
        .find(|feature| feature.name() == "Kečup squeeze bottle clean threaded body")
        .expect("clean threaded bottle body must exist");
    let cap_feature = snapshot
        .features()
        .find(|feature| feature.name() == "Kečup squeeze bottle removable threaded cap")
        .expect("removable threaded cap must exist");
    let FeatureKind::MeshBody(body_mesh) = body_feature.kind() else {
        panic!("ketchup bottle body must be a canonical mesh body");
    };
    let FeatureKind::MeshBody(cap_mesh) = cap_feature.kind() else {
        panic!("ketchup bottle cap must be a canonical mesh body");
    };
    assert!(body_mesh.vertices_mm.len() > 3_000);
    assert!(body_mesh.triangles.len() > 6_000);
    assert!(cap_mesh.vertices_mm.len() > 3_000);
    assert!(cap_mesh.triangles.len() > 6_000);
    let x_extent = body_mesh
        .vertices_mm
        .iter()
        .map(|point| point[0].abs())
        .fold(0.0, f64::max);
    let y_extent = body_mesh
        .vertices_mm
        .iter()
        .filter(|point| point[2] < 145.0)
        .map(|point| point[1].abs())
        .fold(0.0, f64::max);
    assert!(x_extent > y_extent * 1.25);
    let body_thread_radii = body_mesh
        .vertices_mm
        .iter()
        .filter(|point| point[2] > 174.0 && point[2] < 190.0)
        .map(|point| point[0].hypot(point[1]))
        .collect::<Vec<_>>();
    assert!(
        body_thread_radii.iter().copied().fold(0.0, f64::max) > 16.0,
        "the neck must carry a raised external helix"
    );
    assert!(
        body_thread_radii
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
            < 15.1,
        "the helical neck must retain land between thread turns"
    );
    let cap_inner_radii = cap_mesh
        .vertices_mm
        .iter()
        .filter(|point| {
            point[2] > 173.0
                && point[2] < 193.0
                && point[0].hypot(point[1]) > 14.0
                && point[0].hypot(point[1]) < 17.5
        })
        .map(|point| point[0].hypot(point[1]))
        .collect::<Vec<_>>();
    assert!(
        cap_inner_radii.iter().copied().fold(0.0, f64::max) > 16.4,
        "the cap must carry a complementary internal helical groove"
    );
    assert!(
        cap_inner_radii
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
            < 15.5,
        "the cap groove must return to its clearance land"
    );
    assert!(matches!(
        &body_mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-squeeze-bottle-body-v2"
    ));
    assert!(matches!(
        &cap_mesh.authority,
        ketchup_core::document::MeshAuthority::Authored { provenance }
            if provenance == "ketchup-assistant-threaded-cap-v1"
    ));
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/assistant-ketchup-squeeze-bottle.ketchup");
    if std::env::var_os("UPDATE_ASSISTANT_KETCHUP_FIXTURE").is_some() {
        persistence::save_atomic(&fixture_path, &snapshot).unwrap();
    }
    let fixture = persistence::load_file(&fixture_path).unwrap().snapshot();
    assert!(
        fixture
            .features()
            .any(|feature| { feature.name() == "Kečup squeeze bottle clean threaded body" })
    );
    assert!(
        fixture
            .features()
            .any(|feature| { feature.name() == "Kečup squeeze bottle removable threaded cap" })
    );
    assert_eq!(
        fixture
            .occurrences()
            .filter(|occurrence| occurrence.name().starts_with("Kečup squeeze bottle"))
            .count(),
        2
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
#[cfg(feature = "named-product-fixtures")]
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
            rotations: Vec::new(),
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
                ketchup_bottle: None,
            }],
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().occurrence_count(), 3);
    assert_eq!(shell.app().active_box_count(), 3);
    let snapshot = shell.app().document_snapshot();
    let roof_definition = snapshot
        .definitions()
        .find(|definition| definition.name() == "True gable roof")
        .expect("gable roof definition must exist");
    let roof_features = roof_definition
        .feature_ids()
        .iter()
        .map(|id| snapshot.feature(*id).unwrap())
        .collect::<Vec<_>>();
    assert!(
        roof_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Workplane(_)))
    );
    let roof_sketch = roof_features
        .iter()
        .find_map(|feature| match feature.kind() {
            FeatureKind::Sketch(sketch) => Some(sketch),
            _ => None,
        })
        .expect("gable roof editable sketch must exist");
    assert_eq!(roof_sketch.entities.len(), 6);
    assert!(
        roof_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
    );
    assert!(
        !roof_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
    );
    let floor_definition = snapshot
        .definitions()
        .find(|definition| definition.name() == "Attic floor with stair opening")
        .expect("floor opening definition must exist");
    let floor_features = floor_definition
        .feature_ids()
        .iter()
        .map(|id| snapshot.feature(*id).unwrap())
        .collect::<Vec<_>>();
    assert!(
        floor_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
    );
    assert!(
        floor_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Boolean { .. }))
    );
    assert!(
        floor_features
            .iter()
            .all(|feature| !matches!(feature.kind(), FeatureKind::MeshBody(_)))
    );
    let floor_producer = *floor_definition.feature_ids().last().unwrap();
    assert!(
        ExactBRepGraph::from_snapshot(&snapshot, floor_definition.id(), floor_producer).is_ok()
    );
    let stairs_definition = snapshot
        .definitions()
        .find(|definition| definition.name() == "Attic staircase")
        .expect("staircase definition must exist");
    let stairs_features = stairs_definition
        .feature_ids()
        .iter()
        .map(|id| snapshot.feature(*id).unwrap())
        .collect::<Vec<_>>();
    let stairs_sketch = stairs_features
        .iter()
        .find_map(|feature| match feature.kind() {
            FeatureKind::Sketch(sketch) => Some(sketch),
            _ => None,
        })
        .expect("staircase editable sketch must exist");
    assert_eq!(stairs_sketch.entities.len(), 32);
    assert!(
        stairs_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
    );
    assert!(
        !stairs_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
    );

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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().occurrence_count(), 5);
    let snapshot = shell.app().document_snapshot();
    let rafter_definition = snapshot
        .definitions()
        .find(|definition| definition.name() == "Left rafter 1")
        .expect("rafter definition must exist");
    let rafter_features = rafter_definition
        .feature_ids()
        .iter()
        .map(|id| snapshot.feature(*id).unwrap())
        .collect::<Vec<_>>();
    let rafter_sketch = rafter_features
        .iter()
        .find_map(|feature| match feature.kind() {
            FeatureKind::Sketch(sketch) => Some(sketch),
            _ => None,
        })
        .expect("rafter editable sketch must exist");
    assert_eq!(rafter_sketch.entities.len(), 8);
    assert!(
        rafter_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
    );
    assert!(
        !rafter_features
            .iter()
            .any(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
    );
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
fn top_bar_zoom_fit_physically_frames_saved_organic_models() {
    for fixture in [
        "assistant-balloon-letters.ketchup",
        "assistant-ketchup-squeeze-bottle.ketchup",
        "assistant-rounded-teapot.ketchup",
    ] {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(fixture);
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&path)
            .always_discard();
        let window = egui::Vec2::new(1_100.0, 720.0);
        let mut shell = Shell::with_dialogs_at_size(dialogs, window);
        shell.click_menu_command("menu-file", AppCommand::Open);
        shell.settle();

        let viewport = shell.viewport_rect();
        for _ in 0..8 {
            shell.scroll_at(viewport.center(), 120.0);
        }
        let zoom_before = shell.app().camera_zoom();
        let button = shell.command_rect(AppCommand::ZoomFit);
        assert!(
            button.center().x >= 0.0 && button.center().x <= window.x,
            "{fixture} Zoom Fit button must be on screen, got {button:?}"
        );
        shell.click_at_once(button.center());

        assert_ne!(
            shell.app().camera_zoom(),
            zoom_before,
            "{fixture} top-bar Zoom Fit click must change the camera"
        );
        let expected = shell.catalog().format(
            "digest-zoom-fit",
            &std::collections::BTreeMap::from([(
                "count",
                shell.app().active_box_count().to_string(),
            )]),
        );
        assert_eq!(shell.app().action_digest(), expected);

        let snapshot = shell.app().document_snapshot();
        for occurrence in snapshot
            .occurrences()
            .filter(|occurrence| occurrence.visible())
        {
            let transform = occurrence.transform();
            let matrix = transform.matrix();
            let definition = snapshot.definition(occurrence.definition_id()).unwrap();
            for feature_id in definition.feature_ids() {
                let feature = snapshot.feature(*feature_id).unwrap();
                let FeatureKind::MeshBody(mesh) = feature.kind() else {
                    continue;
                };
                for vertex in &mesh.vertices_mm {
                    let world = Vec3::new(
                        matrix[0] * vertex[0]
                            + matrix[1] * vertex[1]
                            + matrix[2] * vertex[2]
                            + matrix[3],
                        matrix[4] * vertex[0]
                            + matrix[5] * vertex[1]
                            + matrix[6] * vertex[2]
                            + matrix[7],
                        matrix[8] * vertex[0]
                            + matrix[9] * vertex[1]
                            + matrix[10] * vertex[2]
                            + matrix[11],
                    );
                    let screen = shell.app().project_to_screen(world, viewport);
                    assert!(
                        viewport.contains(screen),
                        "{fixture} Zoom Fit omitted mesh vertex {world:?}, projected to {screen:?}"
                    );
                }
            }
        }
    }
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    ));

    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let snapshot = shell.app().document_snapshot();
    assert_eq!(snapshot.occurrences().count(), 1);
    assert_eq!(snapshot.definitions().count(), 1);
    let features = snapshot.features().collect::<Vec<_>>();
    assert_eq!(
        features
            .iter()
            .filter(|feature| matches!(feature.kind(), FeatureKind::Boolean { .. }))
            .count(),
        17
    );
    assert_eq!(
        features
            .iter()
            .filter(|feature| matches!(feature.kind(), FeatureKind::Pad(_)))
            .count(),
        18
    );
    assert!(
        features
            .iter()
            .all(|feature| !matches!(feature.kind(), FeatureKind::MeshBody(_)))
    );
    let definition = snapshot.definitions().next().unwrap();
    let producer = *definition.feature_ids().last().unwrap();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition.id(), producer).unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::Boolean { .. }))
            .count(),
        17
    );
    assert_eq!(
        graph.producer_bounds_mm().unwrap(),
        Some([[0.0, 0.0, 0.0], [10_000.0, 160.0, 160.0]])
    );

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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
    assert_eq!(context["boxes"].as_array().unwrap().len(), 24);
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
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
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
            rotations: Vec::new(),
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
            balloon_texts: Vec::new(),
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
