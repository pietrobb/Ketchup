//! Typed CAD edit programs (the MCP `edit` surface) build exact, persistent
//! features as one revision and one undo step.

use crate::harness;

use eframe::egui::accesskit::Role;
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_assistant::sidecar::{
    AssistantAssemblyJointAxis, AssistantAssemblyJointKind, AssistantAssemblyJointLimits,
    AssistantAxisSpec, AssistantCadBodyFeature, AssistantCadBooleanOperation,
    AssistantCadChamferMode, AssistantCadDeletePolicy, AssistantCadEditOperation,
    AssistantCadEditProgram, AssistantCadEntitySelector, AssistantCadFeatureReference,
    AssistantCadFilletRadiusStation, AssistantCadLoftContinuity, AssistantCadLoftSection,
    AssistantCadPartFeature, AssistantCadProgramFeatureOutput, AssistantCadProgramFeatureReference,
    AssistantCadRotation, AssistantCadShellDirection, AssistantCadSurfaceBodySource,
    AssistantInstancePath, AssistantInstancePathStep, AssistantPrincipalPlane,
    AssistantSketchConstraint, AssistantSketchEntity, AssistantSketchPointKind,
    AssistantSketchPointRef, AssistantWorkplaneSpec,
};
use ketchup_geometry::sketch::{FeatureExtent, PadSpec, SketchConstraintKind, SketchEntity};
use ketchup_geometry::sketch::{PadOperation, PadProfile};
use ketchup_model::document::{
    BodyId, BooleanOperation, CanonicalCommand, ChamferEdgeSide, ChamferMode, CommandBatch,
    DefinitionId, Dimension, DocumentStore, EdgeFinishKind, EdgeRef, FeatureId, FeatureKind,
    GroupId, InstancePath, InstancePathStep, LoftSection, OccurrenceId, ProfileSegment,
    SpatialPathSegment, Transform,
};
use ketchup_model::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V15, EXACT_BREP_GRAPH_SCHEMA_V17,
    EXACT_BREP_GRAPH_SCHEMA_V18, ExactBRepGraph, ExactBRepLoftContinuity, ExactBRepOperation,
};
use ketchup_model::exact_product::ExactBodyPackage;
use ketchup_model::persistence;
use ketchup_model::state_view::encode_semantic_state;
use ketchup_model::topology::{TopologicalElementKind, TopologicalReferenceStability};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeMap;

/// Plans and commits one typed CAD program the way an MCP client's
/// `edit propose` + `commit` does: one revision and one undo step.
fn apply_program(shell: &mut Shell, request: &str, program: &AssistantCadEditProgram) {
    if let Err(error) = shell.app_mut().headless_apply_cad_program(program) {
        panic!("{request}: the program must apply: {error}");
    }
    shell.settle();
}

