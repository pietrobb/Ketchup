use std::collections::BTreeSet;

use ketchup_application::model_query::{EntityKind, ModelQuery, PageRequest};
use ketchup_application::{
    AssistantCadResolvedProgramOutput, plan_assistant_cad_edit_program as plan,
    plan_assistant_cad_edit_program_with_outputs as plan_with_outputs,
};
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, PlanarFaceAttachment,
};
use ketchup_core::assistant_sidecar::*;
use ketchup_core::document::{
    BodyKind, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, GroupId, InstancePath, InstancePathStep, OccurrenceId,
    Snapshot, SpatialPathSegment, SurfaceBodySpec, Transform,
};
use ketchup_core::drawing::{DrawingSource, project_orthographic_drawing};
use ketchup_core::drawing_export::export_drawing;
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V14, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::persistence::{ContainerData, LoadOutcome, load, save, save_document_store};
use ketchup_core::sketch::{
    PrincipalPlane, SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};
use std::sync::Arc;

fn part() -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        name: "Editable part".into(),
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
        rotation: None,
    }
}

fn program(operations: Vec<AssistantCadEditOperation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn explicit(id: u64) -> AssistantCadEntitySelector {
    AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![id],
    }
}

fn translate(selector: AssistantCadEntitySelector, delta: [f64; 3]) -> AssistantCadEditOperation {
    AssistantCadEditOperation::Transform {
        selector,
        translation_mm: delta,
        rotation: None,
    }
}

fn seeded() -> DocumentStore {
    let mut document = DocumentStore::new();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![part()]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    document
}

fn cam_setup_operation(definition_id: u64, feature_id: u64) -> AssistantCadEditOperation {
    AssistantCadEditOperation::UpsertCamPlan {
        plan_id: 1,
        name: "Reviewed top setup".into(),
        target_definition_id: definition_id,
        target_feature_id: feature_id,
        stock_minimum_mm: [-15.0, -15.0, -1.0],
        stock_maximum_mm: [15.0, 15.0, 31.0],
        tool_number: 1,
        tool_kind: AssistantCamToolKind::FlatEndMill,
        tool_diameter_mm: 6.0,
        flute_length_mm: 18.0,
        overall_length_mm: 50.0,
        holder_diameter_mm: 20.0,
        holder_length_mm: 35.0,
        spindle_rpm: 12_000,
        feed_mm_per_min: 900.0,
        plunge_mm_per_min: 250.0,
        work_offset: AssistantCamWorkOffset::G54,
        origin_mm: [0.0, 0.0, 31.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        safe_height_mm: 5.0,
        maximum_stepdown_mm: 2.0,
        stepover_ratio: 0.45,
        radial_allowance_mm: 0.2,
        axial_allowance_mm: 0.1,
    }
}

fn assistant_instance_path(snapshot: &Snapshot, path: &InstancePath) -> AssistantInstancePath {
    let mut prefix = InstancePath::root(path.root_occurrence());
    let mut owner_definition_id = snapshot
        .occurrence(path.root_occurrence())
        .unwrap()
        .definition_id();
    let mut steps = Vec::new();
    for step in path.steps() {
        steps.push(match *step {
            InstancePathStep::Group(local_id) => AssistantInstancePathStep::Group {
                owner_definition_id: owner_definition_id.0,
                local_id: local_id.0,
            },
            InstancePathStep::Occurrence(local_id) => AssistantInstancePathStep::Occurrence {
                owner_definition_id: owner_definition_id.0,
                local_id: local_id.0,
            },
        });
        prefix = prefix.with_step(*step);
        owner_definition_id = snapshot
            .resolve_instance_path(&prefix)
            .unwrap()
            .definition_id;
    }
    AssistantInstancePath {
        root_occurrence_id: path.root_occurrence().0,
        steps,
    }
}

fn exact_box_package(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    feature_id: FeatureId,
) -> Arc<ExactBodyPackage> {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, definition_id).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                feature_id,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:public-nested-assembly"),
        )
    });
    Arc::new(
        build_box_render_package(
            &request,
            "public-nested-assembly-input".into(),
            "public-nested-assembly-result".into(),
            "occt".into(),
            "r0".into(),
            [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
            evidence,
        )
        .unwrap()
        .into(),
    )
}

#[test]
fn serializable_create_program_plans_without_gui_and_commits_one_editable_revision() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![part(), part()]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let registry = ExactResultRegistry::default();
    let batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    assert_eq!(batch.commands().len(), 10);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
    assert_eq!(
        batch,
        plan(&document, &BTreeSet::new(), &registry, &input).unwrap()
    );
    let preview = document.preview_batch(&batch).unwrap();
    for (definition, feature) in [(1, 3), (2, 6)] {
        assert!(matches!(
            preview.feature(FeatureId(feature)).unwrap().kind(),
            FeatureKind::Pad(_)
        ));
        assert!(
            ExactBRepGraph::from_snapshot(&preview, DefinitionId(definition), FeatureId(feature))
                .is_ok()
        );
    }
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(committed.definitions().count(), 2);
    assert_eq!(committed.occurrences().count(), 2);
    assert_eq!(committed.features().count(), 6);
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn public_nested_assembly_joint_motion_drawing_round_trip_is_branch_exact() {
    const DEFINITION: DefinitionId = DefinitionId(100);
    const PROFILE: FeatureId = FeatureId(100);
    const EXTRUSION: FeatureId = FeatureId(101);
    const COMPONENT_GROUP: GroupId = GroupId(110);
    const INNER_GROUP: GroupId = GroupId(111);
    const PARENT: OccurrenceId = OccurrenceId(100);
    const CHILD: OccurrenceId = OccurrenceId(101);
    const COPY: OccurrenceId = OccurrenceId(200);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Nested mechanism part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Exact mechanism body".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateGroup {
                id: COMPONENT_GROUP,
                name: "Reusable mechanism".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: INNER_GROUP,
                name: "Nested carriage".into(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: Some(COMPONENT_GROUP),
            },
            CanonicalCommand::CreateOccurrence {
                id: PARENT,
                definition_id: DEFINITION,
                name: "Nested rail".into(),
                transform: Transform::identity(),
                parent: Some(INNER_GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: CHILD,
                definition_id: DEFINITION,
                name: "Nested slider".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER_GROUP),
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let converted = document
        .convert_group_to_component(COMPONENT_GROUP, "Reusable mechanism")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: COPY,
                definition_id: converted.component_definition_id,
                name: "Reusable mechanism copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let mut first_branch = snapshot
        .scene_query()
        .into_iter()
        .filter(|item| {
            item.instance_path.root_occurrence() == converted.component_occurrence_id
                && !item.instance_path.is_root()
        })
        .collect::<Vec<_>>();
    first_branch.sort_by(|left, right| {
        left.transform.matrix()[3]
            .partial_cmp(&right.transform.matrix()[3])
            .unwrap()
    });
    assert_eq!(first_branch.len(), 2);
    let parent_path = first_branch[0].instance_path.clone();
    let child_path = first_branch[1].instance_path.clone();
    assert_eq!(child_path.steps().len(), 2);
    let child_world_before = first_branch[1].transform;
    let twin_parent_path = snapshot
        .scene_query()
        .into_iter()
        .find(|item| {
            item.instance_path.root_occurrence() == COPY
                && item.instance_path.steps() == parent_path.steps()
        })
        .unwrap()
        .instance_path;
    let twin_child = snapshot
        .scene_query()
        .into_iter()
        .find(|item| {
            item.instance_path.root_occurrence() == COPY
                && item.instance_path.steps() == child_path.steps()
        })
        .unwrap();
    let twin_child_path = twin_child.instance_path.clone();
    let twin_world_before = twin_child.transform;
    let reference_package = exact_box_package(&snapshot, DEFINITION, EXTRUSION);
    let top = reference_package
        .reference(ExactFaceRole::Top)
        .unwrap()
        .clone();
    drop(snapshot);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: converted.component_occurrence_id,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: COPY,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(1),
                AssemblyMateEndpoint::resolved_planar_face_at_path(
                    parent_path.clone(),
                    PlanarFaceAttachment::new(top.clone(), [0.0; 3], [0.0, 0.0, 1.0]).unwrap(),
                ),
                AssemblyMateEndpoint::resolved_planar_face_at_path(
                    twin_parent_path,
                    PlanarFaceAttachment::new(top, [0.0; 3], [0.0, 0.0, 1.0]).unwrap(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
        ]))
        .unwrap();
    document.discard_history_before_current();

    let stable_before_invalid = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
    );
    let mut wrong_path = assistant_instance_path(&document.current(), &child_path);
    match &mut wrong_path.steps[0] {
        AssistantInstancePathStep::Group {
            owner_definition_id,
            ..
        }
        | AssistantInstancePathStep::Occurrence {
            owner_definition_id,
            ..
        } => *owner_definition_id += 1,
    }
    let invalid = program(vec![AssistantCadEditOperation::CreateAssemblyJoint {
        parent_instance_path: assistant_instance_path(&document.current(), &parent_path),
        child_instance_path: wrong_path,
        kind: AssistantAssemblyJointKind::Fixed,
    }]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &invalid,
        )
        .is_err()
    );
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
        ),
        stable_before_invalid
    );

    let joint_program = program(vec![AssistantCadEditOperation::CreateAssemblyJoint {
        parent_instance_path: assistant_instance_path(&document.current(), &parent_path),
        child_instance_path: assistant_instance_path(&document.current(), &child_path),
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
    }]);
    let joint_program: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&joint_program).unwrap()).unwrap();
    let before_joint = document.current().canonical_digest();
    let joint_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &joint_program,
    )
    .unwrap();
    assert_eq!(document.current().canonical_digest(), before_joint);
    document.apply_batch(&joint_batch).unwrap();
    assert_eq!(document.current().assembly_joints().count(), 1);
    assert_eq!(
        document
            .current()
            .assembly_joint(ketchup_core::assembly_joint::AssemblyJointId(1))
            .unwrap()
            .child_instance_path(),
        &child_path
    );

    let motion_program = program(vec![AssistantCadEditOperation::SetAssemblyJointPosition {
        joint_id: 1,
        position: 10.0,
    }]);
    let motion_program: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&motion_program).unwrap()).unwrap();
    let before_motion = document.current().canonical_digest();
    let motion_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &motion_program,
    )
    .unwrap();
    assert_eq!(document.current().canonical_digest(), before_motion);
    document.apply_batch(&motion_batch).unwrap();
    let moved_digest = document.current().canonical_digest();
    assert_eq!(
        document
            .current()
            .resolve_instance_path(&child_path)
            .unwrap()
            .world_transform
            .matrix()[3],
        child_world_before.matrix()[3] + 10.0
    );
    assert_eq!(
        document
            .current()
            .resolve_instance_path(&twin_child_path)
            .unwrap()
            .world_transform,
        twin_world_before
    );
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before_motion);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), moved_digest);

    let moved_snapshot = document.current();
    let exact = ExactResultRegistry::accept(
        &moved_snapshot,
        [exact_box_package(&moved_snapshot, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let mut drawing_paths = vec![child_path.clone(), twin_child_path.clone()];
    drawing_paths.sort();
    let drawing_program = program(vec![AssistantCadEditOperation::CreateDrawing {
        name: "Nested slider drawing".into(),
        instance_paths: drawing_paths
            .iter()
            .map(|path| assistant_instance_path(&moved_snapshot, path))
            .collect(),
    }]);
    let drawing_program: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&drawing_program).unwrap()).unwrap();
    drop(moved_snapshot);
    let before_drawing = document.current().canonical_digest();
    let drawing_batch = plan(&document, &BTreeSet::new(), &exact, &drawing_program).unwrap();
    assert_eq!(document.current().canonical_digest(), before_drawing);
    document.apply_batch(&drawing_batch).unwrap();
    let completed = document.current();
    let completed_digest = completed.canonical_digest();
    assert!(matches!(
        completed
            .drawing_sheets()
            .next()
            .unwrap()
            .source(),
        DrawingSource::RigidAssemblyInstances { instance_paths }
            if instance_paths == &drawing_paths
    ));
    let drawing_sheet = completed.drawing_sheets().next().unwrap();
    assert_eq!(drawing_sheet.page().scale().numerator(), 1);
    assert_eq!(drawing_sheet.page().scale().denominator(), 5);
    assert_eq!(drawing_sheet.bom_balloons().len(), 2);
    assert!(
        drawing_sheet
            .bom_balloons()
            .iter()
            .all(|balloon| balloon.position() == 1)
    );
    assert_eq!(
        drawing_sheet
            .bom_balloons()
            .iter()
            .map(|balloon| balloon.instance_path().clone())
            .collect::<Vec<_>>(),
        drawing_paths
    );
    assert_eq!(completed.assembly_mates().count(), 1);
    assert_eq!(completed.assembly_joints().count(), 1);
    let rebuilt_exact = ExactResultRegistry::accept(
        &completed,
        [exact_box_package(&completed, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let drawing = project_orthographic_drawing(&completed, &rebuilt_exact, drawing_sheet).unwrap();
    let exported = export_drawing(&completed, &drawing).unwrap();
    assert!(
        std::str::from_utf8(exported.svg())
            .unwrap()
            .contains("QTY 2")
    );
    assert!(!exported.dxf().is_empty());
    assert!(!exported.pdf().is_empty());

    let reopened = load(&save(&completed)).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), completed_digest);
    assert_eq!(
        reopened
            .resolve_instance_path(&child_path)
            .unwrap()
            .world_transform,
        completed
            .resolve_instance_path(&child_path)
            .unwrap()
            .world_transform
    );
    assert_eq!(reopened.assembly_mates().count(), 1);
    assert_eq!(reopened.assembly_joints().count(), 1);
    assert_eq!(reopened.drawing_sheets().count(), 1);
    let reopened_exact = ExactResultRegistry::accept(
        &reopened,
        [exact_box_package(&reopened, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let reopened_drawing = project_orthographic_drawing(
        &reopened,
        &reopened_exact,
        reopened.drawing_sheets().next().unwrap(),
    )
    .unwrap();
    let reopened_export = export_drawing(&reopened, &reopened_drawing).unwrap();
    assert_eq!(reopened_export.svg(), exported.svg());
    assert_eq!(reopened_export.dxf(), exported.dxf());
    assert_eq!(reopened_export.pdf(), exported.pdf());

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before_drawing);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), completed_digest);
}

#[test]
fn public_cubic_bezier_profile_plans_as_an_exact_editable_part() {
    let document = DocumentStore::new();
    let input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Bezier enclosure".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![
            AssistantSketchEntity::CubicBezier {
                id: 1,
                start_mm: [-20.0, 0.0],
                control_1_mm: [-20.0, 15.0],
                control_2_mm: [20.0, 15.0],
                end_mm: [20.0, 0.0],
            },
            AssistantSketchEntity::CubicBezier {
                id: 2,
                start_mm: [20.0, 0.0],
                control_1_mm: [20.0, -15.0],
                control_2_mm: [-20.0, -15.0],
                end_mm: [-20.0, 0.0],
            },
        ],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned profile must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 2);
    assert!(sketch.entities.iter().all(|entity| matches!(
        entity,
        ketchup_core::sketch::SketchEntity::CubicBezier { .. }
    )));
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn public_large_ellipse_has_a_visible_bounded_deviation_and_reopens_exactly() {
    let mut document = DocumentStore::new();
    let baseline_digest = document.current().canonical_digest();
    let ellipse = |radius_x_mm, radius_y_mm, maximum_deviation_mm| {
        program(vec![AssistantCadEditOperation::CreatePart {
            name: "Bounded ellipse".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Ellipse {
                segment_ids: [1, 2, 3, 4],
                center_mm: [0.0, 0.0],
                radius_x_mm,
                radius_y_mm,
                rotation_degrees: 32.0,
                maximum_deviation_mm,
            }],
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
            translation_mm: [0.0; 3],
            rotation: None,
        }])
    };
    let large = ellipse(200_000.0, 80_000.0, 55.0);
    let serialized = serde_json::to_value(&large).unwrap();
    assert_eq!(
        serialized["operations"][0]["entities"][0]["maximum_deviation_mm"],
        55.0
    );
    assert_eq!(large.validate(), Ok(()));
    let mut too_strict = serialized;
    too_strict["operations"][0]["entities"][0]["maximum_deviation_mm"] = serde_json::json!(54.0);
    let too_strict: AssistantCadEditProgram = serde_json::from_value(too_strict).unwrap();
    assert!(too_strict.validate().is_err());
    assert_eq!(document.current().canonical_digest(), baseline_digest);

    let input = ellipse(24.0, 9.6, 0.007);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned ellipse must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 4);
    let (sin, cos) = 32.0_f64.to_radians().sin_cos();
    let mut maximum_sampled_deviation = 0.0_f64;
    for entity in &sketch.entities {
        let ketchup_core::sketch::SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } = entity
        else {
            panic!("ellipse approximation must remain editable cubic geometry");
        };
        for step in 0..=4_096 {
            let parameter = f64::from(step) / 4_096.0;
            let inverse = 1.0 - parameter;
            let point = [0, 1].map(|axis| {
                inverse.powi(3) * start_mm[axis]
                    + 3.0 * inverse.powi(2) * parameter * control_1_mm[axis]
                    + 3.0 * inverse * parameter.powi(2) * control_2_mm[axis]
                    + parameter.powi(3) * end_mm[axis]
            });
            let local = [
                cos * point[0] + sin * point[1],
                -sin * point[0] + cos * point[1],
            ];
            let normalized_radius = (local[0] / 24.0).hypot(local[1] / 9.6);
            let projected = [local[0] / normalized_radius, local[1] / normalized_radius];
            maximum_sampled_deviation = maximum_sampled_deviation.max(
                (point[0] - (cos * projected[0] - sin * projected[1]))
                    .hypot(point[1] - (sin * projected[0] + cos * projected[1])),
            );
        }
    }
    assert!(maximum_sampled_deviation <= 0.007);
    let large_radius_deviation = maximum_sampled_deviation * (200_000.0 / 24.0);
    assert!(large_radius_deviation > 50.0);
    assert!(large_radius_deviation <= 55.0);
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].entity_ids.len(), 4);
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, baseline_digest);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("bounded ellipse must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
}

