use super::*;

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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Exact mechanism body".into(),
                kind: FeatureKind::extrusion(PROFILE, Dimension::from_decimal("10").unwrap()),
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
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: CHILD,
                definition_id: DEFINITION,
                name: "Nested slider".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: Some(INNER_GROUP),
                tags: Default::default(),
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
                tags: Default::default(),
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
            .assembly_joint(ketchup_model::assembly_joint::AssemblyJointId(1))
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
        holes: Vec::new(),
        pockets: Vec::new(),
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
        ketchup_geometry::sketch::SketchEntity::CubicBezier { .. }
    )));
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn public_large_ellipse_has_a_visible_bounded_deviation_and_reopens_exactly() {
    let mut document = DocumentStore::new();
    let baseline_digest = document.current().canonical_digest();
    let ellipse = |radius_x_mm, radius_y_mm, maximum_deviation_mm| {
        program(vec![AssistantCadEditOperation::CreatePart {
            holes: Vec::new(),
            pockets: Vec::new(),
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
        let ketchup_geometry::sketch::SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } = entity
        else {
            panic!("ellipse approximation must remain editable cubic geometry");
        };
        let curve = ketchup_geometry::linalg::CubicBezier::new([
            *start_mm,
            *control_1_mm,
            *control_2_mm,
            *end_mm,
        ]);
        for step in 0..=4_096 {
            let point = curve.eval(f64::from(step) / 4_096.0);
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
        holes: Vec::new(),
        pockets: Vec::new(),
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
            .filter(|entity| matches!(entity, ketchup_geometry::sketch::SketchEntity::Line { .. }))
            .count(),
        4
    );
    assert_eq!(
        sketch
            .entities
            .iter()
            .filter(|entity| matches!(entity, ketchup_geometry::sketch::SketchEntity::Arc { .. }))
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
        definition_id: 1.into(),
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
        ketchup_geometry::sketch::SketchEntity::Line { .. }
            | ketchup_geometry::sketch::SketchEntity::Arc { .. }
    )));

    let exact_document = DocumentStore::new();
    let exact_input = program(vec![AssistantCadEditOperation::CreatePart {
        holes: Vec::new(),
        pockets: Vec::new(),
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(definition),
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(definition),
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(definition),
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
            definition_id: 1.into(),
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
    let AssistantCadEditOperation::CreateSketch {
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
        let AssistantCadEditOperation::CreateSketch {
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
            profile: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 0.5,
            }],
        },
        AssistantCadEditOperation::CreateHelix {
            name: "Arbitrary axis thread".into(),
            parameters: AssistantHelixParameters {
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
            profile: line_loop(&[[-0.65, -0.468], [0.65, 0.0], [-0.65, 0.468]]),
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
fn helix_sweeps_any_closed_profile_that_fits_between_its_turns() {
    let document = DocumentStore::new();
    let helix = |pitch_mm, profile| {
        program(vec![AssistantCadEditOperation::CreateHelix {
            name: "Buttress thread".into(),
            parameters: AssistantHelixParameters {
                pitch_mm,
                ..AssistantHelixParameters::default()
            },
            profile,
        }])
    };
    // A buttress tooth: a steep flank, a flat crest and a curved root.
    let mut tooth = line_loop(&[[-1.0, -1.2], [1.0, -0.3], [1.0, 0.3], [-1.0, 0.4]]);
    tooth[3] = AssistantSketchEntity::Arc {
        id: 4,
        start_mm: [-1.0, 0.4],
        end_mm: [-1.0, -1.2],
        center_mm: [-1.0, -0.4],
        clockwise: false,
    };
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&helix(3.0, tooth.clone())).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let candidate = document.preview_batch(&batch).unwrap();
    let FeatureKind::Profile { segments, closed } = candidate.feature(FeatureId(1)).unwrap().kind()
    else {
        panic!("the swept profile is the drawn one");
    };
    assert!(closed);
    assert_eq!(segments.len(), 4);
    assert!(matches!(
        segments[3],
        ketchup_model::document::ProfileSegment::CircularArc {
            center_mm: [-1.0, -0.4],
            ..
        }
    ));
    // The helix axis holds the tooth in the axial section on every turn.
    assert!(matches!(
        candidate.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Sweep {
            up: Some([0.0, 0.0, 1.0]),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(3)).unwrap();

    // 2.8 mm wide with its round root: the next turn would cut into it at a 2 mm pitch.
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &helix(2.0, tooth.clone()),
        )
        .is_err()
    );
    let open = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &helix(3.0, tooth[..3].to_vec()),
    );
    if let Ok(batch) = open {
        assert!(
            document.preview_batch(&batch).is_err(),
            "an open chain is no profile"
        );
    }
    let copies = vec![AssistantSketchEntity::ProfileCopies {
        source_entities: tooth,
        copies: Vec::new(),
    }];
    assert!(helix(3.0, copies).validate().is_err());
}

#[test]
fn helix_path_is_a_persistent_non_solid_construction_feature() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateHelix {
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
        profile: Vec::new(),
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
        FeatureKind::Sweep { .. } | FeatureKind::Profile { .. }
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
        AssistantCadEditOperation::CreateHelix {
            name: "Referenced Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
            profile: Vec::new(),
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
    let existing_program = program(vec![AssistantCadEditOperation::CreateHelix {
        name: "Existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis { axis: 1.into() }),
        profile: Vec::new(),
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
        AssistantCadEditOperation::CreateHelix {
            name: "Invalid Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
            profile: Vec::new(),
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
    let invalid_existing = program(vec![AssistantCadEditOperation::CreateHelix {
        name: "Invalid existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis { axis: 1.into() }),
        profile: Vec::new(),
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
        holes: Vec::new(),
        pockets: Vec::new(),
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(definition_output),
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
        ketchup_geometry::sketch::WorkplaneSupport::ConstructionPlane {
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
        FeatureKind::Workplane(ketchup_geometry::sketch::WorkplaneSpec {
            support: ketchup_geometry::sketch::WorkplaneSupport::ConstructionPlane {
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(definition_output),
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Target body".into(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::new("10", 10.0).unwrap()),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "First opening".into(),
                kind: FeatureKind::polygon(&[[2.0, 2.0], [12.0, 2.0], [12.0, 12.0], [2.0, 12.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Second opening".into(),
                kind: FeatureKind::polygon(&[
                    [20.0, 10.0],
                    [30.0, 10.0],
                    [30.0, 20.0],
                    [20.0, 20.0],
                ]),
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
            definition_id: 1.into(),
            name: "First pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2.into(),
                profile_feature_id: 3.into(),
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
            name: "Second pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2.into(),
                profile_feature_id: 4.into(),
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1.into(),
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Adjustable body".into(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::new("10", 10.0).unwrap()),
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
        definition_id: 1.into(),
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
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. }) if height.millimetres() == 20.0
    ));
    assert!(matches!(
        candidate.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: ketchup_model::document::BooleanOperation::Intersect,
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
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
        AssistantCadEditOperation::AppendFeature {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
            name: "Opening".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    0,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                )),
                profile_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    1,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )),
                depth_mm: 10.0,
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
    assert_eq!(batch.commands().len(), 8);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(6)).unwrap().kind(),
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(FeatureId(5)),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::Cut {
                target: FeatureId(3),
                ..
            },
            ..
        })
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
            holes: Vec::new(),
            pockets: Vec::new(),
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
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
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
        AssistantCadEditOperation::AppendFeature {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
            name: "Left pocket".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    0,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                )),
                profile_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    1,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )),
                depth_mm: 4.0,
            },
        },
        AssistantCadEditOperation::CreateSketch {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
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
        AssistantCadEditOperation::AppendFeature {
            definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                0,
                AssistantCadProgramFeatureOutput::Definition,
            )),
            name: "Right pocket".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    2,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                )),
                profile_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                    3,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )),
                depth_mm: 4.0,
            },
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
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(FeatureId(8)),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::Cut {
                target: FeatureId(6),
                ..
            },
            ..
        })
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(9)).unwrap();
}