fn exact_worker_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn write_boolean_fixture(path: &std::path::Path) {
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Target extrusion".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::from_decimal("20").unwrap()),
            },
            CanonicalCommand::CreateBody {
                definition_id: DefinitionId(1),
                id: BodyId(2),
                name: "Tool body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DefinitionId(1),
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Tool profile".to_owned(),
                kind: FeatureKind::polygon(&[
                    [40.0, 0.0],
                    [120.0, 0.0],
                    [120.0, 60.0],
                    [40.0, 60.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(3), Dimension::from_decimal("20").unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Boolean inputs".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn write_planar_offset_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Planar offset input".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[
                    [10.0, 20.0],
                    [110.0, 20.0],
                    [110.0, 100.0],
                    [10.0, 100.0],
                ]),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Planar offset input".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn write_sweep_fixture(path: &std::path::Path) {
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
                kind: FeatureKind::Profile {
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
                name: "Non-coplanar mixed sweep path".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![
                        SpatialPathSegment::Line {
                            start_mm: [0.0, 0.0, 0.0],
                            end_mm: [30.0, 0.0, 0.0],
                        },
                        SpatialPathSegment::CircularArc {
                            start_mm: [30.0, 0.0, 0.0],
                            end_mm: [40.0, 10.0, 0.0],
                            center_mm: [30.0, 10.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            clockwise: false,
                        },
                        SpatialPathSegment::CubicBezier {
                            start_mm: [40.0, 10.0, 0.0],
                            control_1_mm: [40.0, 20.0, 0.0],
                            control_2_mm: [40.0, 30.0, 10.0],
                            end_mm: [40.0, 40.0, 20.0],
                        },
                    ],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Sweep inputs".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn write_loft_fixture(path: &std::path::Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Loft inputs".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Lower spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -4.0],
                    [9.0, -3.0],
                    [7.0, 6.0],
                    [-6.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Upper spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-4.0, -2.0],
                    [5.0, -2.0],
                    [4.0, 3.0],
                    [-3.0, 4.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Loft guide".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 35.0],
                    }],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Loft inputs".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

#[test]
fn cad_edit_program_reviews_selection_transform_copy_pattern_mirror_delete_and_one_undo() {
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
                secondary: None,
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
    let mut shell = Shell::new();
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

    apply_program(&mut shell, request, &program);
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

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
    let undone = shell.app().document_snapshot();
    assert!(undone.occurrence(OccurrenceId(1)).is_some());
    for id in 2..=5 {
        assert!(undone.occurrence(OccurrenceId(id)).is_none());
    }
}

#[test]
fn create_part_program_round_trips_state_view_and_one_step_undo_redo() {
    let request = "Create one editable extruded part";
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            holes: Vec::new(),
            pockets: Vec::new(),
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
    let program_request = program.clone();
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    // The exact result of the new part registers its face evidence in this
    // revision; the state compared after Redo is the one that includes it.
    let new_part = shell
        .app()
        .document_snapshot()
        .definitions()
        .find(|definition| definition.name() == "Editable prism")
        .unwrap()
        .id();
    assert!(
        shell.wait_until(|shell| {
            shell.step();
            shell
                .app()
                .document_snapshot()
                .exact_reference_evidence()
                .any(|evidence| evidence.definition_id == new_part)
        }),
        "the exact result of the new part was never published"
    );
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
    let committed_complete_view = committed_state.complete();
    let committed_agent_view = committed_state.agent();
    assert!(
        committed_complete_view.contains(&format!("source.canonical_digest={committed_digest}"))
    );
    for expected_kind in ["Workplane", "Sketch", "Pad"] {
        assert!(committed_complete_view.contains(&format!(".kind.{expected_kind}.")));
        assert!(committed_agent_view.contains(&format!("kind:{{{expected_kind}:")));
    }
    assert!(!committed_complete_view.contains(".kind.MeshBody."));
    assert!(!committed_agent_view.contains("kind:{MeshBody:"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-editable-prism.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    let reopened = outcome.snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_state = encode_semantic_state(&reopened);
    assert_eq!(reopened_state.complete(), committed_complete_view);
    assert_eq!(reopened_state.agent(), committed_agent_view);

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
        encode_semantic_state(&shell.app().document_snapshot()).complete(),
        committed_complete_view
    );
}

#[test]
fn create_revolved_part_round_trips_state_view_and_one_step_undo_redo() {
    let request = "Create one editable revolved part";
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            holes: Vec::new(),
            pockets: Vec::new(),
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
                axis: AssistantAxisSpec::TwoPoints {
                    start_mm: [0.0, 0.0, 0.0],
                    end_mm: [0.0, 1.0, 0.0],
                },
                angle_degrees: 275.0,
            },
            translation_mm: [10.0, 20.0, 30.0],
            rotation: None,
        }],
    };
    let program_request = program;
    let mut shell = Shell::new();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    // Exact evaluation publishes reference evidence into the revision; the view
    // compared after Redo must be captured once it has arrived.
    shell.wait_until(|shell| {
        shell.settle();
        shell.app().exact_render_body_count() == baseline_occurrences + 1
    });
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
    let committed_complete_view = committed_state.complete();
    let committed_agent_view = committed_state.agent();
    assert!(committed_complete_view.contains(".kind.Revolve."));
    assert!(committed_agent_view.contains("kind:{Revolve:"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-editable-revolve.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    let reopened = outcome.snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_state = encode_semantic_state(&reopened);
    assert_eq!(reopened_state.complete(), committed_complete_view);
    assert_eq!(reopened_state.agent(), committed_agent_view);

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
        encode_semantic_state(&shell.app().document_snapshot()).complete(),
        committed_complete_view
    );
}

#[test]
fn append_boolean_programs_are_exact_persistent_and_one_step() {
    let requests = [
        "Append an exact cut",
        "Append an exact union",
        "Append an exact intersect",
    ];
    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-boolean-inputs.ketchup");
    write_boolean_fixture(&fixture_path);
    let mut shell = Shell::new();
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
        // Each boolean refines the previous result, so body 1 keeps one terminal feature.
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: 1.into(),
                name: format!("Program {assistant_operation:?}"),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: assistant_operation,
                    target_feature_id: [2, 5, 6][index].into(),
                    tool_feature_id: 4.into(),
                },
            }],
        };
        apply_program(&mut shell, request, &program);
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
        let committed = shell.app().document_snapshot();
        let feature_id = FeatureId(5 + index as u64);
        let expected_target = if index == 0 {
            FeatureId(2)
        } else {
            FeatureId(4 + index as u64)
        };
        assert!(matches!(
            committed.feature(feature_id).unwrap().kind(),
            FeatureKind::Boolean {
                operation,
                target,
                tool: FeatureId(4),
            } if *operation == canonical_operation && *target == expected_target
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
}

#[test]
fn append_pocket_is_exact_persistent_and_one_step() {
    let request = "Pocket the existing extrusion";
    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2.into(),
                profile_feature_id: 3.into(),
                depth_mm: 8.0,
            },
        }],
    };

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-pocket-inputs.ketchup");
    write_boolean_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(5)).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(FeatureId(3)), extent: FeatureExtent::Blind(depth), operation: PadOperation::Cut { target: FeatureId(2), .. }, .. }) if depth.millimetres() == 8.0
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
}