#[test]
fn public_rotated_rounded_rectangle_plans_as_an_exact_editable_profile() {
    let document = DocumentStore::new();
    let input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Rotated rounded rectangle".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::RoundedRectangle {
            segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
            center_mm: [-4.0, 6.0],
            width_mm: 48.0,
            height_mm: 28.0,
            corner_radius_mm: 6.0,
            rotation_degrees: 27.0,
        }],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 9.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned rounded rectangle must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 8);
    assert_eq!(
        sketch
            .entities
            .iter()
            .filter(|entity| matches!(entity, ketchup_core::sketch::SketchEntity::Line { .. }))
            .count(),
        4
    );
    assert_eq!(
        sketch
            .entities
            .iter()
            .filter(|entity| matches!(entity, ketchup_core::sketch::SketchEntity::Arc { .. }))
            .count(),
        4
    );
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].entity_ids.len(), 8);
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn public_profile_copies_expand_to_transformed_editable_closed_profiles() {
    let document = seeded();
    let copy =
        |entity_ids, translation_mm, rotation_degrees, uniform_scale| AssistantSketchProfileCopy {
            entity_ids,
            translation_mm,
            rotation_degrees,
            uniform_scale,
        };
    let input = program(vec![AssistantCadEditOperation::CreateSketch {
        definition_id: 1,
        name: "Transformed profile copies".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::ProfileCopies {
            source_entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 20.0,
                height_mm: 12.0,
                corner_radius_mm: 2.0,
                rotation_degrees: 0.0,
            }],
            copies: vec![
                copy((11..=18).collect(), [-18.0, 4.0], 25.0, 1.0),
                copy((21..=28).collect(), [18.0, -5.0], -30.0, 0.7),
            ],
        }],
        constraints: Vec::new(),
    }]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(5)).unwrap().kind() else {
        panic!("profile copies must remain one editable sketch");
    };
    assert_eq!(sketch.entities.len(), 16);
    assert_eq!(sketch.solved_regions().unwrap().len(), 2);
    assert_eq!(
        sketch
            .entities
            .iter()
            .map(|entity| entity.id().0)
            .collect::<Vec<_>>(),
        (11..=18).chain(21..=28).collect::<Vec<_>>()
    );
    assert!(sketch.entities.iter().all(|entity| matches!(
        entity,
        ketchup_core::sketch::SketchEntity::Line { .. }
            | ketchup_core::sketch::SketchEntity::Arc { .. }
    )));

    let exact_document = DocumentStore::new();
    let exact_input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Transformed exact profile".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::ProfileCopies {
            source_entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 20.0,
                height_mm: 12.0,
                corner_radius_mm: 2.0,
                rotation_degrees: 0.0,
            }],
            copies: vec![copy((31..=38).collect(), [7.0, -3.0], 40.0, 1.25)],
        }],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let exact_batch = plan(
        &exact_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &exact_input,
    )
    .unwrap();
    let exact_preview = exact_document.preview_batch(&exact_batch).unwrap();
    assert!(ExactBRepGraph::from_snapshot(&exact_preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn parametric_sketch_outputs_build_a_complex_loft_graph_without_manual_points() {
    let mut document = DocumentStore::new();
    let baseline_digest = document.current().canonical_digest();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let definition = output(0, AssistantCadProgramFeatureOutput::Definition);
    let sketch = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(output(
            operation_index,
            AssistantCadProgramFeatureOutput::SketchFeature,
        ))
    };
    let input = program(vec![
        part(),
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Dimensioned circular section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 13.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 13.0,
            }],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Rounded middle section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [2.0, -1.0, -8.0],
                x_axis: [0.866_025_403_784, 0.5, 0.0],
                y_axis: [-0.5, 0.866_025_403_784, 0.0],
            },
            entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 30.0,
                height_mm: 18.0,
                corner_radius_mm: 4.0,
                rotation_degrees: 18.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Transformed elliptic section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [4.0, 2.0, -15.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.939_692_620_786, 0.342_020_143_326],
            },
            entities: vec![AssistantSketchEntity::ProfileCopies {
                source_entities: vec![AssistantSketchEntity::Ellipse {
                    segment_ids: [1, 2, 3, 4],
                    center_mm: [0.0, 0.0],
                    radius_x_mm: 9.0,
                    radius_y_mm: 6.0,
                    rotation_degrees: 0.0,
                    maximum_deviation_mm: 0.003,
                }],
                copies: vec![AssistantSketchProfileCopy {
                    entity_ids: vec![11, 12, 13, 14],
                    translation_mm: [3.0, -2.0],
                    rotation_degrees: -27.0,
                    uniform_scale: 0.85,
                }],
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Parametric multi-section loft".into(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(1),
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(2),
                        elevation_mm: 28.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(3),
                        elevation_mm: 55.0,
                    },
                ],
                guide_feature_id: None,
                continuity: AssistantCadLoftContinuity::Position,
            },
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let mut holed = input.clone();
    let AssistantCadEditOperation::CreateProgramSketch {
        entities,
        constraints,
        ..
    } = &mut holed.operations[1]
    else {
        unreachable!("the first Loft section is a program sketch")
    };
    *entities = vec![
        AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 13.0,
        },
        AssistantSketchEntity::Circle {
            id: 2,
            center_mm: [0.0, 0.0],
            radius_mm: 4.0,
        },
    ];
    constraints.clear();
    assert!(
        plan(
            &DocumentStore::new(),
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &holed,
        )
        .is_err(),
        "Loft sections with mismatched hole counts must fail closed during public planning"
    );
    for (operation_index, (outer_radius, hole_radius)) in [(2, (11.0, 3.0)), (3, (9.0, 2.0))] {
        let AssistantCadEditOperation::CreateProgramSketch {
            entities,
            constraints,
            ..
        } = &mut holed.operations[operation_index]
        else {
            unreachable!("each Loft section is a program sketch")
        };
        *entities = vec![
            AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: outer_radius,
            },
            AssistantSketchEntity::Circle {
                id: 2,
                center_mm: [0.0, 0.0],
                radius_mm: hole_radius,
            },
        ];
        constraints.clear();
    }
    let input = holed;
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let loft_feature = preview
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::Loft { .. }))
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), loft_feature.id()).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V14);
    assert!(
        graph
            .profiles
            .iter()
            .map(|profile| profile.frame_bits)
            .collect::<BTreeSet<_>>()
            .len()
            == 3
    );
    assert!(matches!(
        graph.nodes[0].operation,
        ExactBRepOperation::Loft { ref sections, .. } if sections.len() == 3
    ));
    assert_eq!(graph.profiles.len(), 3);
    assert!(graph.profiles.iter().all(|profile| matches!(
        &profile.geometry,
        ExactBRepPlanarGeometry::Region { holes, .. } if holes.len() == 1
    )));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, baseline_digest);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("framed Loft must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let reopened = document.current();
    let reopened_loft = reopened
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::Loft { .. }))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), reopened_loft.id())
            .unwrap()
            .schema,
        EXACT_BREP_GRAPH_SCHEMA_V14
    );
}