#[test]
fn create_part_preserves_through_intent_and_old_hole_json_defaults_to_blind() {
    let legacy: AssistantPartHole = serde_json::from_value(serde_json::json!({
        "id": "bore", "entry_local_mm": [20,20,0],
        "inward_unit_local": [0,0,1], "diameter_mm": 8, "depth_mm": 18
    }))
    .unwrap();
    assert!(!legacy.through);
    for (through, depth, expected_through) in [
        (false, 18.0, false),
        (true, 18.0, true),
        (true, 22.0, true),
        (true, 12.0, false),
        (false, 22.0, false),
    ] {
        let mut document = DocumentStore::new();
        let part = cuboid_part(
            "stock".into(),
            [100.0, 50.0, 18.0],
            vec![AssistantPartHole {
                through,
                depth_mm: depth,
                ..legacy.clone()
            }],
            Vec::new(),
            [0.0; 3],
            None,
        );
        let batch = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![part]),
        )
        .unwrap();
        document.apply_batch(&batch).unwrap();
        let snapshot = document.current();
        let graph =
            ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(1), FeatureId(6)).unwrap();
        if through && depth < 18.0 {
            assert!(matches!(
                graph.nodes.last().unwrap().operation,
                ketchup_model::exact_brep_graph::ExactBRepOperation::Boolean { .. }
            ));
        } else {
            let ketchup_model::exact_brep_graph::ExactBRepOperation::ProfileCut {
                depth_bits, ..
            } = &graph.nodes.last().unwrap().operation
            else {
                panic!("expected profile cut")
            };
            assert_eq!(
                depth_bits.map(f64::from_bits),
                (!expected_through).then_some(depth)
            );
        }
    }
}