#[test]
fn append_pocket_rejects_missing_profile_without_mutation() {
    let request = "Create an opening from missing profile 999";
    let invalid_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Rejected Assistant pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2.into(),
                profile_feature_id: 999.into(),
                depth_mm: 8.0,
            },
        }],
    };

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory
        .path()
        .join("assistant-invalid-pocket-inputs.ketchup");
    write_boolean_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();

    let rejection = shell
        .app_mut()
        .headless_apply_cad_program(&invalid_program)
        .expect_err(request)
        .to_string();
    assert!(rejection.contains("999"), "{rejection}");
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
}

#[test]
fn append_planar_offset_is_exact_persistent_and_one_step() {
    let request = "Offset the existing rectangle inward";
    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant planar offset".to_owned(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id: 1,
                distance_mm: -7.5,
            },
        }],
    };

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory
        .path()
        .join("assistant-planar-offset-input.ketchup");
    write_planar_offset_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::PlanarOffset {
            profile: FeatureId(1),
            distance,
        } if distance.millimetres() == -7.5
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(2)).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.is_current(&committed));
    // The 100 x 80 mm rectangle offset inward by 7.5 mm on every side.
    assert_eq!(package.bounds_mm, [[17.5, 27.5, 0.0], [102.5, 92.5, 0.0]]);
    assert!((package.area_mm2 - 85.0 * 65.0).abs() <= 1.0e-6);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory
        .path()
        .join("assistant-planar-offset-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(2)).unwrap();
    let reopened_package = worker.evaluate_exact_brep_graph(&reopened_graph).unwrap();
    assert!(reopened_package.is_current(&reopened));
    assert_eq!(reopened_package.bounds_mm, package.bounds_mm);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
}

#[test]
fn append_sweep_is_exact_persistent_and_one_step() {
    let request = "Sweep the existing profile along the existing path";
    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: 1,
                path_feature_id: 2,
            },
        }],
    };

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-sweep-inputs.ketchup");
    write_sweep_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Sweep {
            profile: FeatureId(1),
            path: FeatureId(2),
            ..
        }
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(graph.producer_feature_id, 3);
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V12);
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    )));
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
}

#[test]
fn append_guided_loft_json_is_exact_persistent_and_one_step() {
    let request = "Loft the existing spline profiles";
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant guided loft".to_owned(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: 1.into(),
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: 2.into(),
                        elevation_mm: 35.0,
                    },
                ],
                guide_feature_id: Some(3.into()),
                continuity: AssistantCadLoftContinuity::Tangent,
            },
        }],
    };
    let json = serde_json::to_value(&program).unwrap();
    assert_eq!(json["operations"][0]["feature"]["guide_feature_id"], 3);
    assert_eq!(json["operations"][0]["feature"]["continuity"], "tangent");
    let program_request =
        serde_json::from_value(json).expect("public guided Loft JSON must round-trip");

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-loft-inputs.ketchup");
    write_loft_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Loft {
            sections,
            guide: Some(FeatureId(3)),
            continuity: ketchup_model::document::LoftContinuity::Tangent,
        } if sections == &vec![
            LoftSection { profile: FeatureId(1), elevation_mm: 0.0 },
            LoftSection { profile: FeatureId(2), elevation_mm: 35.0 },
        ]
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(4)).unwrap();
    assert_eq!(graph.producer_feature_id, 4);
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V15);
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::Loft {
            guide: Some(guide),
            continuity: ExactBRepLoftContinuity::Tangent,
            ..
        } if guide.source_feature_id == 3 && guide.segments.len() == 1
    )));
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.volume_mm3 > 0.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-loft-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(4)).unwrap();
    assert_eq!(reopened_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V15);
    assert!(matches!(
        &reopened_graph.nodes[0].operation,
        ExactBRepOperation::Loft {
            guide: Some(guide),
            continuity: ExactBRepLoftContinuity::Tangent,
            ..
        } if guide.source_feature_id == 3
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
}

#[test]
fn append_closed_symmetric_shell_is_exact_persistent_and_one_step() {
    let request = "Create a closed symmetric shell from the existing exact body";
    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-shell-input.ketchup");
    let fixture = ketchup_app::KetchupApp::new();
    let fixture_snapshot = fixture.document_snapshot();
    persistence::save_atomic(&fixture_path, &fixture_snapshot).unwrap();

    let base_graph =
        ExactBRepGraph::from_snapshot(&fixture_snapshot, DefinitionId(1), FeatureId(2)).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();
    let base_volume = base_package.volume_mm3;

    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant shell".to_owned(),
            feature: AssistantCadBodyFeature::TopologyShell {
                target_feature_id: 2,
                removed_face_reference_ids: Vec::new(),
                thickness_mm: 2.0,
                direction: ketchup_assistant::sidecar::AssistantCadShellDirection::Symmetric,
            },
        }],
    };

    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Shell {
            target: FeatureId(2),
            removed_faces,
            thickness,
            direction: ketchup_model::document::ShellDirection::Symmetric,
            ..
        } if removed_faces.is_empty() && thickness.millimetres() == 2.0
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(
        graph.schema,
        ketchup_model::exact_brep_graph::EXACT_BREP_GRAPH_SCHEMA_V16
    );
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::Shell {
            removed_faces,
            direction: ketchup_model::exact_brep_graph::ExactBRepShellDirection::Symmetric,
            ..
        } if removed_faces.is_empty()
    )));
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.volume_mm3 > 0.0);
    assert!((package.volume_mm3 - base_volume).abs() > 1.0e-6);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-shell-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(matches!(
        reopened.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Shell {
            removed_faces,
            direction: ketchup_model::document::ShellDirection::Symmetric,
            ..
        } if removed_faces.is_empty()
    ));
    assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(3)).is_ok());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
}