#[test]
fn public_helix_and_thread_program_is_serializable_atomic_and_exact() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![
        AssistantCadEditOperation::CreateHelix {
            name: "Arbitrary axis helix".into(),
            parameters: AssistantHelixParameters {
                axis: AssistantAxisSpec::TwoPoints {
                    start_mm: [4.0, -3.0, 2.0],
                    end_mm: [5.0, -1.0, 5.0],
                },
                radius_mm: 7.0,
                pitch_mm: 4.5,
                turns: 2.25,
                start_angle_degrees: 37.0,
                handedness: AssistantHelixHandedness::Left,
            },
        },
        AssistantCadEditOperation::CreateThread {
            name: "Arbitrary axis thread".into(),
            parameters: AssistantThreadParameters {
                helix: AssistantHelixParameters {
                    axis: AssistantAxisSpec::OriginDirection {
                        origin_mm: [28.0, 0.0, 0.0],
                        direction: [0.35, 0.2, 1.0],
                    },
                    radius_mm: 8.0,
                    pitch_mm: 6.0,
                    turns: 2.0,
                    start_angle_degrees: 15.0,
                    handedness: AssistantHelixHandedness::Right,
                },
                profile_radius_mm: 0.65,
                profile: AssistantThreadProfile::V,
            },
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 10);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 2);
    assert_eq!(candidate.occurrences().count(), 2);
    assert_eq!(candidate.features().count(), 6);
    let path_lengths = candidate
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::SpatialPath { segments } => Some(segments.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(path_lengths, vec![9, 8]);
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(3)).unwrap();
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(2), FeatureId(6)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn helix_path_is_a_persistent_non_solid_construction_feature() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Construction helix".into(),
        parameters: AssistantHelixParameters {
            axis: AssistantAxisSpec::OriginDirection {
                origin_mm: [4.0, -3.0, 2.0],
                direction: [1.0, 2.0, 3.0],
            },
            radius_mm: 7.0,
            pitch_mm: 4.5,
            turns: 2.25,
            start_angle_degrees: 37.0,
            handedness: AssistantHelixHandedness::Left,
        },
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments.len() == 9
    ));
    assert!(!candidate.features().any(|feature| matches!(
        feature.kind(),
        FeatureKind::Sweep { .. } | FeatureKind::SegmentProfile { .. }
    )));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction path must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments.len() == 9
    ));
}

#[test]
fn helix_resolves_existing_and_same_program_construction_axes() {
    let parameters = |axis| AssistantHelixParameters {
        axis,
        radius_mm: 7.0,
        pitch_mm: 4.5,
        turns: 2.25,
        start_angle_degrees: 37.0,
        handedness: AssistantHelixHandedness::Left,
    };
    let construction_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let same_program = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Referenced axis".into(),
            origin_mm: [4.0, -3.0, 2.0],
            direction: [1.0, 2.0, 3.0],
        },
        AssistantCadEditOperation::CreateHelixPath {
            name: "Referenced Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
        },
    ]);
    let same_program: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&same_program).unwrap()).unwrap();
    let empty = DocumentStore::new();
    let batch = plan(
        &empty,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &same_program,
    )
    .unwrap();
    let candidate = empty.preview_batch(&batch).unwrap();
    let FeatureKind::SpatialPath {
        segments: same_program_segments,
    } = candidate.feature(FeatureId(2)).unwrap().kind()
    else {
        panic!("referenced axis must produce a spatial Helix path");
    };

    let mut existing = DocumentStore::new();
    let axis_program = program(vec![AssistantCadEditOperation::CreateConstructionAxis {
        name: "Existing axis".into(),
        origin_mm: [4.0, -3.0, 2.0],
        direction: [1.0, 2.0, 3.0],
    }]);
    let axis_batch = plan(
        &existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &axis_program,
    )
    .unwrap();
    existing.apply_batch(&axis_batch).unwrap();
    let existing_program = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::Existing(1),
        }),
    }]);
    let existing_batch = plan(
        &existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &existing_program,
    )
    .unwrap();
    let existing_candidate = existing.preview_batch(&existing_batch).unwrap();
    let FeatureKind::SpatialPath {
        segments: existing_segments,
    } = existing_candidate.feature(FeatureId(2)).unwrap().kind()
    else {
        panic!("existing axis must produce a spatial Helix path");
    };
    assert_eq!(same_program_segments, existing_segments);
    assert_eq!(
        same_program_segments,
        &parameters(AssistantAxisSpec::OriginDirection {
            origin_mm: [4.0, -3.0, 2.0],
            direction: [1.0, 2.0, 3.0],
        })
        .spatial_path_segments()
        .unwrap()
    );

    let wrong_kind = program(vec![
        AssistantCadEditOperation::CreateConstructionPoint {
            name: "Not an axis".into(),
            position_mm: [0.0; 3],
        },
        AssistantCadEditOperation::CreateHelixPath {
            name: "Invalid Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
        },
    ]);
    assert!(wrong_kind.validate().is_err());

    let mut wrong_existing = DocumentStore::new();
    let point_program = program(vec![AssistantCadEditOperation::CreateConstructionPoint {
        name: "Existing point".into(),
        position_mm: [0.0; 3],
    }]);
    let point_batch = plan(
        &wrong_existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &point_program,
    )
    .unwrap();
    wrong_existing.apply_batch(&point_batch).unwrap();
    let invalid_existing = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Invalid existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::Existing(1),
        }),
    }]);
    assert!(
        plan(
            &wrong_existing,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &invalid_existing,
        )
        .is_err()
    );
}

#[test]
fn revolve_resolves_shared_world_axis_in_arbitrary_workplane() {
    let workplane = AssistantWorkplaneSpec::Frame {
        origin_mm: [3.0, -2.0, 5.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
    };
    let revolve_part = |axis| AssistantCadEditOperation::CreatePart {
        name: "Arbitrary-axis revolve".into(),
        workplane: workplane.clone(),
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [5.0, 0.0],
            radius_mm: 1.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 1.0,
        }],
        feature: AssistantCadPartFeature::Revolve {
            axis,
            angle_degrees: 270.0,
        },
        translation_mm: [0.0; 3],
        rotation: None,
    };
    let direct = program(vec![revolve_part(AssistantAxisSpec::TwoPoints {
        start_mm: [3.0, -2.0, 5.0],
        end_mm: [3.0, -2.0, 8.0],
    })]);
    let direct_batch = plan(
        &DocumentStore::new(),
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &direct,
    )
    .unwrap();
    let direct_candidate = DocumentStore::new().preview_batch(&direct_batch).unwrap();
    let FeatureKind::Revolve {
        axis_start_mm: direct_start,
        axis_end_mm: direct_end,
        ..
    } = direct_candidate.feature(FeatureId(3)).unwrap().kind()
    else {
        panic!("shared 3D axis must create Revolve");
    };
    assert_eq!((*direct_start, *direct_end), ([0.0, 0.0], [0.0, 1.0]));
    assert!(
        ExactBRepGraph::from_snapshot(&direct_candidate, DefinitionId(1), FeatureId(3)).is_ok()
    );

    let axis_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let referenced = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Referenced revolve axis".into(),
            origin_mm: [3.0, -2.0, 5.0],
            direction: [0.0, 0.0, 3.0],
        },
        revolve_part(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::ProgramOutput(axis_output),
        }),
    ]);
    let referenced: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&referenced).unwrap()).unwrap();
    let referenced_document = DocumentStore::new();
    let referenced_batch = plan(
        &referenced_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &referenced,
    )
    .unwrap();
    let referenced_candidate = referenced_document
        .preview_batch(&referenced_batch)
        .unwrap();
    let FeatureKind::Revolve {
        axis_start_mm: referenced_start,
        axis_end_mm: referenced_end,
        ..
    } = referenced_candidate.feature(FeatureId(4)).unwrap().kind()
    else {
        panic!("referenced construction axis must create Revolve");
    };
    assert_eq!(
        (*referenced_start, *referenced_end),
        (*direct_start, *direct_end)
    );

    let off_plane = program(vec![revolve_part(AssistantAxisSpec::OriginDirection {
        origin_mm: [4.0, -2.0, 5.0],
        direction: [0.0, 0.0, 1.0],
    })]);
    assert!(
        plan(
            &DocumentStore::new(),
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &off_plane,
        )
        .is_err()
    );
}

#[test]
fn arbitrary_spatial_path_is_a_persistent_non_solid_3d_curve() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateSpatialPath {
        name: "Mixed construction curve".into(),
        segments: vec![
            AssistantSpatialPathSegment::Line {
                start_mm: [0.0, 0.0, 0.0],
                end_mm: [10.0, 0.0, 0.0],
            },
            AssistantSpatialPathSegment::CircularArc {
                start_mm: [10.0, 0.0, 0.0],
                end_mm: [20.0, 10.0, 0.0],
                center_mm: [10.0, 10.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                clockwise: false,
            },
            AssistantSpatialPathSegment::CubicBezier {
                start_mm: [20.0, 10.0, 0.0],
                control_1_mm: [20.0, 15.0, 0.0],
                control_2_mm: [20.0, 20.0, 5.0],
                end_mm: [25.0, 25.0, 10.0],
            },
        ],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments }
            if matches!(
                segments.as_slice(),
                [
                    SpatialPathSegment::Line { .. },
                    SpatialPathSegment::CircularArc { .. },
                    SpatialPathSegment::CubicBezier { .. }
                ]
            )
    ));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native 3D construction curve must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
}