#[test]
fn one_part_operation_creates_named_physical_holes_and_one_local_edit_moves_one_hole() {
    let mut document = DocumentStore::new();
    let panel = cuboid_part(
        "Side panel".into(),
        [100.0, 50.0, 18.0],
        vec![AssistantPartHole {
            id: "pin-1".into(),
            entry_local_mm: [20.0, 20.0, 0.0],
            inward_unit_local: [0.0, 0.0, 1.0],
            diameter_mm: 8.0,
            depth_mm: 16.0,
            through: false,
            thread: None,
        }],
        Vec::new(),
        [0.0, 0.0, 0.0],
        None,
    );
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
        "Side panel hole pin-1"
    );
    assert_eq!(
        before_move.feature(FeatureId(6)).unwrap().name(),
        "Side panel hole pin-1 pocket"
    );
    ExactBRepGraph::from_snapshot(&before_move, DefinitionId(1), FeatureId(6)).unwrap();

    let move_hole = AssistantCadEditOperation::SetFeatureParameter {
        feature_id: 5,
        parameter_path: "entities.1.center.x".into(),
        value_type: AssistantCadParameterValueType::Length,
        value: 20.0.into(),
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
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(FeatureId(5)),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::Cut {
                target: FeatureId(3),
                ..
            },
            ..
        })
    ));
    ExactBRepGraph::from_snapshot(&after_move, DefinitionId(1), FeatureId(6)).unwrap();
}

#[test]
fn create_part_drills_into_the_bounding_faces_of_any_extruded_profile() {
    let cylinder = |holes| AssistantCadEditOperation::CreatePart {
        name: "Round part".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 30.0,
        }],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 20.0 },
        holes,
        pockets: Vec::new(),
        translation_mm: [0.0, 0.0, 0.0],
        rotation: None,
    };
    let hole = |entry_local_mm, inward_unit_local| AssistantPartHole {
        id: "axis".into(),
        entry_local_mm,
        inward_unit_local,
        diameter_mm: 6.0,
        depth_mm: 10.0,
        through: false,
        thread: None,
    };
    let mut document = DocumentStore::new();
    let baseline = document.current();
    // The body spans -30..30 in x and y, so a hole at negative x is on its top face.
    for outside in [
        hole([45.0, 0.0, 20.0], [0.0, 0.0, -1.0]),
        hole([-10.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ] {
        let error = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![cylinder(vec![outside])]),
        )
        .unwrap_err();
        assert_eq!(error.code, "planning.cad_part_cut_outside");
        assert_eq!(
            document.current().canonical_digest(),
            baseline.canonical_digest()
        );
    }
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![cylinder(vec![hole(
            [-10.0, 0.0, 20.0],
            [0.0, 0.0, -1.0],
        )])]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    assert_eq!(document.visible_undo_steps(), 1);
    let names = snapshot
        .features()
        .map(|feature| feature.name().to_owned())
        .collect::<Vec<_>>();
    assert!(
        names.contains(&"Round part hole axis".to_owned()),
        "{names:?}"
    );
    let pocket = snapshot
        .features()
        .find(|feature| feature.name() == "Round part hole axis pocket")
        .unwrap();
    ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(1), pocket.id()).unwrap();
}