#[test]
fn append_topology_fillet_is_exact_persistent_and_one_step() {
    let request = "Fillet an existing exact body edge";
    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-fillet-input.ketchup");
    let fixture = ketchup_app::KetchupApp::new();
    let fixture_snapshot = fixture.document_snapshot();
    persistence::save_atomic(&fixture_path, &fixture_snapshot).unwrap();

    let base_graph =
        ExactBRepGraph::from_snapshot(&fixture_snapshot, DefinitionId(1), FeatureId(2)).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();
    let reference_id = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference
                .producer_element_id
                .starts_with("generated-result/edge/")
        })
        .unwrap()
        .lineage_digest
        .clone();

    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant fillet".to_owned(),
            feature: AssistantCadBodyFeature::TopologyFillet {
                target_feature_id: 2,
                edge_reference_ids: vec![reference_id.clone()],
                radius_mm: 2.0,
                radius_stations: vec![
                    AssistantCadFilletRadiusStation {
                        position: 0.5,
                        radius_mm: 3.0,
                    },
                    AssistantCadFilletRadiusStation {
                        position: 1.0,
                        radius_mm: 1.0,
                    },
                ],
            },
        }],
    };

    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::EdgeFinish {
            target: FeatureId(2),
            edges,
            kind: EdgeFinishKind::Fillet,
            amount,
            fillet_radius_stations,
            ..
        } if edges.len() == 1
            && edges[0].topological().is_some_and(|edge| edge.lineage_digest == reference_id)
            && amount.millimetres() == 2.0
            && fillet_radius_stations.len() == 2
            && fillet_radius_stations[0].position == 0.5
            && fillet_radius_stations[0].radius.millimetres() == 3.0
            && fillet_radius_stations[1].position == 1.0
            && fillet_radius_stations[1].radius.millimetres() == 1.0
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V17);
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::EdgeFinish {
            fillet_radius_stations,
            ..
        } if fillet_radius_stations.len() == 2
            && f64::from_bits(fillet_radius_stations[0].position_bits) == 0.5
            && f64::from_bits(fillet_radius_stations[0].radius_bits) == 3.0
            && f64::from_bits(fillet_radius_stations[1].position_bits) == 1.0
            && f64::from_bits(fillet_radius_stations[1].radius_bits) == 1.0
    )));
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.volume_mm3 > 0.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-fillet-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(matches!(
        reopened.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::EdgeFinish {
            edges,
            kind: EdgeFinishKind::Fillet,
            fillet_radius_stations,
            ..
        } if edges.len() == 1
            && edges[0].topological().is_some_and(|edge| edge.lineage_digest == reference_id)
            && fillet_radius_stations.len() == 2
            && fillet_radius_stations[0].position == 0.5
            && fillet_radius_stations[0].radius.millimetres() == 3.0
            && fillet_radius_stations[1].position == 1.0
            && fillet_radius_stations[1].radius.millimetres() == 1.0
    ));
    assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(3)).is_ok());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
}