#[test]
fn construction_point_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionPoint {
        name: "Datum point".into(),
        position_mm: [12.5, -3.25, 7.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPoint { position_mm }
            if *position_mm == [12.5, -3.25, 7.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionPoint");
    assert_eq!(detail["item"]["construction"]["type"], "point");
    assert_eq!(
        detail["item"]["construction"]["position_mm"],
        serde_json::json!([12.5, -3.25, 7.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction point must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPoint { position_mm }
            if *position_mm == [12.5, -3.25, 7.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionPoint {
            name: "Invalid point".into(),
            position_mm: [f64::NAN, 0.0, 0.0],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn construction_axis_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionAxis {
        name: "Datum axis".into(),
        origin_mm: [4.0, -2.0, 9.5],
        direction: [1.0, 2.0, 3.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction
        } if *origin_mm == [4.0, -2.0, 9.5] && *direction == [1.0, 2.0, 3.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionAxis");
    assert_eq!(detail["item"]["construction"]["type"], "axis");
    assert_eq!(
        detail["item"]["construction"]["origin_mm"],
        serde_json::json!([4.0, -2.0, 9.5])
    );
    assert_eq!(
        detail["item"]["construction"]["direction"],
        serde_json::json!([1.0, 2.0, 3.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction axis must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction
        } if *origin_mm == [4.0, -2.0, 9.5] && *direction == [1.0, 2.0, 3.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionAxis {
            name: "Invalid axis".into(),
            origin_mm: [0.0; 3],
            direction: [0.0; 3],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn construction_plane_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionPlane {
        name: "Datum plane".into(),
        origin_mm: [4.0, -2.0, 9.5],
        normal: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } if *origin_mm == [4.0, -2.0, 9.5]
            && *normal == [0.0, 0.0, 1.0]
            && *x_direction == [1.0, 0.0, 0.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionPlane");
    assert_eq!(detail["item"]["construction"]["type"], "plane");
    assert_eq!(
        detail["item"]["construction"]["origin_mm"],
        serde_json::json!([4.0, -2.0, 9.5])
    );
    assert_eq!(
        detail["item"]["construction"]["normal"],
        serde_json::json!([0.0, 0.0, 1.0])
    );
    assert_eq!(
        detail["item"]["construction"]["x_direction"],
        serde_json::json!([1.0, 0.0, 0.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction plane must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } if *origin_mm == [4.0, -2.0, 9.5]
            && *normal == [0.0, 0.0, 1.0]
            && *x_direction == [1.0, 0.0, 0.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionPlane {
            name: "Invalid plane".into(),
            origin_mm: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            x_direction: [0.0, 0.0, 1.0],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn program_sketch_persistently_references_typed_construction_plane_output() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let plane_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let definition_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::Definition,
    };
    let input = program(vec![
        AssistantCadEditOperation::CreateConstructionPlane {
            name: "Oblique datum plane".into(),
            origin_mm: [4.0, -2.0, 9.5],
            normal: [0.0, 0.0, 2.0],
            x_direction: [3.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: definition_output,
            name: "Referenced plane sketch".into(),
            workplane: AssistantWorkplaneSpec::ConstructionPlane {
                plane: AssistantCadFeatureReference::ProgramOutput(plane_output),
            },
            entities: vec![AssistantSketchEntity::Line {
                id: 1,
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            }],
            constraints: Vec::new(),
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 5);
    let candidate = document.preview_batch(&batch).unwrap();
    let FeatureKind::Workplane(workplane) = candidate.feature(FeatureId(2)).unwrap().kind() else {
        panic!("downstream sketch must own a workplane");
    };
    assert!(matches!(
        workplane.support,
        ketchup_core::sketch::WorkplaneSupport::ConstructionPlane {
            feature: FeatureId(1)
        }
    ));
    assert_eq!(workplane.frame.origin_mm, [4.0, -2.0, 9.5]);
    assert_eq!(workplane.frame.x_axis, [1.0, 0.0, 0.0]);
    assert_eq!(workplane.frame.y_axis, [0.0, 1.0, 0.0]);
    assert_eq!(workplane.frame.normal, [0.0, 0.0, 1.0]);
    assert_eq!(
        candidate
            .feature(FeatureId(2))
            .unwrap()
            .kind()
            .dependencies(),
        [FeatureId(1)].into_iter().collect()
    );
    assert!(!candidate.features().any(|feature| matches!(
        feature.kind(),
        FeatureKind::Pad(_) | FeatureKind::Sweep { .. }
    )));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(
        document
            .preview_batch(&CommandBatch::new(vec![CanonicalCommand::DeleteFeature {
                id: FeatureId(1),
            }]))
            .is_err(),
        "the referenced construction plane must not be deletable"
    );

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("construction-plane sketch reference must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::Workplane(ketchup_core::sketch::WorkplaneSpec {
            support: ketchup_core::sketch::WorkplaneSupport::ConstructionPlane {
                feature: FeatureId(1)
            },
            ..
        })
    ));

    let invalid = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Not a plane".into(),
            origin_mm: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: definition_output,
            name: "Invalid referenced sketch".into(),
            workplane: AssistantWorkplaneSpec::ConstructionPlane {
                plane: AssistantCadFeatureReference::ProgramOutput(plane_output),
            },
            entities: vec![AssistantSketchEntity::Line {
                id: 1,
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            }],
            constraints: Vec::new(),
        },
    ]);
    assert!(invalid.validate().is_err());
}

#[test]
fn same_program_boolean_resolves_host_assigned_body_outputs_atomically() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boolean chain".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Target profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Target body".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "First opening".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 2.0], [12.0, 2.0], [12.0, 12.0], [2.0, 12.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Second opening".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[20.0, 10.0], [30.0, 10.0], [30.0, 20.0], [20.0, 20.0]],
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let earlier_body = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let input = program(vec![
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "First pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2,
                profile_feature_id: 3,
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Second pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2,
                profile_feature_id: 4,
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Chained Boolean".into(),
            feature: AssistantCadBodyFeature::Boolean {
                operation: AssistantCadBooleanOperation::Intersect,
                target_feature_id: earlier_body(0),
                tool_feature_id: earlier_body(1),
            },
        },
    ]);

    let mut guessed_output = input.clone();
    let AssistantCadEditOperation::AppendFeature {
        feature: AssistantCadBodyFeature::Boolean {
            target_feature_id, ..
        },
        ..
    } = &mut guessed_output.operations[2]
    else {
        unreachable!("the third operation is the chained Boolean")
    };
    *target_feature_id = 5.into();
    let rejection = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &guessed_output,
    )
    .unwrap_err();
    assert_eq!(rejection.code, "canonical.feature_not_found");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);
    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(7)).unwrap().kind(),
        FeatureKind::Boolean {
            target: FeatureId(5),
            tool: FeatureId(6),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(7)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn same_program_set_dimension_updates_existing_boolean_inputs_atomically() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Dimensioned Boolean".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Adjustable body".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Raised body".into(),
                kind: FeatureKind::RigidTransform {
                    target: FeatureId(2),
                    transform: Transform::from_translation(0.0, 0.0, 15.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let intersect = AssistantCadEditOperation::AppendFeature {
        definition_id: 1,
        name: "Overlap".into(),
        feature: AssistantCadBodyFeature::Boolean {
            operation: AssistantCadBooleanOperation::Intersect,
            target_feature_id: 2.into(),
            tool_feature_id: 3.into(),
        },
    };

    let rejection = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![intersect.clone()]),
    )
    .unwrap_err();
    assert_eq!(rejection.code, "planning.cad_feature_result_empty");

    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            AssistantCadEditOperation::SetDimension {
                feature_id: 2,
                constraint_id: None,
                value_mm: 20.0,
            },
            intersect,
        ]),
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 2);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    assert!(matches!(
        candidate.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: ketchup_core::document::BooleanOperation::Intersect,
            target: FeatureId(2),
            tool: FeatureId(3),
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(4)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn create_part_sketch_and_pocket_resolve_typed_program_outputs_atomically() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let input = program(vec![
        part(),
        AssistantCadEditOperation::CreateProgramSketch {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Opening profile".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 4.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendProgramPocket {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Opening".into(),
            target_feature: output(0, AssistantCadProgramFeatureOutput::BodyFeature),
            profile_feature: output(1, AssistantCadProgramFeatureOutput::SketchFeature),
            depth_mm: 10.0,
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 8);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(6)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(3),
            profile: FeatureId(5),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(6)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn one_part_accepts_chained_pockets_from_opposed_workplanes() {
    let document = DocumentStore::new();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let input = program(vec![
        AssistantCadEditOperation::CreatePart {
            name: "Panel".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![
                AssistantSketchEntity::Line {
                    id: 1,
                    start_mm: [0.0, 0.0],
                    end_mm: [100.0, 0.0],
                },
                AssistantSketchEntity::Line {
                    id: 2,
                    start_mm: [100.0, 0.0],
                    end_mm: [100.0, 50.0],
                },
                AssistantSketchEntity::Line {
                    id: 3,
                    start_mm: [100.0, 50.0],
                    end_mm: [0.0, 50.0],
                },
                AssistantSketchEntity::Line {
                    id: 4,
                    start_mm: [0.0, 50.0],
                    end_mm: [0.0, 0.0],
                },
            ],
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 18.0 },
            translation_mm: [0.0, 0.0, 0.0],
            rotation: None,
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Left holes".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [5.0, 5.0],
                radius_mm: 2.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendProgramPocket {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Left pocket".into(),
            target_feature: output(0, AssistantCadProgramFeatureOutput::BodyFeature),
            profile_feature: output(1, AssistantCadProgramFeatureOutput::SketchFeature),
            depth_mm: 4.0,
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Right holes".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [100.0, 0.0, 18.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, -1.0],
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 2,
                center_mm: [5.0, 5.0],
                radius_mm: 2.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendProgramPocket {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Right pocket".into(),
            target_feature: output(2, AssistantCadProgramFeatureOutput::BodyFeature),
            profile_feature: output(3, AssistantCadProgramFeatureOutput::SketchFeature),
            depth_mm: 4.0,
        },
    ]);

    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(9)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(6),
            profile: FeatureId(8),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(9)).unwrap();
}

#[test]
fn one_panel_operation_creates_named_physical_holes_and_one_local_edit_moves_one_hole() {
    let mut document = DocumentStore::new();
    let panel = AssistantCadEditOperation::CreatePanel {
        name: "Side panel".into(),
        dimensions_mm: [100.0, 50.0, 18.0],
        holes: vec![AssistantPanelHole {
            id: "dowel-1".into(),
            entry_local_mm: [20.0, 20.0, 0.0],
            inward_unit_local: [0.0, 0.0, 1.0],
            diameter_mm: 8.0,
            depth_mm: 16.0,
        }],
        translation_mm: [0.0, 0.0, 0.0],
        rotation: None,
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![panel]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let before_move = document.current();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(before_move.features().count(), 6);
    assert_eq!(
        before_move.feature(FeatureId(5)).unwrap().name(),
        "Side panel hole dowel-1"
    );
    assert_eq!(
        before_move.feature(FeatureId(6)).unwrap().name(),
        "Side panel hole dowel-1 pocket"
    );
    ExactBRepGraph::from_snapshot(&before_move, DefinitionId(1), FeatureId(6)).unwrap();

    let move_hole = AssistantCadEditOperation::SetFeatureParameter {
        feature_id: 5,
        parameter_path: "entities.1.center.x".into(),
        value_type: AssistantCadParameterValueType::Length,
        value: 20.0,
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![move_hole]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let after_move = document.current();
    assert_eq!(document.visible_undo_steps(), 2);
    assert_eq!(after_move.features().count(), 6);
    assert!(matches!(
        after_move.feature(FeatureId(6)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(3),
            profile: FeatureId(5),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&after_move, DefinitionId(1), FeatureId(6)).unwrap();
}

#[test]
fn one_dowel_joint_operation_derives_matching_sixteen_millimetre_holes_for_both_panels() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm| AssistantCadEditOperation::CreatePanel {
        name: name.into(),
        dimensions_mm: [100.0, 50.0, 18.0],
        holes: Vec::new(),
        translation_mm,
        rotation: None,
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Lower panel", [0.0, 0.0, 0.0]),
            panel("Upper panel", [0.0, 0.0, 18.0]),
        ]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();

    let face = |occurrence_id, face_z, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::CreateDowelJoint {
            name: "Shelf row".into(),
            first: face(1, 18.0, [0.0, 0.0, -1.0]),
            second: face(2, 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 25.0,
            dowel: AssistantStandardDowel::D8x30,
            physical_hole_pairs: Vec::new(),
        }]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    let projection = ketchup_core::joinery::project_dowel_joint_contract(
        &snapshot,
        snapshot
            .dowel_joint(ketchup_core::joinery::DowelJointId(1))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(projection.pairs.len(), 3);
    assert!(projection.pairs.iter().all(|pair| {
        pair.first.depth_mm == 16.0
            && pair.second.depth_mm == 16.0
            && pair.first.shared_center_world_mm == pair.second.shared_center_world_mm
    }));
    assert_eq!(document.visible_undo_steps(), 2);
}

#[test]
fn one_physical_dowel_joint_operation_creates_both_hole_rows_atomically() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm| AssistantCadEditOperation::CreatePanel {
        name: name.into(),
        dimensions_mm: [100.0, 50.0, 18.0],
        holes: Vec::new(),
        translation_mm,
        rotation: None,
    };
    let panels = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Lower panel", [0.0, 0.0, 0.0]),
            panel("Upper panel", [0.0, 0.0, 18.0]),
        ]),
    )
    .unwrap();
    document.apply_batch(&panels).unwrap();
    let baseline = document.current();
    let undo_before = document.visible_undo_steps();
    let face = |occurrence_id, face_z, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let input = program(vec![AssistantCadEditOperation::CreatePhysicalDowelJoint {
        joint_id: None,
        name: "Physical shelf row".into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, [0.0, 0.0, 1.0]),
        first_center_local_mm: [20.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 3,
        spacing_mm: 25.0,
        dowel: AssistantStandardDowel::D8x30,
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pocket { .. },
                    ..
                }
            ))
            .count(),
        6
    );
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::UpsertDowelJoint(_)))
            .count(),
        1
    );

    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let joint = committed
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap();
    let bindings = joint.physical_hole_pairs.as_ref().unwrap();
    assert_eq!(bindings.len(), 3);
    assert!(bindings.iter().all(|binding| {
        matches!(
            committed
                .feature(binding.first_pocket_feature_id)
                .unwrap()
                .kind(),
            FeatureKind::Pocket { .. }
        ) && matches!(
            committed
                .feature(binding.second_pocket_feature_id)
                .unwrap()
                .kind(),
            FeatureKind::Pocket { .. }
        )
    }));
    let projection =
        ketchup_core::joinery::project_dowel_joint_contract(&committed, joint).unwrap();
    assert_eq!(projection.pairs.len(), 3);
    assert!(projection.pairs.iter().all(|pair| {
        pair.physical_probe_coincidence
            .as_ref()
            .is_some_and(|probe| probe.maximum_endpoint_error_mm == 0.0)
    }));

    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let update = program(vec![AssistantCadEditOperation::CreatePhysicalDowelJoint {
        joint_id: Some(1),
        name: "Physical shelf row".into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, [0.0, 0.0, 1.0]),
        first_center_local_mm: [25.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 2,
        spacing_mm: 30.0,
        dowel: AssistantStandardDowel::D8x30,
    }]);
    let update_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &update,
    )
    .unwrap();
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::DeleteDowelJoint { .. }))
            .count(),
        1
    );
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::DeleteFeature { .. }))
            .count(),
        18
    );
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pocket { .. },
                    ..
                }
            ))
            .count(),
        4
    );
    let undo_before_update = document.visible_undo_steps();
    document.apply_batch(&update_batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before_update + 1);
    let updated = document.current();
    assert!(
        baseline
            .features()
            .all(|feature| updated.feature(feature.id()).is_some())
    );
    let updated_joint = updated
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap();
    assert_eq!(updated_joint.physical_hole_pairs.as_ref().unwrap().len(), 2);
    let updated_projection =
        ketchup_core::joinery::project_dowel_joint_contract(&updated, updated_joint).unwrap();
    assert!(updated_projection.pairs.iter().all(|pair| {
        pair.physical_probe_coincidence
            .as_ref()
            .is_some_and(|probe| probe.maximum_endpoint_error_mm == 0.0)
    }));

    let revision_before_repeat = updated.revision_id();
    let undo_before_repeat = document.visible_undo_steps();
    let repeated = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &update,
    )
    .unwrap();
    assert!(repeated.commands().is_empty());
    assert_eq!(document.current().revision_id(), revision_before_repeat);
    assert_eq!(document.visible_undo_steps(), undo_before_repeat);

    let delete_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::DeletePhysicalDowelJoint {
            joint_id: 1,
        }]),
    )
    .unwrap();
    document.apply_batch(&delete_batch).unwrap();
    let deleted = document.current();
    assert!(
        deleted
            .dowel_joint(ketchup_core::joinery::DowelJointId(1))
            .is_none()
    );
    assert_eq!(deleted.features().count(), baseline.features().count());
    assert!(
        baseline
            .features()
            .all(|feature| deleted.feature(feature.id()).is_some())
    );

    document.undo().unwrap();
    let restored = document.current();
    let owned_pocket = restored
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap()
        .physical_hole_pairs
        .as_ref()
        .unwrap()[0]
        .first_pocket_feature_id;
    let definition_id = restored.feature(owned_pocket).unwrap().definition_id();
    let manual_feature_id = FeatureId(
        restored
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap()
            + 1,
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: manual_feature_id,
            definition_id,
            name: "Manual downstream feature".into(),
            kind: FeatureKind::RigidTransform {
                target: owned_pocket,
                transform: Transform::identity(),
            },
        }]))
        .unwrap();
    let guarded = document.current();
    let guarded_digest = guarded.canonical_digest();
    let guarded_undo = document.visible_undo_steps();
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![AssistantCadEditOperation::DeletePhysicalDowelJoint {
                joint_id: 1,
            }]),
        )
        .is_err()
    );
    assert_eq!(document.current().canonical_digest(), guarded_digest);
    assert_eq!(document.visible_undo_steps(), guarded_undo);
}