#[test]
fn one_pin_joint_operation_derives_matching_sixteen_millimetre_holes_for_both_panels() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            Vec::new(),
            Vec::new(),
            translation_mm,
            None,
        )
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

    let face = |occurrence_id, face_z, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::CreatePinJoint {
            joint_id: None,
            name: "Shelf row".into(),
            first: face(1, 18.0, [0.0, 0.0, -1.0]),
            second: face(2, 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 25.0,
            pin: AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
            holes: AssistantPinHoles::Logical,
        }]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    let projection = ketchup_model::pin_joint::project_pin_joint_contract(
        &snapshot,
        snapshot
            .pin_joint(ketchup_model::pin_joint::PinJointId(1))
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
fn physical_pin_split_fits_a_thin_face_to_board_end_corner_in_one_program() {
    let face = |occurrence_id, origin, inward, maximum| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: origin,
        inward_unit_local: inward,
        bounds_min_local_mm: [0.0; 3],
        bounds_max_local_mm: maximum,
    };
    let corner = |first_insertion_mm| {
        program(vec![
            cuboid_part(
                "Front 15".into(),
                [100.0, 15.0, 60.0],
                Vec::new(),
                Vec::new(),
                [0.0, 0.0, 0.0],
                None,
            ),
            cuboid_part(
                "Side 15".into(),
                [15.0, 50.0, 60.0],
                Vec::new(),
                Vec::new(),
                [0.0, 15.0, 0.0],
                None,
            ),
            AssistantCadEditOperation::CreatePinJoint {
                joint_id: None,
                name: "Corner".into(),
                first: face(2, [0.0; 3], [0.0, 1.0, 0.0], [15.0, 50.0, 60.0]),
                second: face(1, [0.0, 15.0, 0.0], [0.0, -1.0, 0.0], [100.0, 15.0, 60.0]),
                first_center_local_mm: [7.5, 0.0, 15.0],
                row_unit_first_local: [0.0, 0.0, 1.0],
                count: 2,
                spacing_mm: 30.0,
                pin: AssistantPin {
                    diameter_mm: 8.0,
                    length_mm: 30.0,
                    hole_clearance_mm: 1.0,
                },
                holes: AssistantPinHoles::Drill { first_insertion_mm },
            },
        ])
    };
    let mut document = DocumentStore::new();
    let empty = BTreeSet::new();
    let exact = ExactResultRegistry::default();
    // Half of a 30 mm pin plus clearance would drill through the 15 mm face.
    assert!(plan(&document, &empty, &exact, &corner(None)).is_err());
    assert!(plan(&document, &empty, &exact, &corner(Some(30.0))).is_err());

    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&corner(Some(20.0))).unwrap()).unwrap();
    let batch = plan(&document, &empty, &exact, &input).unwrap();
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    let committed = document.current();
    let joint = committed.pin_joints().next().unwrap();
    assert_eq!(joint.pin.first_insertion_mm, 20.0);
    assert_eq!(joint.pin.second_insertion_mm, 10.0);
    let projection =
        ketchup_model::pin_joint::project_pin_joint_contract(&committed, joint).unwrap();
    assert_eq!(projection.pairs.len(), 2);
    assert!(projection.pairs.iter().all(|pair| {
        pair.first.depth_mm == 21.0
            && pair.second.depth_mm == 11.0
            && pair
                .physical_probe_coincidence
                .as_ref()
                .is_some_and(|probe| probe.maximum_endpoint_error_mm < 1e-9)
    }));
}