#[test]
fn append_topology_chamfer_is_exact_persistent_and_one_step() {
    let request = "Add a two-distance Chamfer to an existing exact body edge";
    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-chamfer-input.ketchup");
    let fixture = ketchup_app::KetchupApp::new();
    let fixture_snapshot = fixture.document_snapshot();
    persistence::save_atomic(&fixture_path, &fixture_snapshot).unwrap();

    let base_graph =
        ExactBRepGraph::from_snapshot(&fixture_snapshot, DefinitionId(1), FeatureId(2)).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();
    let selected_edge = base_package
        .edge_evidence
        .iter()
        .find(|edge| edge.adjacent_face_ordinals.len() == 2)
        .unwrap();
    let reference_id = base_package
        .topological_references
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Edge)
        .nth(selected_edge.edge_ordinal as usize)
        .unwrap()
        .lineage_digest
        .clone();
    let side_face_reference_id = base_package
        .topological_references
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .nth(selected_edge.adjacent_face_ordinals[0] as usize)
        .unwrap()
        .lineage_digest
        .clone();

    let program_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Assistant chamfer".to_owned(),
            feature: AssistantCadBodyFeature::TopologyChamfer {
                target_feature_id: 2,
                edge_reference_ids: vec![reference_id.clone()],
                distance_mm: 2.0,
                mode: AssistantCadChamferMode::TwoDistance {
                    second_distance_mm: 3.0,
                },
                side_face_reference_ids: vec![side_face_reference_id.clone()],
            },
        }],
    };

    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    let baseline_redo = shell.app().redo_step_count();
    let baseline_occurrences = shell.app().document_snapshot().occurrences().count();

    apply_program(&mut shell, request, &program_request);
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::EdgeFinish {
            target: FeatureId(2),
            edges,
            kind: EdgeFinishKind::Chamfer,
            amount,
            chamfer_mode: ChamferMode::TwoDistance { second_distance },
            chamfer_edge_sides,
            ..
        } if edges.len() == 1
            && edges[0].topological().is_some_and(|edge| edge.lineage_digest == reference_id)
            && amount.millimetres() == 2.0
            && second_distance.millimetres() == 3.0
            && chamfer_edge_sides.len() == 1
            && chamfer_edge_sides[0].side_face.lineage_digest == side_face_reference_id
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(&node.operation, ExactBRepOperation::EdgeFinish { .. }))
    );
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.volume_mm3 > 0.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(committed.occurrences().count(), baseline_occurrences);

    let committed_digest = committed.canonical_digest();
    let saved_path = directory.path().join("assistant-chamfer-result.ketchup");
    persistence::save_atomic(&saved_path, &committed).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(matches!(
        reopened.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::EdgeFinish {
            edges,
            kind: EdgeFinishKind::Chamfer,
            ..
        } if edges.len() == 1 && edges[0].topological().is_some_and(|edge| edge.lineage_digest == reference_id)
    ));
    assert!(ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(3)).is_ok());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo);
    assert_eq!(shell.app().redo_step_count(), baseline_redo + 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().redo_step_count(), baseline_redo);
}