#[test]
fn physical_dowel_joint_geometry_regressions_fail_closed_without_mutation() {
    let seed = |lower_holes: Vec<AssistantPanelHole>, upper_z: f64| {
        let mut document = DocumentStore::new();
        let panel = |name: &str, translation_mm, holes| AssistantCadEditOperation::CreatePanel {
            name: name.into(),
            dimensions_mm: [100.0, 50.0, 18.0],
            holes,
            translation_mm,
            rotation: None,
        };
        let batch = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![
                panel("Lower panel", [0.0, 0.0, 0.0], lower_holes),
                panel("Upper panel", [0.0, 0.0, upper_z], Vec::new()),
            ]),
        )
        .unwrap();
        document.apply_batch(&batch).unwrap();
        document
    };
    let face = |occurrence_id, face_z, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let joint = |second_normal, count, spacing_mm, dowel| {
        AssistantCadEditOperation::CreatePhysicalDowelJoint {
            joint_id: None,
            name: "Guarded physical row".into(),
            first: face(1, 18.0, [0.0, 0.0, -1.0]),
            second: face(2, 0.0, second_normal),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count,
            spacing_mm,
            dowel,
        }
    };
    let assert_rejected_without_mutation =
        |document: &DocumentStore, operation: AssistantCadEditOperation| {
            let before = document.current();
            let undo = document.visible_undo_steps();
            assert!(
                plan(
                    document,
                    &BTreeSet::new(),
                    &ExactResultRegistry::default(),
                    &program(vec![operation]),
                )
                .is_err()
            );
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            assert_eq!(document.visible_undo_steps(), undo);
        };

    let clean = seed(Vec::new(), 18.0);
    assert_rejected_without_mutation(
        &clean,
        joint([0.0, 0.0, -1.0], 2, 25.0, AssistantStandardDowel::D8x30),
    );
    assert_rejected_without_mutation(
        &clean,
        joint([0.0, 0.0, 1.0], 2, 7.0, AssistantStandardDowel::D8x30),
    );
    assert_rejected_without_mutation(
        &clean,
        joint([0.0, 0.0, 1.0], 1, 0.0, AssistantStandardDowel::D8x40),
    );

    let separated = seed(Vec::new(), 20.0);
    assert_rejected_without_mutation(
        &separated,
        joint([0.0, 0.0, 1.0], 1, 0.0, AssistantStandardDowel::D8x30),
    );

    let hardware = AssistantPanelHole {
        id: "hinge-hardware".into(),
        entry_local_mm: [20.0, 20.0, 18.0],
        inward_unit_local: [0.0, 0.0, -1.0],
        diameter_mm: 10.0,
        depth_mm: 10.0,
    };
    let obstructed = seed(vec![hardware], 18.0);
    assert_rejected_without_mutation(
        &obstructed,
        joint([0.0, 0.0, 1.0], 1, 0.0, AssistantStandardDowel::D8x30),
    );

    let mut shallow = seed(Vec::new(), 18.0);
    let create = plan(
        &shallow,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![joint(
            [0.0, 0.0, 1.0],
            1,
            0.0,
            AssistantStandardDowel::D8x30,
        )]),
    )
    .unwrap();
    shallow.apply_batch(&create).unwrap();
    let committed = shallow.current();
    let first_pocket = committed
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap()
        .physical_hole_pairs
        .as_ref()
        .unwrap()[0]
        .first_pocket_feature_id;
    let revision_before_shallow = committed.revision_id();
    let undo_before_shallow = shallow.visible_undo_steps();
    let make_shallow = plan(
        &shallow,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::SetFeatureParameter {
            feature_id: first_pocket.0,
            parameter_path: "depth".into(),
            value_type: AssistantCadParameterValueType::Length,
            value: 15.0,
        }]),
    )
    .unwrap();
    assert!(shallow.apply_batch(&make_shallow).is_err());
    assert_eq!(shallow.current().revision_id(), revision_before_shallow);
    assert_eq!(shallow.visible_undo_steps(), undo_before_shallow);
}

#[test]
fn physical_dowel_joint_refuses_shared_root_and_nested_definitions_without_mutation() {
    for nested in [false, true] {
        for shared_id in [1, 2] {
            let mut document = DocumentStore::new();
            let panels = plan(
                &document,
                &BTreeSet::new(),
                &ExactResultRegistry::default(),
                &program(
                    vec![0.0, 18.0]
                        .into_iter()
                        .map(|z| AssistantCadEditOperation::CreatePanel {
                            name: format!("Panel {z}"),
                            dimensions_mm: [100.0, 50.0, 18.0],
                            holes: vec![],
                            translation_mm: [0.0, 0.0, z],
                            rotation: None,
                        })
                        .collect(),
                ),
            )
            .unwrap();
            document.apply_batch(&panels).unwrap();
            let mut commands = Vec::new();
            if nested {
                commands.push(CanonicalCommand::CreateGroup {
                    id: GroupId(1),
                    name: "Nested assembly".into(),
                    transform: Transform::identity(),
                    parent: None,
                });
            }
            commands.push(CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(shared_id),
                name: "Untouched hidden sibling".into(),
                transform: Transform::from_translation(200.0, 0.0, 0.0).unwrap(),
                parent: nested.then_some(GroupId(1)),
                tag: None,
                visible: false,
            });
            document.apply_batch(&CommandBatch::new(commands)).unwrap();
            if nested {
                document
                    .convert_group_to_component(GroupId(1), "Nested component")
                    .unwrap();
            }
            let before = document.current();
            let undo = document.visible_undo_steps();
            let redo = document.visible_redo_steps();
            let epoch = document.mutation_epoch();
            let sibling = before
                .scene_query()
                .into_iter()
                .find(|instance| {
                    instance.definition_id == DefinitionId(shared_id)
                        && (instance.instance_path.is_root() == !nested)
                        && instance.occurrence_name == "Untouched hidden sibling"
                })
                .expect("the shared sibling must actually exist");
            assert!(!sibling.visible);
            let face = |id, z, inward_unit_local| AssistantDowelJointFace {
                instance_path: AssistantInstancePath {
                    root_occurrence_id: id,
                    steps: vec![],
                },
                face_origin_local_mm: [0.0, 0.0, z],
                inward_unit_local,
                bounds_min_local_mm: [0.0; 3],
                bounds_max_local_mm: [100.0, 50.0, 18.0],
            };
            let rejection = plan(
                &document,
                &BTreeSet::new(),
                &ExactResultRegistry::default(),
                &program(vec![AssistantCadEditOperation::CreatePhysicalDowelJoint {
                    joint_id: None,
                    name: "Must not drill sibling".into(),
                    first: face(1, 18.0, [0.0, 0.0, -1.0]),
                    second: face(2, 0.0, [0.0, 0.0, 1.0]),
                    first_center_local_mm: [20.0, 20.0, 18.0],
                    row_unit_first_local: [1.0, 0.0, 0.0],
                    count: 3,
                    spacing_mm: 25.0,
                    dowel: AssistantStandardDowel::D8x30,
                }]),
            )
            .unwrap_err();
            assert_eq!(rejection.code, "planning.physical_dowel_shared_definition");
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            assert_eq!(document.current().revision_id(), before.revision_id());
            assert_eq!(document.mutation_epoch(), epoch);
            assert_eq!(document.visible_undo_steps(), undo);
            assert_eq!(document.visible_redo_steps(), redo);
            assert_eq!(document.current().dowel_joints().count(), 0);
        }
    }
}

#[test]
fn physical_dowel_joint_supports_both_rotated_sides_and_preserves_existing_work() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm, rotation: Option<AssistantCadRotation>, holes| {
        AssistantCadEditOperation::CreatePanel {
            name: name.into(),
            dimensions_mm: [100.0, 50.0, 18.0],
            holes,
            translation_mm,
            rotation,
        }
    };
    let rotation = |pivot_mm, angle_degrees| AssistantCadRotation {
        pivot_mm,
        axis: [0.0, 1.0, 0.0],
        angle_degrees,
    };
    let hardware = AssistantPanelHole {
        id: "hinge-hardware".into(),
        entry_local_mm: [50.0, 25.0, 18.0],
        inward_unit_local: [0.0, 0.0, -1.0],
        diameter_mm: 6.0,
        depth_mm: 10.0,
    };
    let panels = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Back panel", [0.0, 0.0, 0.0], None, vec![hardware]),
            panel("Top panel", [0.0, 0.0, 18.0], None, Vec::new()),
            panel(
                "Left side",
                [0.0, 0.0, 0.0],
                Some(rotation([0.0, 0.0, 0.0], -90.0)),
                Vec::new(),
            ),
            panel(
                "Right side",
                [100.0, 0.0, 100.0],
                Some(rotation([100.0, 0.0, 100.0], 90.0)),
                Vec::new(),
            ),
        ]),
    )
    .unwrap();
    document.apply_batch(&panels).unwrap();
    let face = |occurrence_id, face_origin_local_mm, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm,
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let top_joint = AssistantCadEditOperation::CreatePhysicalDowelJoint {
        joint_id: None,
        name: "Existing top row".into(),
        first: face(1, [0.0, 0.0, 18.0], [0.0, 0.0, -1.0]),
        second: face(2, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        first_center_local_mm: [30.0, 45.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 2,
        spacing_mm: 40.0,
        dowel: AssistantStandardDowel::D8x30,
    };
    let top_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![top_joint]),
    )
    .unwrap();
    document.apply_batch(&top_batch).unwrap();
    let before_sides = document.current();
    let preserved_joint = before_sides
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap()
        .clone();
    let hardware_feature_id = before_sides
        .features()
        .find(|feature| feature.name() == "Back panel hole hinge-hardware pocket")
        .unwrap()
        .id();
    let side_joint = |name: &str, first, second, first_center_local_mm| {
        AssistantCadEditOperation::CreatePhysicalDowelJoint {
            joint_id: None,
            name: name.into(),
            first,
            second,
            first_center_local_mm,
            row_unit_first_local: [0.0, 1.0, 0.0],
            count: 2,
            spacing_mm: 20.0,
            dowel: AssistantStandardDowel::D8x30,
        }
    };
    let sides = program(vec![
        side_joint(
            "Left physical row",
            face(1, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            face(3, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            [0.0, 10.0, 9.0],
        ),
        side_joint(
            "Right physical row",
            face(1, [100.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
            face(4, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            [100.0, 10.0, 9.0],
        ),
    ]);
    let side_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &sides,
    )
    .unwrap();
    assert_eq!(
        side_batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pocket { .. },
                    ..
                }
            ))
            .count(),
        8
    );
    let undo_before_sides = document.visible_undo_steps();
    document.apply_batch(&side_batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before_sides + 1);
    let committed = document.current();
    assert_eq!(committed.dowel_joints().count(), 3);
    assert_eq!(
        committed
            .dowel_joint(ketchup_core::joinery::DowelJointId(1))
            .unwrap(),
        &preserved_joint
    );
    assert!(committed.feature(hardware_feature_id).is_some());
    assert_eq!(
        committed
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::Pocket { .. }))
            .count(),
        13
    );
    for (joint_id, expected_centers, first_inward) in [
        (
            1,
            [[30.0, 45.0, 18.0], [70.0, 45.0, 18.0]],
            [0.0, 0.0, -1.0],
        ),
        (2, [[0.0, 10.0, 9.0], [0.0, 30.0, 9.0]], [1.0, 0.0, 0.0]),
        (
            3,
            [[100.0, 10.0, 9.0], [100.0, 30.0, 9.0]],
            [-1.0, 0.0, 0.0],
        ),
    ] {
        let joint = committed
            .dowel_joint(ketchup_core::joinery::DowelJointId(joint_id))
            .unwrap();
        let projection =
            ketchup_core::joinery::project_dowel_joint_contract(&committed, joint).unwrap();
        assert_eq!(projection.pairs.len(), 2);
        assert_eq!(
            projection
                .pairs
                .iter()
                .map(|pair| pair.first.shared_center_world_mm)
                .collect::<Vec<_>>(),
            expected_centers
        );
        for (pair, center) in projection.pairs.iter().zip(expected_centers) {
            assert_eq!(pair.first.diameter_mm, 8.0);
            assert_eq!(pair.second.diameter_mm, 8.0);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            let probe = pair.physical_probe_coincidence.unwrap();
            assert!(probe.maximum_endpoint_error_mm <= 1.0e-8);
            // D8x30 manufacturing oracle: 15 mm insertion on each side, not the 16 mm hole depth.
            for (endpoint, insertion) in [(0, 15.0), (1, -15.0)] {
                for axis in 0..3 {
                    let expected = center[axis] + first_inward[axis] * insertion;
                    assert!(
                        (probe.first_probe_endpoints_world_mm[endpoint][axis] - expected).abs()
                            <= 1.0e-8
                    );
                    assert!(
                        (probe.second_probe_endpoints_world_mm[endpoint][axis] - expected).abs()
                            <= 1.0e-8
                    );
                }
            }
        }
    }
}

#[test]
fn named_program_outputs_create_panels_physical_holes_and_joint_in_one_atomic_batch() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let panel = |name: &str, translation_mm, entry_z, inward_unit_local| {
        AssistantCadEditOperation::CreatePanel {
            name: name.into(),
            dimensions_mm: [100.0, 50.0, 18.0],
            holes: vec![AssistantPanelHole {
                id: "dowel-1".into(),
                entry_local_mm: [20.0, 20.0, entry_z],
                inward_unit_local,
                diameter_mm: 8.0,
                depth_mm: 16.0,
            }],
            translation_mm,
            rotation: None,
        }
    };
    let bind = |name: &str, operation_index, output| AssistantCadEditOperation::BindProgramOutput {
        name: name.into(),
        source: AssistantCadProgramFeatureReference {
            operation_index,
            output,
        },
    };
    let named = |name: &str, output| AssistantCadNamedProgramOutputReference {
        name: name.into(),
        output,
    };
    let face = |name: &str, face_z, inward_unit_local| AssistantProgramDowelJointFace {
        occurrence: named(name, AssistantCadProgramFeatureOutput::Occurrence),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let input = program(vec![
        panel("Lower panel", [0.0, 0.0, 0.0], 18.0, [0.0, 0.0, -1.0]),
        bind("lower", 0, AssistantCadProgramFeatureOutput::Occurrence),
        bind(
            "lower-hole",
            0,
            AssistantCadProgramFeatureOutput::BodyFeature,
        ),
        panel("Upper panel", [0.0, 0.0, 18.0], 0.0, [0.0, 0.0, 1.0]),
        bind("upper", 3, AssistantCadProgramFeatureOutput::Occurrence),
        bind(
            "upper-hole",
            3,
            AssistantCadProgramFeatureOutput::BodyFeature,
        ),
        AssistantCadEditOperation::CreateProgramDowelJoint {
            name: "Bound row".into(),
            first: face("lower", 18.0, [0.0, 0.0, -1.0]),
            second: face("upper", 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 1,
            spacing_mm: 0.0,
            dowel: AssistantStandardDowel::D8x30,
            physical_hole_pairs: vec![AssistantProgramDowelPhysicalHolePair {
                first_pocket_feature: named(
                    "lower-hole",
                    AssistantCadProgramFeatureOutput::BodyFeature,
                ),
                second_pocket_feature: named(
                    "upper-hole",
                    AssistantCadProgramFeatureOutput::BodyFeature,
                ),
            }],
        },
        bind("joint", 6, AssistantCadProgramFeatureOutput::DowelJoint),
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();

    let planned = plan_with_outputs(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
    let repeated = plan_with_outputs(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(repeated.outputs, planned.outputs);
    assert_eq!(repeated.batch.commands(), planned.batch.commands());
    assert!(matches!(
        planned.outputs.get("lower"),
        Some(AssistantCadResolvedProgramOutput::Occurrence(1))
    ));
    assert!(matches!(
        planned.outputs.get("upper"),
        Some(AssistantCadResolvedProgramOutput::Occurrence(2))
    ));
    assert!(matches!(
        planned.outputs.get("lower-hole"),
        Some(AssistantCadResolvedProgramOutput::BodyFeature(_))
    ));
    assert!(matches!(
        planned.outputs.get("upper-hole"),
        Some(AssistantCadResolvedProgramOutput::BodyFeature(_))
    ));
    assert_eq!(
        planned.outputs.get("joint"),
        Some(&AssistantCadResolvedProgramOutput::DowelJoint(1))
    );

    document.apply_batch(&planned.batch).unwrap();
    let committed = document.current();
    let joint = committed
        .dowel_joint(ketchup_core::joinery::DowelJointId(1))
        .unwrap();
    assert_eq!(joint.physical_hole_pairs.as_ref().unwrap().len(), 1);
    let projection =
        ketchup_core::joinery::project_dowel_joint_contract(&committed, joint).unwrap();
    assert_eq!(
        projection.pairs[0]
            .physical_probe_coincidence
            .as_ref()
            .unwrap()
            .maximum_endpoint_error_mm,
        0.0
    );
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn named_program_outputs_reject_duplicates_forward_wrong_types_and_late_failure_without_mutation() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let bind = |name: &str, operation_index, output| AssistantCadEditOperation::BindProgramOutput {
        name: name.into(),
        source: AssistantCadProgramFeatureReference {
            operation_index,
            output,
        },
    };
    let duplicate = program(vec![
        part(),
        bind("part", 0, AssistantCadProgramFeatureOutput::Definition),
        bind("part", 0, AssistantCadProgramFeatureOutput::Occurrence),
    ]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &duplicate
        )
        .is_err()
    );

    let wrong_type = program(vec![
        part(),
        bind(
            "not-a-joint",
            0,
            AssistantCadProgramFeatureOutput::DowelJoint,
        ),
    ]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &wrong_type,
        )
        .is_err()
    );

    let late_failure = program(vec![
        part(),
        bind("part", 0, AssistantCadProgramFeatureOutput::Occurrence),
        AssistantCadEditOperation::SetDimension {
            feature_id: u64::MAX,
            constraint_id: None,
            value_mm: 20.0,
        },
    ]);
    let late_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &late_failure,
    )
    .unwrap();
    assert!(document.apply_batch(&late_batch).is_err());
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);

    let missing_or_forward: AssistantCadEditProgram = serde_json::from_value(serde_json::json!({
        "operations": [
            {
                "operation": "create_program_dowel_joint",
                "name": "Invalid",
                "first": {
                    "occurrence": {"name": "later", "output": "occurrence"},
                    "face_origin_local_mm": [0.0, 0.0, 18.0],
                    "inward_unit_local": [0.0, 0.0, -1.0],
                    "bounds_min_local_mm": [0.0, 0.0, 0.0],
                    "bounds_max_local_mm": [100.0, 50.0, 18.0]
                },
                "second": {
                    "occurrence": {"name": "wrong-type", "output": "occurrence"},
                    "face_origin_local_mm": [0.0, 0.0, 0.0],
                    "inward_unit_local": [0.0, 0.0, 1.0],
                    "bounds_min_local_mm": [0.0, 0.0, 0.0],
                    "bounds_max_local_mm": [100.0, 50.0, 18.0]
                },
                "first_center_local_mm": [20.0, 20.0, 18.0],
                "row_unit_first_local": [1.0, 0.0, 0.0],
                "count": 1,
                "spacing_mm": 0.0,
                "dowel": "d8x30",
                "physical_hole_pairs": [{
                    "first_pocket_feature": {"name": "first-hole", "output": "body_feature"},
                    "second_pocket_feature": {"name": "second-hole", "output": "body_feature"}
                }]
            },
            {
                "operation": "bind_program_output",
                "name": "later",
                "source": {"operation_index": 2, "output": "occurrence"}
            },
            {
                "operation": "create_panel",
                "name": "Later panel",
                "dimensions_mm": [100.0, 50.0, 18.0],
                "holes": [],
                "translation_mm": [0.0, 0.0, 0.0]
            }
        ]
    }))
    .unwrap();
    assert!(missing_or_forward.validate().is_err());
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn bound_dowel_joint_rejects_moving_only_one_physical_hole() {
    let mut document = DocumentStore::new();
    let holes = |entry_z, inward_unit_local| {
        [20.0, 45.0, 70.0]
            .into_iter()
            .enumerate()
            .map(|(index, x)| AssistantPanelHole {
                id: format!("dowel-{}", index + 1),
                entry_local_mm: [x, 20.0, entry_z],
                inward_unit_local,
                diameter_mm: 8.0,
                depth_mm: 16.0,
            })
            .collect()
    };
    let panel = |name: &str, translation_mm, entry_z, inward_unit_local| {
        AssistantCadEditOperation::CreatePanel {
            name: name.into(),
            dimensions_mm: [100.0, 50.0, 18.0],
            holes: holes(entry_z, inward_unit_local),
            translation_mm,
            rotation: None,
        }
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Lower bound panel", [0.0, 0.0, 0.0], 18.0, [0.0, 0.0, -1.0]),
            panel("Upper bound panel", [0.0, 0.0, 18.0], 0.0, [0.0, 0.0, 1.0]),
        ]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let pocket_ids = |snapshot: &Snapshot, definition_id| {
        snapshot
            .definition(definition_id)
            .unwrap()
            .feature_ids()
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    snapshot.feature(*id).unwrap().kind(),
                    FeatureKind::Pocket { .. }
                )
            })
            .collect::<Vec<_>>()
    };
    let before_joint = document.current();
    let first_pockets = pocket_ids(&before_joint, DefinitionId(1));
    let second_pockets = pocket_ids(&before_joint, DefinitionId(2));
    assert_eq!(first_pockets.len(), 3);
    assert_eq!(second_pockets.len(), 3);
    let face = |occurrence_id, face_z, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let physical_hole_pairs = first_pockets
        .iter()
        .zip(&second_pockets)
        .map(|(first, second)| AssistantDowelPhysicalHolePair {
            first_pocket_feature_id: first.0,
            second_pocket_feature_id: second.0,
        })
        .collect();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::CreateDowelJoint {
            name: "Bound shelf row".into(),
            first: face(1, 18.0, [0.0, 0.0, -1.0]),
            second: face(2, 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 25.0,
            dowel: AssistantStandardDowel::D8x30,
            physical_hole_pairs,
        }]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let bound = document.current();
    let reopened = load(&save(&bound)).unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .dowel_joint(ketchup_core::joinery::DowelJointId(1))
            .unwrap()
            .physical_hole_pairs
            .as_ref()
            .unwrap()
            .len(),
        3
    );
    let relation_page = ModelQuery::default()
        .page(
            &bound,
            &PageRequest {
                kind: EntityKind::Relations,
                limit: 10,
                search: "dowel_joint".into(),
                definition_id: None,
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
            },
        )
        .unwrap();
    let pairs = relation_page["items"][0]["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 3);
    assert!(pairs.iter().all(|pair| {
        pair["physical_probe_coincidence"]["full_length_coincident"] == true
            && pair["physical_probe_coincidence"]["maximum_endpoint_error_mm"] == 0.0
    }));
    let profile_id = match bound.feature(first_pockets[0]).unwrap().kind() {
        FeatureKind::Pocket { profile, .. } => *profile,
        _ => unreachable!(),
    };
    let revision_before_invalid_move = bound.revision_id();
    let invalid_move = program(vec![AssistantCadEditOperation::SetFeatureParameter {
        feature_id: profile_id.0,
        parameter_path: "entities.1.center.x".into(),
        value_type: AssistantCadParameterValueType::Length,
        value: 1.0,
    }]);
    let invalid_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &invalid_move,
    )
    .unwrap();
    assert!(document.apply_batch(&invalid_batch).is_err());
    assert_eq!(
        document.current().revision_id(),
        revision_before_invalid_move
    );
}