#[test]
fn integrated_finishing_chain_rebuilds_exactly_through_cad_programs() {
    let shell_request = "Create a closed symmetric shell";
    let fillet_request = "Add a variable fillet to the shelled part";
    let invalid_chamfer_request = "Use an edge as the Chamfer orientation face";
    let chamfer_request = "Add an oriented two-distance chamfer after the fillet";
    let program_shell_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Integrated shell".to_owned(),
            feature: AssistantCadBodyFeature::TopologyShell {
                target_feature_id: 2,
                removed_face_reference_ids: Vec::new(),
                thickness_mm: 0.5,
                direction: ketchup_assistant::sidecar::AssistantCadShellDirection::Symmetric,
            },
        }],
    };

    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("integrated-finishing-input.ketchup");
    let fixture = ketchup_app::KetchupApp::new();
    let fixture_snapshot = fixture.document_snapshot();
    persistence::save_atomic(&fixture_path, &fixture_snapshot).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_graph =
        ExactBRepGraph::from_snapshot(&fixture_snapshot, DefinitionId(1), FeatureId(2)).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();

    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    let initial_revision = shell.app().document_revision();
    let initial_undo = shell.app().undo_step_count();

    apply_program(&mut shell, shell_request, &program_shell_request);
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 1);
    let shell_snapshot = shell.app().document_snapshot();
    let shell_graph =
        ExactBRepGraph::from_snapshot(&shell_snapshot, DefinitionId(1), FeatureId(3)).unwrap();
    assert_eq!(
        shell_graph.schema,
        ketchup_model::exact_brep_graph::EXACT_BREP_GRAPH_SCHEMA_V16
    );
    let shell_package = worker.evaluate_exact_brep_graph(&shell_graph).unwrap();
    let fillet_edge_reference_id = shell_package
        .edge_evidence
        .iter()
        .filter(|edge| edge.adjacent_face_ordinals.len() == 2)
        .find_map(|edge| {
            shell_package
                .topological_references
                .iter()
                .filter(|reference| reference.kind == TopologicalElementKind::Edge)
                .nth(edge.edge_ordinal as usize)
                .filter(|reference| {
                    reference.stability == TopologicalReferenceStability::Guaranteed
                })
                .map(|reference| reference.lineage_digest.clone())
        })
        .expect("the closed Shell must expose a guaranteed manifold Fillet edge");
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(shell_package))
    );
    let program_fillet_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Integrated variable fillet".to_owned(),
            feature: AssistantCadBodyFeature::TopologyFillet {
                target_feature_id: 3,
                edge_reference_ids: vec![fillet_edge_reference_id],
                radius_mm: 0.25,
                radius_stations: vec![
                    AssistantCadFilletRadiusStation {
                        position: 0.5,
                        radius_mm: 0.35,
                    },
                    AssistantCadFilletRadiusStation {
                        position: 1.0,
                        radius_mm: 0.2,
                    },
                ],
            },
        }],
    };

    apply_program(&mut shell, fillet_request, &program_fillet_request);
    assert_eq!(shell.app().document_revision(), initial_revision + 2);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 2);
    let fillet_snapshot = shell.app().document_snapshot();
    let fillet_graph =
        ExactBRepGraph::from_snapshot(&fillet_snapshot, DefinitionId(1), FeatureId(4)).unwrap();
    assert_eq!(fillet_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V17);
    let fillet_package = worker.evaluate_exact_brep_graph(&fillet_graph).unwrap();
    let mut chamfer_selection = None;
    'edges: for edge_evidence in &fillet_package.edge_evidence {
        let edge = fillet_package
            .topological_references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Edge)
            .nth(edge_evidence.edge_ordinal as usize)
            .unwrap()
            .clone();
        if edge.stability != TopologicalReferenceStability::Guaranteed {
            continue;
        }
        for face_ordinal in &edge_evidence.adjacent_face_ordinals {
            let side_face = fillet_package
                .topological_references
                .iter()
                .filter(|reference| reference.kind == TopologicalElementKind::Face)
                .nth(*face_ordinal as usize)
                .unwrap()
                .clone();
            if side_face.stability != TopologicalReferenceStability::Guaranteed {
                continue;
            }
            let mut candidate = persistence::load(&persistence::save(&fillet_snapshot))
                .unwrap()
                .into_editable()
                .unwrap_or_else(|_| panic!("schema 73 candidate must remain editable"));
            candidate
                .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                    id: FeatureId(5),
                    definition_id: DefinitionId(1),
                    name: "Chamfer feasibility candidate".to_owned(),
                    kind: FeatureKind::EdgeFinish {
                        target: FeatureId(4),
                        edges: vec![EdgeRef::from(edge.clone())],
                        kind: EdgeFinishKind::Chamfer,
                        amount: Dimension::from_decimal("0.2").unwrap(),
                        fillet_radius_stations: Vec::new(),
                        chamfer_mode: ChamferMode::TwoDistance {
                            second_distance: Dimension::from_decimal("0.3").unwrap(),
                        },
                        chamfer_edge_sides: vec![ChamferEdgeSide {
                            edge: edge.clone(),
                            side_face: side_face.clone(),
                        }],
                    },
                }]))
                .unwrap();
            let candidate_graph =
                ExactBRepGraph::from_snapshot(&candidate.current(), DefinitionId(1), FeatureId(5))
                    .unwrap();
            if worker.evaluate_exact_brep_graph(&candidate_graph).is_ok() {
                chamfer_selection = Some((edge.lineage_digest, side_face.lineage_digest));
                break 'edges;
            }
        }
    }
    let (chamfer_edge_reference_id, chamfer_face_reference_id) = chamfer_selection
        .expect("the integrated exact part must expose a feasible oriented Chamfer edge/face pair");
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(fillet_package))
    );
    let invalid_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Rejected edge-as-face Chamfer".to_owned(),
            feature: AssistantCadBodyFeature::TopologyChamfer {
                target_feature_id: 4,
                edge_reference_ids: vec![chamfer_edge_reference_id.clone()],
                distance_mm: 0.2,
                mode: AssistantCadChamferMode::TwoDistance {
                    second_distance_mm: 0.3,
                },
                side_face_reference_ids: vec![chamfer_edge_reference_id.clone()],
            },
        }],
    };
    let before_rejection_revision = shell.app().document_revision();
    let before_rejection_digest = shell.app().canonical_digest();
    let before_rejection_undo = shell.app().undo_step_count();
    assert!(
        shell
            .app_mut()
            .headless_apply_cad_program(&invalid_program)
            .is_err(),
        "{invalid_chamfer_request}"
    );
    assert_eq!(shell.app().document_revision(), before_rejection_revision);
    assert_eq!(shell.app().canonical_digest(), before_rejection_digest);
    assert_eq!(shell.app().undo_step_count(), before_rejection_undo);

    let program_chamfer_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Integrated two-distance chamfer".to_owned(),
            feature: AssistantCadBodyFeature::TopologyChamfer {
                target_feature_id: 4,
                edge_reference_ids: vec![chamfer_edge_reference_id],
                distance_mm: 0.2,
                mode: AssistantCadChamferMode::TwoDistance {
                    second_distance_mm: 0.3,
                },
                side_face_reference_ids: vec![chamfer_face_reference_id],
            },
        }],
    };

    apply_program(&mut shell, chamfer_request, &program_chamfer_request);
    assert_eq!(shell.app().document_revision(), initial_revision + 3);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 3);
    let finished_snapshot = shell.app().document_snapshot();
    let finished_graph =
        ExactBRepGraph::from_snapshot(&finished_snapshot, DefinitionId(1), FeatureId(5)).unwrap();
    assert_eq!(finished_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    let finished_package = worker.evaluate_exact_brep_graph(&finished_graph).unwrap();
    assert!(finished_package.volume_mm3 > 0.0);
    assert_eq!(finished_package.topology_counts[4], 1);
    let finished_fingerprint = finished_package.identity.result_fingerprint;
    let before_upstream_digest = finished_snapshot.canonical_digest();
    let history = shell.catalog().text("feature-history-title");
    shell.click_role_and_label(Role::Button, &history);
    let upstream_feature = finished_snapshot.feature(FeatureId(2)).unwrap();
    let upstream_label = shell.catalog().format(
        "feature-history-select-feature",
        &BTreeMap::from([
            ("name", upstream_feature.name().to_owned()),
            ("id", "2".to_owned()),
            (
                "status",
                shell.catalog().text("feature-history-state-active"),
            ),
        ]),
    );
    shell.click_role_and_label(Role::Button, &upstream_label);

    apply_program(
        &mut shell,
        "Change the upstream extrusion to 30 mm",
        &AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::SetDimension {
                feature_id: 2,
                constraint_id: None,
                value_mm: 30.0,
            }],
        },
    );
    assert_eq!(shell.app().document_revision(), initial_revision + 4);
    assert_eq!(shell.app().undo_step_count(), initial_undo + 4);
    let rebuilt = shell.app().document_snapshot();
    assert_ne!(rebuilt.canonical_digest(), before_upstream_digest);
    let rebuilt_graph =
        ExactBRepGraph::from_snapshot(&rebuilt, DefinitionId(1), FeatureId(5)).unwrap();
    assert_eq!(rebuilt_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    let rebuilt_package = worker.evaluate_exact_brep_graph(&rebuilt_graph).unwrap();
    assert!(rebuilt_package.volume_mm3 > 0.0);
    assert_eq!(rebuilt_package.topology_counts[4], 1);
    assert_ne!(
        rebuilt_package.identity.result_fingerprint,
        finished_fingerprint
    );

    let rebuilt_digest = rebuilt.canonical_digest();
    let saved_path = directory.path().join("integrated-finishing-result.ketchup");
    persistence::save_atomic(&saved_path, &rebuilt).unwrap();
    let reopened = persistence::load_file(&saved_path).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), rebuilt_digest);
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), FeatureId(5)).unwrap();
    assert_eq!(reopened_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    assert_eq!(
        worker
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap()
            .identity
            .result_fingerprint,
        rebuilt_package.identity.result_fingerprint
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before_upstream_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), rebuilt_digest);
}