#[test]
fn typed_program_outputs_reject_forward_and_cross_kind_references_without_mutation() {
    let document = DocumentStore::new();
    let baseline = document.current();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let sketch = |definition| AssistantCadEditOperation::CreateProgramSketch {
        definition,
        name: "Opening profile".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 4.0,
        }],
        constraints: Vec::new(),
    };
    let pocket = |target_feature, profile_feature| AssistantCadEditOperation::AppendProgramPocket {
        definition: output(0, AssistantCadProgramFeatureOutput::Definition),
        name: "Opening".into(),
        target_feature,
        profile_feature,
        depth_mm: 10.0,
    };
    let invalid_programs = [
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::BodyFeature)),
        ]),
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::Definition)),
            pocket(
                output(0, AssistantCadProgramFeatureOutput::BodyFeature),
                output(0, AssistantCadProgramFeatureOutput::BodyFeature),
            ),
        ]),
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::Definition)),
            pocket(
                output(2, AssistantCadProgramFeatureOutput::BodyFeature),
                output(1, AssistantCadProgramFeatureOutput::SketchFeature),
            ),
        ]),
        program(vec![
            part(),
            AssistantCadEditOperation::CreateProgramSketch {
                definition: output(0, AssistantCadProgramFeatureOutput::Definition),
                name: "Guessed workplane".into(),
                workplane: AssistantWorkplaneSpec::Offset {
                    base_feature_id: 1,
                    distance_mm: 1.0,
                },
                entities: vec![AssistantSketchEntity::Circle {
                    id: 1,
                    center_mm: [0.0, 0.0],
                    radius_mm: 4.0,
                }],
                constraints: Vec::new(),
            },
        ]),
    ];
    for input in invalid_programs {
        let rejection = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &input,
        )
        .unwrap_err();
        assert_eq!(rejection.phase, AssistantRejectionPhase::IntentValidation);
        assert_eq!(rejection.code, "intent.cad_edit_program_invalid");
    }
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn explicit_targets_ignore_selection_and_current_selection_is_borrowed() {
    let document = seeded();
    let registry = ExactResultRegistry::default();
    let input = program(vec![translate(explicit(1), [1.0, 2.0, 3.0])]);
    let explicit_batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    // Even an unrelated stale selection cannot override explicit targets.
    assert_eq!(
        explicit_batch,
        plan(
            &document,
            &BTreeSet::from([OccurrenceId(999)]),
            &registry,
            &input
        )
        .unwrap()
    );
    let selected_input = program(vec![translate(
        AssistantCadEntitySelector::CurrentSelection {},
        [1.0, 2.0, 3.0],
    )]);
    let selection = BTreeSet::from([OccurrenceId(1)]);
    assert_eq!(
        explicit_batch,
        plan(&document, &selection, &registry, &selected_input).unwrap()
    );
    assert_eq!(selection, BTreeSet::from([OccurrenceId(1)]));
    let error = plan(&document, &BTreeSet::new(), &registry, &selected_input).unwrap_err();
    assert_eq!(error.phase, AssistantRejectionPhase::ProposalPlanning);
    assert_eq!(error.code, "planning.cad_selector_invalid");
}

#[test]
fn missing_and_stale_selection_keep_canonical_diagnostics_without_mutation() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let stale_selection = BTreeSet::from([OccurrenceId(1)]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(1),
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    for (selector, id) in [
        (explicit(999), 999),
        (AssistantCadEntitySelector::CurrentSelection {}, 1),
    ] {
        let error = plan(
            &document,
            &stale_selection,
            &registry,
            &program(vec![translate(selector, [1.0, 0.0, 0.0])]),
        )
        .unwrap_err();
        assert_eq!(error.phase, AssistantRejectionPhase::CanonicalValidation);
        assert_eq!(
            error.code,
            CanonicalError::OccurrenceNotFound(OccurrenceId(id)).code()
        );
        assert_eq!(error.operation, "transform_occurrence");
        assert_eq!(error.target, format!("occurrence:{id}"));
        assert!(error.retryable);
        assert_eq!(error.validate(), Ok(()));
    }
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.current().revision_id(), baseline.revision_id());
    assert_eq!(document.visible_undo_steps(), undo);
}

#[test]
fn transform_copy_pattern_and_mirror_use_accumulated_transforms_atomically() {
    let mut document = seeded();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let input = program(vec![
        translate(explicit(1), [10.0, 0.0, 0.0]),
        AssistantCadEditOperation::Transform {
            selector: explicit(1),
            translation_mm: [0.0; 3],
            rotation: Some(AssistantCadRotation {
                pivot_mm: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                angle_degrees: 90.0,
            }),
        },
        AssistantCadEditOperation::Copy {
            selector: explicit(1),
            translation_mm: [0.0, 20.0, 0.0],
        },
        AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances: 3,
            step_mm: [25.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::Mirror {
            selector: explicit(1),
            plane_origin_mm: [0.0; 3],
            plane_normal: [1.0, 0.0, 0.0],
        },
    ]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 6);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    for (id, expected) in [
        (1, [-6.0, 15.0, 7.0]),
        (2, [-6.0, 35.0, 7.0]),
        (3, [19.0, 15.0, 7.0]),
        (4, [44.0, 15.0, 7.0]),
        (5, [6.0, 15.0, 7.0]),
    ] {
        let occurrence = committed.occurrence(OccurrenceId(id)).unwrap();
        assert_eq!(occurrence.definition_id(), DefinitionId(1));
        let transform = occurrence.transform();
        for (index, value) in [3, 7, 11].into_iter().zip(expected) {
            assert!((transform.matrix()[index] - value).abs() < 1e-9);
        }
    }
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn circular_pattern_uses_the_shared_arbitrary_axis_atomically() {
    let direct = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 4,
        axis: AssistantAxisSpec::OriginDirection {
            origin_mm: [1.0, 2.0, 3.0],
            direction: [1.0, 1.0, 0.0],
        },
        angle_step_degrees: 90.0,
    }]);
    let two_points = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 4,
        axis: AssistantAxisSpec::TwoPoints {
            start_mm: [1.0, 2.0, 3.0],
            end_mm: [2.0, 3.0, 3.0],
        },
        angle_step_degrees: 90.0,
    }]);
    let mut document = seeded();
    let before = document.current();
    let undo = document.visible_undo_steps();
    let registry = ExactResultRegistry::default();
    let direct_batch = plan(&document, &BTreeSet::new(), &registry, &direct).unwrap();
    let points_batch = plan(&document, &BTreeSet::new(), &registry, &two_points).unwrap();
    assert_eq!(direct_batch.commands(), points_batch.commands());
    assert_eq!(direct_batch.commands().len(), 3);

    document.apply_batch(&direct_batch).unwrap();
    let patterned = document.current();
    assert_eq!(patterned.occurrences().count(), 4);
    let first_copy = patterned.occurrence(OccurrenceId(2)).unwrap().transform();
    let expected_translation = [7.828_427_124_746_19, 3.171_572_875_253_81, 3.0];
    for (index, expected) in [3, 7, 11].into_iter().zip(expected_translation) {
        assert!((first_copy.matrix()[index] - expected).abs() < 1.0e-9);
    }
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        patterned.canonical_digest()
    );

    let duplicate = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 3,
        axis: AssistantAxisSpec::OriginDirection {
            origin_mm: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        },
        angle_step_degrees: 180.0,
    }]);
    let error = plan(&document, &BTreeSet::new(), &registry, &duplicate).unwrap_err();
    assert_eq!(error.phase, AssistantRejectionPhase::IntentValidation);
}

#[test]
fn invalid_and_deleted_targets_fail_the_whole_plan() {
    let document = seeded();
    let baseline = document.current();
    let registry = ExactResultRegistry::default();
    let error = plan(&document, &BTreeSet::new(), &registry, &program(vec![])).unwrap_err();
    assert_eq!(error.code, "intent.cad_edit_program_invalid");
    assert_eq!(error.phase, AssistantRejectionPhase::IntentValidation);
    let input = program(vec![
        translate(explicit(1), [100.0, 0.0, 0.0]),
        AssistantCadEditOperation::Delete {
            selector: explicit(1),
            dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
        },
        translate(explicit(1), [10.0, 0.0, 0.0]),
    ]);
    let error = plan(&document, &BTreeSet::new(), &registry, &input).unwrap_err();
    assert_eq!(error.code, "planning.cad_target_deleted");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn canonical_batch_validation_remains_atomic_for_deferred_dimension_errors() {
    let mut document = seeded();
    let baseline = document.current();
    // S1 preserves the planner/DocumentStore validation boundary: dimensions
    // are validated canonically when the returned batch is previewed/applied.
    let input = program(vec![
        translate(explicit(1), [10.0, 0.0, 0.0]),
        AssistantCadEditOperation::SetDimension {
            feature_id: 999,
            constraint_id: None,
            value_mm: 10.0,
        },
    ]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(
        document.preview_batch(&batch).err().unwrap(),
        CanonicalError::FeatureNotFound(FeatureId(999))
    );
    assert_eq!(
        document.apply_batch(&batch).err().unwrap(),
        CanonicalError::FeatureNotFound(FeatureId(999))
    );
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn public_framed_sketch_output_builds_an_exact_editable_planar_offset() {
    let mut document = DocumentStore::new();
    let part_program = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Framed cubic profile".into(),
        workplane: AssistantWorkplaneSpec::Frame {
            origin_mm: [30.0, -20.0, 15.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
        },
        entities: vec![
            AssistantSketchEntity::CubicBezier {
                id: 1,
                start_mm: [-20.0, 0.0],
                control_1_mm: [-20.0, 15.0],
                control_2_mm: [20.0, 15.0],
                end_mm: [20.0, 0.0],
            },
            AssistantSketchEntity::CubicBezier {
                id: 2,
                start_mm: [20.0, 0.0],
                control_1_mm: [20.0, -15.0],
                control_2_mm: [-20.0, -15.0],
                end_mm: [-20.0, 0.0],
            },
        ],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let registry = ExactResultRegistry::default();
    let part_batch = plan(&document, &BTreeSet::new(), &registry, &part_program).unwrap();
    document.apply_batch(&part_batch).unwrap();
    let baseline_digest = document.current().canonical_digest();
    let offset_program = |distance_mm| {
        program(vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Framed planar offset".into(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id: 2,
                distance_mm,
            },
        }])
    };
    let input = offset_program(3.0);
    let encoded = serde_json::to_vec(&input).unwrap();
    let decoded: AssistantCadEditProgram = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, input);

    let batch = plan(&document, &BTreeSet::new(), &registry, &decoded).unwrap();
    let candidate = document.preview_batch(&batch).unwrap();
    let graph = ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(4)).unwrap();
    assert!(graph.terminal_is_planar_offset());
    assert_eq!(graph.profiles[0].source_feature_id, 2);
    assert_eq!(graph.profiles[0].frame_bits[0], 30.0_f64.to_bits());
    assert_eq!(graph.profiles[0].frame_bits[1], (-20.0_f64).to_bits());
    assert_eq!(graph.profiles[0].frame_bits[2], 15.0_f64.to_bits());
    let bounds = graph.producer_bounds_mm().unwrap().unwrap();
    assert_eq!(bounds[0][0], 30.0);
    assert_eq!(bounds[1][0], 30.0);
    assert!(bounds[0][1] < -40.0 && bounds[1][1] > 0.0);
    assert!(bounds[0][2] < 15.0 && bounds[1][2] > 15.0);

    let error = plan(
        &document,
        &BTreeSet::new(),
        &registry,
        &offset_program(-21.0),
    )
    .unwrap_err();
    assert_eq!(error.code, "planning.cad_feature_input_unsupported");
    assert_eq!(document.current().canonical_digest(), baseline_digest);

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 2);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("framed Planar Offset must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(1), FeatureId(4),).is_ok()
    );
}

#[test]
fn missing_topology_evidence_cannot_authorize_a_finish() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Body".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("30", 30.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Instance".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: 1,
        name: "Finish".into(),
        feature: AssistantCadBodyFeature::TopologyFillet {
            target_feature_id: 2,
            edge_reference_ids: vec!["a".repeat(64)],
            radius_mm: 1.0,
            radius_stations: Vec::new(),
        },
    }]);
    let error = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap_err();
    assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn set_grounded_program_grounds_and_ungrounds_selected_occurrences_in_one_undo_step() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let before = document.current();
    let ground = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: explicit(1),
        grounded: true,
    }]);
    document
        .apply_batch(&plan(&document, &BTreeSet::new(), &registry, &ground).unwrap())
        .unwrap();
    assert!(document.current().occurrence_is_grounded(OccurrenceId(1)));
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    let unground = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: explicit(1),
        grounded: false,
    }]);
    document
        .apply_batch(&plan(&document, &BTreeSet::new(), &registry, &unground).unwrap())
        .unwrap();
    assert!(!document.current().occurrence_is_grounded(OccurrenceId(1)));
    let missing = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![999],
        },
        grounded: true,
    }]);
    assert!(plan(&document, &BTreeSet::new(), &registry, &missing).is_err());
}

#[test]
fn color_program_is_atomic_and_copy_pattern_mirror_preserve_accumulated_color() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let color = Some([12, 128, 255]);
    let input = program(vec![
        AssistantCadEditOperation::SetColor {
            selector: explicit(1),
            color,
        },
        AssistantCadEditOperation::Copy {
            selector: explicit(1),
            translation_mm: [30.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances: 3,
            step_mm: [0.0, 30.0, 0.0],
        },
        AssistantCadEditOperation::Mirror {
            selector: explicit(1),
            plane_origin_mm: [0.0; 3],
            plane_normal: [1.0, 0.0, 0.0],
        },
    ]);
    let before = document.current();
    let batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.current().occurrences().count(), 5);
    assert!(document.current().occurrences().all(|o| o.color() == color));
    let colored = document.current().canonical_digest();
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), colored);
    let selection = BTreeSet::from([OccurrenceId(1)]);
    let reset = program(vec![AssistantCadEditOperation::SetColor {
        selector: AssistantCadEntitySelector::CurrentSelection {},
        color: None,
    }]);
    document
        .apply_batch(&plan(&document, &selection, &registry, &reset).unwrap())
        .unwrap();
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .color(),
        None
    );
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(2))
            .unwrap()
            .color(),
        color
    );
    let baseline = document.current().canonical_digest();
    for ids in [vec![], vec![1, 1], vec![1, 999]] {
        let invalid = program(vec![AssistantCadEditOperation::SetColor {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: ids,
            },
            color,
        }]);
        assert!(plan(&document, &selection, &registry, &invalid).is_err());
        assert_eq!(document.current().canonical_digest(), baseline);
    }
}

#[test]
fn color_wire_rejects_non_rgb_bytes() {
    for color in [
        "[-1,0,0]",
        "[256,0,0]",
        "[1.5,0,0]",
        "[true,0,0]",
        "[1,2]",
        "[1,2,3,4]",
        "\"red\"",
    ] {
        let json = format!(
            r#"{{"operations":[{{"operation":"set_color","selector":{{"type":"occurrences","occurrence_ids":[1]}},"color":{color}}}]}}"#
        );
        assert!(
            serde_json::from_str::<AssistantCadEditProgram>(&json).is_err(),
            "{json}"
        );
    }
}

#[test]
fn public_program_sweeps_a_general_sketch_profile_along_a_spatial_path_atomically() {
    let definition = DefinitionId(1);
    let workplane = FeatureId(1);
    let profile = FeatureId(2);
    let path = FeatureId(3);
    let profile_sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-2.0, -2.0],
                end_mm: [2.0, -2.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [2.0, -2.0],
                end_mm: [2.0, 2.0],
                center_mm: [2.0, 0.0],
                clockwise: false,
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [2.0, 2.0],
                control_1_mm: [0.5, 3.0],
                control_2_mm: [-0.5, 3.0],
                end_mm: [-2.0, 2.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-2.0, 2.0],
                end_mm: [-2.0, -2.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(5),
                center_mm: [0.0, 0.0],
                radius_mm: 0.5,
            },
        ],
        constraints: Vec::new(),
    };
    let spatial_segments = vec![
        SpatialPathSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [0.0, 0.0, 20.0],
        },
        SpatialPathSegment::CircularArc {
            start_mm: [0.0, 0.0, 20.0],
            end_mm: [10.0, 0.0, 30.0],
            center_mm: [10.0, 0.0, 20.0],
            normal: [0.0, 1.0, 0.0],
            clockwise: false,
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [10.0, 0.0, 30.0],
            control_1_mm: [15.0, 0.0, 30.0],
            control_2_mm: [20.0, 5.0, 35.0],
            end_mm: [25.0, 10.0, 40.0],
        },
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "General sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line arc cubic profile".into(),
                kind: FeatureKind::Sketch(profile_sketch.clone()),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "3D path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: spatial_segments.clone(),
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0,
        name: "Public general sweep".into(),
        feature: AssistantCadBodyFeature::Sweep {
            profile_feature_id: profile.0,
            path_feature_id: path.0,
        },
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let sweep = FeatureId(4);
    let graph = ExactBRepGraph::from_snapshot(&preview, definition, sweep).unwrap();
    assert!(matches!(
        &graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    ));
    assert!(matches!(
        &graph.profiles[0].geometry,
        ExactBRepPlanarGeometry::Region { outer, holes }
            if matches!(outer, ketchup_core::exact_brep_graph::ExactBRepPlanarLoop::Boundary { segments } if segments.len() == 4)
                && holes.len() == 1
    ));

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(document.visible_undo_steps(), 2);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("general Sketch/SpatialPath Sweep must reopen losslessly");
    };
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );

    let mut sketch_path_document = DocumentStore::new();
    sketch_path_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Sketch path sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line arc cubic profile".into(),
                kind: FeatureKind::Sketch(profile_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Path plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xz)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: definition,
                name: "2D curved path".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: path,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            end_mm: [0.0, 20.0],
                        },
                        SketchEntity::Arc {
                            id: SketchEntityId(2),
                            start_mm: [0.0, 20.0],
                            end_mm: [10.0, 30.0],
                            center_mm: [10.0, 20.0],
                            clockwise: true,
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [10.0, 30.0],
                            control_1_mm: [15.0, 30.0],
                            control_2_mm: [20.0, 35.0],
                            end_mm: [25.0, 40.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
        ]))
        .unwrap();
    let sketch_path_program = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0,
        name: "Public Sketch path sweep".into(),
        feature: AssistantCadBodyFeature::Sweep {
            profile_feature_id: profile.0,
            path_feature_id: 4,
        },
    }]);
    let sketch_path_batch = plan(
        &sketch_path_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &sketch_path_program,
    )
    .unwrap();
    let sketch_path_preview = sketch_path_document
        .preview_batch(&sketch_path_batch)
        .unwrap();
    let sketch_path_graph =
        ExactBRepGraph::from_snapshot(&sketch_path_preview, definition, FeatureId(5)).unwrap();
    assert!(matches!(
        &sketch_path_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    ));

    let mut invalid = DocumentStore::new();
    invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Invalid sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Open profile".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![SketchEntity::Line {
                        id: SketchEntityId(1),
                        start_mm: [-2.0, 0.0],
                        end_mm: [2.0, 0.0],
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "3D path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: spatial_segments,
                },
            },
        ]))
        .unwrap();
    let invalid_baseline = invalid.current();
    assert!(
        plan(
            &invalid,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &input,
        )
        .is_err()
    );
    assert_eq!(
        invalid.current().canonical_digest(),
        invalid_baseline.canonical_digest()
    );
}

#[test]
fn public_surface_program_is_typed_atomic_and_refuses_solid_surface_targets() {
    let definition = DefinitionId(1);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Public surfaces".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: definition,
                name: "Outer profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: definition,
                name: "Cutter profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[5.0, -5.0], [15.0, -5.0], [15.0, 15.0], [5.0, 15.0]],
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let earlier_body = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let input = program(vec![
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Planar surface A".into(),
            feature: AssistantCadBodyFeature::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar {
                    profile_feature_id: FeatureId(1).0.into(),
                },
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Planar surface B".into(),
            feature: AssistantCadBodyFeature::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar {
                    profile_feature_id: FeatureId(2).0.into(),
                },
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Trimmed surface".into(),
            feature: AssistantCadBodyFeature::SurfaceTrim {
                target_feature_id: earlier_body(0),
                cutter_feature_id: earlier_body(1),
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Extended surface".into(),
            feature: AssistantCadBodyFeature::SurfaceExtend {
                target_feature_id: earlier_body(2),
                distance_mm: 2.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Knitted surface".into(),
            feature: AssistantCadBodyFeature::SurfaceKnit {
                surface_feature_ids: vec![earlier_body(3), earlier_body(1)],
                tolerance_mm: 0.001,
                make_solid: false,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0,
            name: "Thickened solid".into(),
            feature: AssistantCadBodyFeature::SurfaceThicken {
                target_feature_id: earlier_body(4),
                thickness_mm: 1.5,
                direction: AssistantCadShellDirection::Symmetric,
            },
        },
    ]);

    assert_eq!(input.validate(), Ok(()));
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        preview.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::SurfaceBody(SurfaceBodySpec::Planar {
            profile: FeatureId(1)
        })
    ));
    assert!(matches!(
        preview.feature(FeatureId(5)).unwrap().kind(),
        FeatureKind::SurfaceTrim {
            target: FeatureId(3),
            cutter: FeatureId(4)
        }
    ));
    assert!(matches!(
        preview.feature(FeatureId(7)).unwrap().kind(),
        FeatureKind::SurfaceKnit { surfaces, make_solid: false, .. }
            if surfaces == &[FeatureId(4), FeatureId(6)]
    ));
    assert_eq!(
        preview.feature(FeatureId(8)).unwrap().kind().body_kind(),
        Some(BodyKind::Solid)
    );
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), 2);

    let committed = document.current();
    let invalid = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0,
        name: "Invalid solid extension".into(),
        feature: AssistantCadBodyFeature::SurfaceExtend {
            target_feature_id: FeatureId(8).0.into(),
            distance_mm: 1.0,
        },
    }]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &invalid,
        )
        .is_err()
    );
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn public_assistant_cam_setup_is_exact_bound_atomic_undoable_and_fail_closed() {
    let mut document = seeded();
    let snapshot = document.current();
    let target = snapshot
        .features()
        .find(|feature| feature.kind().body_kind() == Some(BodyKind::Solid))
        .unwrap();
    let definition_id = target.definition_id().0;
    let feature_id = target.id().0;
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    let input = program(vec![cam_setup_operation(definition_id, feature_id)]);

    assert_eq!(input.validate(), Ok(()));
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(document.current().canonical_digest(), before_digest);
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    let committed = document.current();
    let cam = committed
        .cam_plan(ketchup_core::cam::CamPlanId(1))
        .expect("Assistant setup must publish one canonical CAM plan")
        .clone();
    assert_eq!(cam.name(), "Reviewed top setup");
    assert_eq!(cam.target().definition_id.0, definition_id);
    assert_eq!(cam.target().feature_id.0, feature_id);
    assert_eq!(cam.tool().spindle_rpm, 12_000);
    assert_eq!(
        cam.setup().work_offset,
        ketchup_core::cam::CamWorkOffset::G54
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert!(
        document
            .current()
            .cam_plan(ketchup_core::cam::CamPlanId(1))
            .is_none()
    );
    document.redo().unwrap();

    let mut invalid = cam_setup_operation(definition_id, feature_id);
    if let AssistantCadEditOperation::UpsertCamPlan { stepover_ratio, .. } = &mut invalid {
        *stepover_ratio = 1.2;
    }
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![invalid]),
        )
        .is_err()
    );
    assert_eq!(
        document.current().cam_plan(ketchup_core::cam::CamPlanId(1)),
        Some(&cam)
    );
}