#[test]
fn sketch_program_reviews_creates_and_edits_workplanes_entities_and_constraints() {
    let create_principal = "Create a reviewed mixed sketch on the XY workplane";
    let create_offset = "Create a reviewed circle sketch on an offset workplane";
    let edit_dimensions = "Edit the offset workplane and circle radius together";
    let point = |entity_id, point| AssistantSketchPointRef { entity_id, point };
    let program_create_principal = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1.into(),
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
    };
    let mut shell = Shell::new();

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    apply_program(&mut shell, create_principal, &program_create_principal);
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

    let program_create_offset = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1.into(),
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
    };
    apply_program(&mut shell, create_offset, &program_create_offset);
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    assert_eq!(shell.app().undo_step_count(), before_undo + 2);

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
    let program_edit_dimensions = AssistantCadEditProgram {
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
    };
    let before_edit_revision = shell.app().document_revision();
    let before_edit_digest = shell.app().canonical_digest();
    let before_edit_undo = shell.app().undo_step_count();
    apply_program(&mut shell, edit_dimensions, &program_edit_dimensions);

    assert_eq!(shell.app().document_revision(), before_edit_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_edit_undo + 1);
    let edited = shell.app().document_snapshot();
    let FeatureKind::Workplane(workplane) = edited.feature(offset_workplane_id).unwrap().kind()
    else {
        panic!("expected reviewed offset workplane")
    };
    assert!(matches!(
        &workplane.support,
        ketchup_geometry::sketch::WorkplaneSupport::Offset { distance, .. }
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
}

#[test]
fn sketch_program_rejects_invalid_constraint_without_mutation() {
    let invalid_request = "Create a sketch with an invalid constraint reference";
    let invalid_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1.into(),
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
    let mut invalid_shell = Shell::new();
    let before_revision = invalid_shell.app().document_revision();
    let before_digest = invalid_shell.app().canonical_digest();
    let before_undo = invalid_shell.app().undo_step_count();
    let rejection = invalid_shell
        .app_mut()
        .headless_apply_cad_program(&invalid_program)
        .expect_err(invalid_request)
        .to_string();
    assert!(
        rejection.contains("sketch constraint reference is invalid"),
        "{rejection}"
    );
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
}

#[test]
fn surface_program_applies_as_one_undoable_step() {
    let create_request = "Create, extend, and thicken a planar surface";
    let earlier_body = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let program_create_request = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::AppendFeature {
                definition_id: 1.into(),
                name: "Assistant planar surface".into(),
                feature: AssistantCadBodyFeature::SurfaceBody {
                    source: AssistantCadSurfaceBodySource::Planar {
                        profile_feature_id: 1.into(),
                    },
                },
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: 1.into(),
                name: "Assistant extended surface".into(),
                feature: AssistantCadBodyFeature::SurfaceExtend {
                    target_feature_id: earlier_body(0),
                    distance_mm: 2.0,
                },
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: 1.into(),
                name: "Assistant thickened solid".into(),
                feature: AssistantCadBodyFeature::SurfaceThicken {
                    target_feature_id: earlier_body(1),
                    thickness_mm: 1.5,
                    direction: AssistantCadShellDirection::Symmetric,
                },
            },
        ],
    };

    // A profile-only part: the surface workflow becomes the definition's only body.
    let directory = tempfile::tempdir().unwrap();
    let fixture_path = directory.path().join("assistant-surface-input.ketchup");
    write_planar_offset_fixture(&fixture_path);
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture_path));
    shell.settle();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();

    apply_program(&mut shell, create_request, &program_create_request);
    let committed_revision = shell.app().document_revision();
    let committed_digest = shell.app().canonical_digest();
    assert_eq!(committed_revision, baseline_revision + 1);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    let committed = shell.app().document_snapshot();
    assert!(matches!(
        committed.feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::SurfaceBody(_)
    ));
    assert!(matches!(
        committed.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::SurfaceExtend {
            target: FeatureId(2),
            ..
        }
    ));
    assert!(matches!(
        committed.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::SurfaceThicken {
            target: FeatureId(3),
            direction: ketchup_model::document::ShellDirection::Symmetric,
            ..
        }
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
}

#[test]
fn nested_assembly_joint_and_motion_preserve_consent_and_repeated_branch_identity() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("assistant-nested-motion.ketchup");
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(50),
                name: "Mechanism part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(50),
                name: "Mechanism profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(50),
                name: "Mechanism extrusion".into(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::from_decimal("5").unwrap()),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(60),
                name: "Reusable mechanism".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(50),
                definition_id: DefinitionId(50),
                name: "Rail".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(60)),
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(51),
                definition_id: DefinitionId(50),
                name: "Slider".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(GroupId(60)),
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let converted = document
        .convert_group_to_component(GroupId(60), "Reusable mechanism")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: converted.component_definition_id,
                name: "Reusable mechanism copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: converted.component_occurrence_id,
                grounded: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut branch = snapshot
        .scene_query()
        .into_iter()
        .filter(|item| {
            item.instance_path.root_occurrence() == converted.component_occurrence_id
                && !item.instance_path.is_root()
        })
        .collect::<Vec<_>>();
    branch.sort_by(|left, right| {
        left.transform.matrix()[3]
            .partial_cmp(&right.transform.matrix()[3])
            .unwrap()
    });
    let parent_path = branch[0].instance_path.clone();
    let child_path = branch[1].instance_path.clone();
    let child_before = branch[1].transform;
    let twin = snapshot
        .scene_query()
        .into_iter()
        .find(|item| {
            item.instance_path.root_occurrence() == OccurrenceId(80)
                && item.instance_path.steps() == child_path.steps()
        })
        .unwrap();
    let twin_path = twin.instance_path.clone();
    let twin_before = twin.transform;
    let encode_path = |path: &InstancePath| AssistantInstancePath {
        root_occurrence_id: path.root_occurrence().0,
        steps: path
            .steps()
            .iter()
            .map(|step| match *step {
                InstancePathStep::Group(local_id) => AssistantInstancePathStep::Group {
                    owner_definition_id: converted.component_definition_id.0,
                    local_id: local_id.0,
                },
                InstancePathStep::Occurrence(local_id) => AssistantInstancePathStep::Occurrence {
                    owner_definition_id: converted.component_definition_id.0,
                    local_id: local_id.0,
                },
            })
            .collect(),
    };
    let parent_input = encode_path(&parent_path);
    let child_input = encode_path(&child_path);
    drop(snapshot);
    persistence::save_atomic(&fixture, &document.current()).unwrap();

    let create_request = "Create the nested slider joint";
    let move_request = "Move the nested slider by ten millimetres";
    let program_create_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateAssemblyJoint {
            parent_instance_path: parent_input,
            child_instance_path: child_input,
            kind: AssistantAssemblyJointKind::Prismatic {
                axis: AssistantAssemblyJointAxis {
                    direction_in_parent: [1.0, 0.0, 0.0],
                    pivot_in_parent_mm: [0.0; 3],
                },
                limits: Some(AssistantAssemblyJointLimits {
                    min: 0.0,
                    max: 20.0,
                }),
                position_mm: 0.0,
            },
        }],
    };
    let program_move_request = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetAssemblyJointPosition {
            joint_id: 1,
            position: 10.0,
        }],
    };
    let mut shell = Shell::new();
    assert!(shell.app_mut().open_document_path(&fixture));

    let baseline = shell.app().canonical_digest();
    let baseline_undo = shell.app().undo_step_count();
    apply_program(&mut shell, create_request, &program_create_request);
    let joint_digest = shell.app().canonical_digest();
    assert_ne!(joint_digest, baseline);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 1);
    assert_eq!(shell.app().document_snapshot().assembly_joints().count(), 1);

    apply_program(&mut shell, move_request, &program_move_request);
    assert_eq!(shell.app().undo_step_count(), baseline_undo + 2);
    let moved_digest = shell.app().canonical_digest();
    let moved = shell.app().document_snapshot();
    assert_eq!(
        moved
            .resolve_instance_path(&child_path)
            .unwrap()
            .world_transform
            .matrix()[3],
        child_before.matrix()[3] + 10.0
    );
    assert_eq!(
        moved
            .resolve_instance_path(&twin_path)
            .unwrap()
            .world_transform,
        twin_before
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), joint_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), moved_digest);
}
