use super::*;

#[test]
fn parameter_paths_are_general_bounded_and_canonical() {
    let path = ParameterPath::new("features.custom.wall_thickness").unwrap();
    assert_eq!(path.as_str(), "features.custom.wall_thickness");
    assert_eq!(ParameterPath::new(""), Err(ParameterPathError::Empty));
    assert_eq!(
        ParameterPath::new("features..height"),
        Err(ParameterPathError::InvalidSegment)
    );
    assert_eq!(
        ParameterPath::new("features.Height"),
        Err(ParameterPathError::InvalidSegment)
    );
    assert_eq!(
        ParameterPath::new("x".repeat(257)),
        Err(ParameterPathError::TooLong)
    );
}

#[test]
fn parameter_descriptors_are_derived_from_feature_and_sketch_structure() {
    let profile = FeatureKind::polygon(&[[0.0, 0.0], [4.0, 2.0]]);
    assert_eq!(
        profile
            .parameter_descriptors()
            .iter()
            .map(|descriptor| (descriptor.path().as_str(), descriptor.value_type()))
            .collect::<Vec<_>>(),
        vec![
            ("segments.0.start.x", ParameterValueType::Length),
            ("segments.0.start.y", ParameterValueType::Length),
            ("segments.1.start.x", ParameterValueType::Length),
            ("segments.1.start.y", ParameterValueType::Length),
        ]
    );

    let revolve = FeatureKind::Revolve {
        profile: FeatureId(1),
        axis_start_mm: [0.0, 0.0],
        axis_end_mm: [0.0, 1.0],
        angle_degrees: 180.0,
    };
    let revolve_descriptors = revolve.parameter_descriptors();
    assert_eq!(revolve_descriptors.len(), 5);
    assert_eq!(revolve_descriptors.last().unwrap().path().as_str(), "angle");
    assert_eq!(
        revolve_descriptors.last().unwrap().value_type(),
        ParameterValueType::Angle
    );

    let center = SketchPointRef {
        entity: SketchEntityId(7),
        point: SketchPointKind::Center,
    };
    let sketch = FeatureKind::Sketch(SketchSpec {
        workplane: FeatureId(1),
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(7),
            center_mm: [3.0, 4.0],
            radius_mm: 2.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(8),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(7),
                    value: height("2"),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(9),
                kind: SketchConstraintKind::FixedPoint {
                    point: center,
                    position_mm: [3.0, 4.0],
                },
            },
        ],
    });
    assert_eq!(
        sketch
            .parameter_descriptors()
            .iter()
            .map(|descriptor| descriptor.path().as_str())
            .collect::<Vec<_>>(),
        vec![
            "entities.7.center.x",
            "entities.7.center.y",
            "entities.7.radius",
            "constraints.8.value",
            "constraints.9.position.x",
            "constraints.9.position.y",
        ]
    );
}

#[test]
fn product_workflow_uses_one_revision_store_and_read_only_scene_queries() {
    let mut document = seed_product_document();
    let initial_digest = document.current().canonical_digest();
    let scene = document.current().scene_query();
    assert_eq!(scene.len(), 2);
    assert!(scene.iter().all(|item| item.shared_occurrence_count == 2));
    assert_eq!(
        document
            .current()
            .world_transform_for_occurrence(FIRST)
            .unwrap()
            .matrix()[3],
        100.0
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: SECOND,
                transform: Transform::from_translation(900.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let moved_digest = document.current().canonical_digest();
    assert_ne!(moved_digest, initial_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), moved_digest);
}

#[test]
fn make_unique_clones_features_and_repoints_only_one_occurrence() {
    let mut document = seed_product_document();
    document.make_unique(SECOND, "Base Cabinet unique").unwrap();

    let snapshot = document.current();
    assert_eq!(snapshot.occurrence(FIRST).unwrap().definition_id(), CABINET);
    assert_eq!(
        snapshot.occurrence(SECOND).unwrap().definition_id(),
        DefinitionId(2)
    );
    assert_eq!(
        snapshot.definition(DefinitionId(2)).unwrap().feature_ids(),
        &[FeatureId(12), FeatureId(13)]
    );
    assert!(matches!(
        snapshot.feature(FeatureId(13)).unwrap().kind(),
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(FeatureId(12)),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::NewBody,
            ..
        })
    ));
    assert_eq!(snapshot.scene_query()[0].shared_occurrence_count, 1);
    assert_eq!(snapshot.scene_query()[1].shared_occurrence_count, 1);
}

#[test]
fn product_schema_round_trip_preserves_identity_hierarchy_values_and_digest() {
    let mut document = seed_product_document();
    document.make_unique(SECOND, "Base Cabinet unique").unwrap();
    let expected = document.current();
    let loaded = persistence::load(&persistence::save(&expected)).unwrap();

    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(loaded.migration_losses().is_empty());
    let actual = loaded.snapshot();
    assert_eq!(actual.document_id(), expected.document_id());
    assert_eq!(actual.units(), expected.units());
    assert_eq!(actual.canonical_digest(), expected.canonical_digest());
    assert_eq!(actual.occurrence(FIRST).unwrap().parent(), Some(GROUP));
    assert_eq!(actual.group(GROUP).unwrap().name(), "Kitchen run");
    assert_eq!(
        actual.world_transform_for_occurrence(FIRST),
        expected.world_transform_for_occurrence(FIRST)
    );
    assert_eq!(
        actual.feature(EXTRUSION).unwrap().kind(),
        expected.feature(EXTRUSION).unwrap().kind()
    );

    // The same document as written by schema 15 and by schema 9.
    let schema_fifteen = include_bytes!("../fixtures/persistence/legacy/product-schema15.bin");
    let previous_current = persistence::load(schema_fifteen).unwrap();
    assert_eq!(previous_current.source_schema(), 15);
    assert_eq!(
        previous_current.snapshot().canonical_digest(),
        with_document_id(&expected, previous_current.snapshot().document_id()).canonical_digest()
    );

    let schema_nine = include_bytes!("../fixtures/persistence/legacy/product-schema9.bin");
    let legacy_current = persistence::load(schema_nine).unwrap();
    assert_eq!(legacy_current.source_schema(), 9);
    assert_eq!(
        legacy_current.snapshot().canonical_digest(),
        with_document_id(&expected, legacy_current.snapshot().document_id()).canonical_digest()
    );
}

#[test]
fn invalid_product_batch_rolls_back_without_partial_entities() {
    let mut document = seed_product_document();
    let before = document.current().canonical_digest();
    let steps = document.visible_undo_steps();
    let error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateGroup {
            id: GroupId(31),
            name: "Temporary".to_owned(),
            transform: Transform::identity(),
            parent: None,
        },
        CanonicalCommand::SetGroupParent {
            id: GROUP,
            parent: Some(GroupId(31)),
        },
        CanonicalCommand::SetGroupParent {
            id: GroupId(31),
            parent: Some(GROUP),
        },
    ])) {
        Ok(_) => panic!("cyclic batch must fail"),
        Err(error) => error,
    };

    assert!(matches!(error, CanonicalError::GroupCycle(_)));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), steps);
    assert!(document.current().group(GroupId(31)).is_none());
}

#[test]
fn profile_parameter_edit_and_history_baseline_are_canonical() {
    let mut document = seed_product_document();
    document.discard_history_before_current();
    assert_eq!(document.visible_undo_steps(), 0);
    assert_eq!(document.visible_redo_steps(), 0);

    let resized = vec![[0.0, 0.0], [650.0, 0.0], [650.0, 580.0], [0.0, 580.0]];
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: resized.clone(),
            },
        ]))
        .unwrap();

    assert!(matches!(
        document.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &resized
    ));
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert!(matches!(
        document.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm[1][0] == 600.0
    ));
}

#[test]
fn closed_profile_contract_preserves_exact_points_and_rejects_invalid_batches_atomically() {
    let mut document = seed_product_document();
    document.discard_history_before_current();
    let exact_points = vec![
        [-125.25, 40.5],
        [300.125, 40.5],
        [300.125, 240.75],
        [-125.25, 240.75],
    ];
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: exact_points.clone(),
            },
        ]))
        .unwrap();
    assert!(matches!(
        document.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &exact_points
    ));

    let valid_digest = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let invalid_profiles = [
        vec![[0.0, 0.0], [10.0, 0.0]],
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]],
        vec![[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]],
        vec![[0.0, 0.0], [1.0e-10, 0.0], [10.0, 10.0], [0.0, 10.0]],
        vec![
            [0.0, 0.0],
            [1_000_000.001, 0.0],
            [1_000_000.001, 10.0],
            [0.0, 10.0],
        ],
    ];
    for points_mm in invalid_profiles {
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(99),
                    name: "Must roll back".to_owned(),
                },
                CanonicalCommand::SetProfilePoints {
                    id: PROFILE,
                    points_mm,
                },
            ]))
            .err()
            .expect("invalid closed profile must reject the entire batch");
        assert_eq!(error, CanonicalError::InvalidProfile);
        assert_eq!(document.current().canonical_digest(), valid_digest);
        assert_eq!(document.visible_undo_steps(), undo_steps);
        assert!(document.current().definition(DefinitionId(99)).is_none());
    }

    let clockwise = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]];
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: clockwise.clone(),
            },
        ]))
        .expect("loop direction does not make a closed profile invalid");
    assert!(matches!(
        document.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &clockwise
    ));
}

#[test]
fn general_revolve_is_canonical_atomic_cloneable_and_losslessly_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(60);
    const PROFILE: FeatureId = FeatureId(61);
    const REVOLVE: FeatureId = FeatureId(62);
    const OCCURRENCE: OccurrenceId = OccurrenceId(63);
    const AXIS_START: [f64; 2] = [0.0, -10.0];
    const AXIS_END: [f64; 2] = [0.0, 50.0];
    const ANGLE: f64 = 225.0;

    let profile = vec![[10.0, 0.0], [30.0, 0.0], [30.0, 40.0], [10.0, 40.0]];
    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "General revolve".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Closed profile".to_owned(),
                kind: FeatureKind::polygon(&profile),
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: DEFINITION,
                name: "225 degree revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: PROFILE,
                    axis_start_mm: AXIS_START,
                    axis_end_mm: AXIS_END,
                    angle_degrees: ANGLE,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Revolved body".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let applied_digest = document.current().canonical_digest();
    assert_ne!(applied_digest, empty_digest);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied_digest);

    for (axis_start_mm, axis_end_mm, angle_degrees) in [
        ([0.0, 0.0], [0.0, 0.0], 90.0),
        ([f64::NAN, 0.0], [0.0, 1.0], 90.0),
        ([0.0, 0.0], [0.0, 1.0], 0.0),
        ([0.0, 0.0], [0.0, 1.0], 360.000_001),
        ([0.0, 0.0], [0.0, 1.0], f64::NAN),
    ] {
        let before = document.current().canonical_digest();
        let undo_steps = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(99),
                definition_id: DEFINITION,
                name: "Invalid revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: PROFILE,
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                },
            }]))
            .err()
            .expect("invalid revolve must reject the entire batch");
        assert_eq!(error, CanonicalError::InvalidRevolve);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), undo_steps);
    }

    document.make_unique(OCCURRENCE, "Unique revolve").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    assert_eq!(unique_features, &[FeatureId(63), FeatureId(64)]);
    assert!(matches!(
        unique.feature(FeatureId(64)).unwrap().kind(),
        FeatureKind::Revolve {
            profile: FeatureId(63),
            axis_start_mm: AXIS_START,
            axis_end_mm: AXIS_END,
            angle_degrees: ANGLE,
        }
    ));

    let unique_digest = unique.canonical_digest();
    let loaded = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(loaded.migration_losses().is_empty());
    assert_eq!(loaded.snapshot().canonical_digest(), unique_digest);
    assert!(matches!(
        loaded.snapshot().feature(FeatureId(64)).unwrap().kind(),
        FeatureKind::Revolve {
            profile: FeatureId(63),
            axis_start_mm: AXIS_START,
            axis_end_mm: AXIS_END,
            angle_degrees: ANGLE,
        }
    ));

    let digest_for = |axis_end_mm, angle_degrees| {
        let mut candidate = DocumentStore::new();
        candidate
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "General revolve".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: PROFILE,
                    definition_id: DEFINITION,
                    name: "Closed profile".to_owned(),
                    kind: FeatureKind::polygon(&profile),
                },
                CanonicalCommand::CreateFeature {
                    id: REVOLVE,
                    definition_id: DEFINITION,
                    name: "225 degree revolve".to_owned(),
                    kind: FeatureKind::Revolve {
                        profile: PROFILE,
                        axis_start_mm: AXIS_START,
                        axis_end_mm,
                        angle_degrees,
                    },
                },
            ]))
            .unwrap();
        candidate.current().canonical_digest()
    };
    assert_ne!(digest_for(AXIS_END, ANGLE), digest_for([1.0, 50.0], ANGLE));
    assert_ne!(digest_for(AXIS_END, ANGLE), digest_for(AXIS_END, 180.0));
}

#[test]
fn m6_invalid_profile_or_revolve_axis_rolls_back_atomically() {
    const BOTTLE: DefinitionId = DefinitionId(50);
    const PROFILE: FeatureId = FeatureId(51);
    const REVOLVE: FeatureId = FeatureId(52);
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE,
                name: "Invalid bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: BOTTLE,
                name: "Offset half-profile".to_owned(),
                kind: FeatureKind::polygon(&[[1.0, 0.0], [20.0, 0.0], [20.0, 100.0], [1.0, 100.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: BOTTLE,
                name: "Invalid revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: PROFILE,
                    axis_start_mm: [5.0, 5.0],
                    axis_end_mm: [5.0, 5.0],
                    angle_degrees: 360.0,
                },
            },
        ]))
        .err()
        .expect("zero-length revolve axis must fail");
    assert_eq!(error, CanonicalError::InvalidRevolve);
    assert_eq!(document.current().canonical_digest(), before);
    assert!(document.current().definition(BOTTLE).is_none());

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE,
                name: "Bottle".to_owned(),
            },
        ]))
        .unwrap();
    let before_profile = document.current().canonical_digest();
    let error = document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: BOTTLE,
            name: "Self-intersecting profile".to_owned(),
            kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 100.0], [20.0, 0.0], [0.0, 100.0]]),
        }]))
        .err()
        .expect("self-intersecting profile must fail");
    assert_eq!(error, CanonicalError::InvalidProfile);
    assert_eq!(document.current().canonical_digest(), before_profile);
    assert!(document.current().feature(PROFILE).is_none());
}

#[test]
fn cyclic_group_conversion_rejects_without_publishing_partial_groups() {
    use ketchup_model::document::ConvertGroupPlan;
    let mut document = seed_product_document();
    let before = document.current();
    for parent in [GroupId(42), GROUP] {
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(41),
                name: "First".into(),
                parent: Some(parent),
                transform: Transform::identity(),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(42),
                name: "Second".into(),
                parent: Some(GroupId(41)),
                transform: Transform::identity(),
            },
            CanonicalCommand::SetGroupParent {
                id: GROUP,
                parent: Some(GroupId(42)),
            },
            CanonicalCommand::ConvertGroupToComponent(ConvertGroupPlan::new(
                GROUP,
                DefinitionId(50),
                OccurrenceId(51),
                "Converted".into(),
            )),
        ]);
        assert!(document.apply_batch(&batch).is_err());
        let after = document.current();
        assert_eq!(
            after.groups().collect::<Vec<_>>(),
            before.groups().collect::<Vec<_>>()
        );
        assert_eq!(
            after.occurrences().collect::<Vec<_>>(),
            before.occurrences().collect::<Vec<_>>()
        );
        assert!(after.definition(DefinitionId(50)).is_none());
    }
}

#[test]
fn forward_group_references_can_still_be_converted_in_one_batch() {
    use ketchup_model::document::ConvertGroupPlan;
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(41),
                name: "Child".into(),
                parent: Some(GroupId(42)),
                transform: Transform::identity(),
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(42),
                name: "Parent".into(),
                parent: Some(GROUP),
                transform: Transform::identity(),
            },
            CanonicalCommand::ConvertGroupToComponent(ConvertGroupPlan::new(
                GROUP,
                DefinitionId(50),
                OccurrenceId(51),
                "Converted".into(),
            )),
        ]))
        .unwrap();
    let after = document.current();
    assert!(after.group(GROUP).is_none());
    assert!(after.group(GroupId(41)).is_none());
    assert!(after.definition(DefinitionId(50)).is_some());
    assert_eq!(
        after.occurrence(OccurrenceId(51)).unwrap().name(),
        "Converted"
    );
    document.undo().unwrap();
    assert!(document.current().group(GROUP).is_some());
    assert!(document.current().definition(DefinitionId(50)).is_none());
}

#[test]
fn local_groups_move_shared_children_and_reject_invalid_graphs_atomically() {
    let mut document = seed_product_document();
    let component = document
        .convert_group_to_component(GROUP, "Assembly")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(90),
                definition_id: component.component_definition_id,
                name: "Copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let before = document.current();
    let member = before.local_occurrences().next().unwrap().key();
    let key = LocalGroupKey {
        definition_id: component.component_definition_id,
        local_id: LocalGroupId(99),
    };
    let create = CanonicalCommand::CreateLocalGroup {
        key,
        name: "Inner".into(),
        transform: Transform::from_translation(0.0, 70.0, 0.0).unwrap(),
        parent: None,
    };
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            create.clone(),
            CanonicalCommand::SetLocalOccurrenceParent {
                key: member,
                parent: Some(key.local_id),
            },
        ]))
        .unwrap();
    let preview = document.preview_verified_proposal(&proposal).unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.commit_verified_proposal(&proposal).unwrap();
    let added = document.current();
    assert_eq!(added.scene_query(), preview.scene_query());
    assert!(document.commit_verified_proposal(&proposal).is_err());
    let positions = |snapshot: &Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .filter(|part| !part.instance_path.is_root())
            .map(|part| (part.occurrence_id, part.transform.transform_point([0.0; 3])))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(
        positions(&added)[&component.component_occurrence_id],
        [100.0, 70.0, 0.0]
    );
    assert_eq!(positions(&added)[&OccurrenceId(90)], [500.0, 70.0, 0.0]);
    let moved = CanonicalCommand::SetLocalGroupTransform {
        key,
        transform: Transform::from_translation(10.0, 20.0, 30.0).unwrap(),
    };
    let pending = document
        .prepare_proposal(CommandBatch::new(vec![moved.clone()]))
        .unwrap();
    let second_key = LocalGroupKey {
        local_id: LocalGroupId(100),
        ..key
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateLocalGroup {
                key: second_key,
                name: "Outer".into(),
                transform: Transform::identity(),
                parent: None,
            },
        ]))
        .unwrap();
    assert!(document.commit_verified_proposal(&pending).is_err());
    document
        .apply_batch(&CommandBatch::new(vec![
            moved.clone(),
            CanonicalCommand::SetLocalGroupParent {
                key,
                parent: Some(second_key.local_id),
            },
        ]))
        .unwrap();
    let transformed = document.current();
    assert_eq!(
        positions(&transformed)[&component.component_occurrence_id],
        [110.0, 20.0, 30.0]
    );
    assert_eq!(
        positions(&transformed)[&OccurrenceId(90)],
        [510.0, 20.0, 30.0]
    );
    assert_eq!(
        transformed.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    let missing = LocalGroupKey {
        local_id: LocalGroupId(999),
        ..key
    };
    for invalid in [
        create,
        CanonicalCommand::CreateLocalGroup {
            key: LocalGroupKey {
                local_id: LocalGroupId(member.local_id.0),
                ..key
            },
            name: "Collision".into(),
            transform: Transform::identity(),
            parent: None,
        },
        CanonicalCommand::CreateLocalGroup {
            key: LocalGroupKey {
                definition_id: DefinitionId(999),
                ..missing
            },
            name: "No owner".into(),
            transform: Transform::identity(),
            parent: None,
        },
        CanonicalCommand::SetLocalGroupParent {
            key,
            parent: Some(key.local_id),
        },
        CanonicalCommand::SetLocalGroupParent {
            key: second_key,
            parent: Some(key.local_id),
        },
        CanonicalCommand::SetLocalGroupParent {
            key,
            parent: Some(missing.local_id),
        },
        CanonicalCommand::SetLocalGroupParent {
            key: missing,
            parent: None,
        },
        CanonicalCommand::SetLocalGroupTransform {
            key: missing,
            transform: Transform::identity(),
        },
        CanonicalCommand::DeleteLocalGroup { key: missing },
        CanonicalCommand::DeleteLocalGroup { key },
        CanonicalCommand::DeleteLocalGroup { key: second_key },
    ] {
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetLocalGroupTransform {
                        key,
                        transform: Transform::identity()
                    },
                    invalid,
                ]))
                .is_err()
        );
        assert_eq!(document.current().scene_query(), transformed.scene_query());
        assert_eq!(
            document.current().definitions().collect::<Vec<_>>(),
            transformed.definitions().collect::<Vec<_>>()
        );
        assert_eq!(
            document.current().local_groups().collect::<Vec<_>>(),
            transformed.local_groups().collect::<Vec<_>>()
        );
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteLocalGroup { key },
            CanonicalCommand::DeleteLocalGroup { key: second_key },
            CanonicalCommand::SetLocalOccurrenceParent {
                key: member,
                parent: None,
            },
        ]))
        .unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    assert_eq!(document.current().local_groups().count(), 0);
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), transformed.scene_query());
    document.redo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
}

#[test]
fn local_member_creation_reparent_repoint_and_removal_update_every_copy() {
    let mut document = seed_product_document();
    document.make_unique(SECOND, "Other geometry").unwrap();
    let target = document
        .current()
        .occurrence(SECOND)
        .unwrap()
        .definition_id();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: GroupId(40),
            name: "Inner".into(),
            transform: Transform::from_translation(0.0, 70.0, 0.0).unwrap(),
            parent: Some(GROUP),
        }]))
        .unwrap();
    let component = document
        .convert_group_to_component(GROUP, "Assembly")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(90),
                definition_id: component.component_definition_id,
                name: "Copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let before = document.current();
    let group = before.local_groups().next().unwrap().key().local_id;
    let key = LocalOccurrenceKey {
        definition_id: component.component_definition_id,
        local_id: LocalOccurrenceId(99),
    };
    let create = CanonicalCommand::CreateLocalOccurrence {
        key,
        definition_id: CABINET,
        name: "Added".into(),
        transform: Transform::from_translation(10.0, 20.0, 30.0).unwrap(),
        parent: Some(group),
        tags: Default::default(),
        visible: true,
    };
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateLocalOccurrence {
                    key: LocalOccurrenceKey {
                        local_id: LocalOccurrenceId(group.0),
                        ..key
                    },
                    definition_id: CABINET,
                    name: "ID collision".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                }
            ]))
            .is_err()
    );
    assert_eq!(document.current().scene_query(), before.scene_query());
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![create]))
        .unwrap();
    let preview = document.preview_verified_proposal(&proposal).unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.commit_verified_proposal(&proposal).unwrap();
    let added = document.current();
    assert_eq!(added.scene_query(), preview.scene_query());
    assert!(document.commit_verified_proposal(&proposal).is_err());
    assert_eq!(document.current().scene_query(), added.scene_query());
    let positions = added
        .scene_query()
        .into_iter()
        .filter(|part| part.occurrence_name == "Added")
        .map(|part| (part.occurrence_id, part.transform.transform_point([0.0; 3])))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        positions[&component.component_occurrence_id],
        [110.0, 90.0, 30.0]
    );
    assert_eq!(positions[&OccurrenceId(90)], [510.0, 90.0, 30.0]);
    assert_eq!(
        added.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetLocalOccurrenceParent { key, parent: None },
            CanonicalCommand::RepointLocalOccurrence {
                key,
                definition_id: target,
            },
        ]))
        .unwrap();
    let changed = document.current();
    let members = changed
        .scene_query()
        .into_iter()
        .filter(|part| part.occurrence_name == "Added")
        .collect::<Vec<_>>();
    assert_eq!(members.len(), 2);
    for member in &members {
        assert_eq!(member.definition_id, target);
        assert_eq!(member.transform.matrix()[7], 20.0);
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteLocalOccurrence { key },
        ]))
        .unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    assert!(document.current().local_occurrence(key).is_none());
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), changed.scene_query());
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), added.scene_query());
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    for expected in [&added, &changed, &before] {
        document.redo().unwrap();
        assert_eq!(document.current().scene_query(), expected.scene_query());
    }
}

#[test]
fn deleting_a_component_definition_removes_its_members_and_a_used_part_is_refused() {
    let mut document = seed_product_document();
    document.make_unique(FIRST, "Inner geometry").unwrap();
    let inner = document
        .current()
        .occurrence(FIRST)
        .unwrap()
        .definition_id();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: GroupId(40),
            name: "Inner".into(),
            transform: Transform::identity(),
            parent: Some(GROUP),
        }]))
        .unwrap();
    let component = document
        .convert_group_to_component(GROUP, "Assembly")
        .unwrap();
    let before = document.current();
    assert!(before.local_groups().next().is_some());
    assert!(
        before
            .local_occurrences()
            .any(|o| o.definition_id() == inner)
    );

    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteDefinition { id: inner }
            ]))
            .err(),
        Some(CanonicalError::DefinitionInUse(inner))
    );
    assert_eq!(document.current().revision_id(), before.revision_id());

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: component.component_occurrence_id,
            },
            CanonicalCommand::DeleteDefinition {
                id: component.component_definition_id,
            },
        ]))
        .unwrap();
    let deleted = document.current();
    assert!(
        deleted
            .definition(component.component_definition_id)
            .is_none()
    );
    assert!(deleted.local_groups().next().is_none());
    assert!(deleted.local_occurrences().next().is_none());
    assert!(deleted.definition(inner).is_some());
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteDefinition { id: inner },
        ]))
        .unwrap();
    assert!(document.current().definition(inner).is_none());

    document.undo().unwrap();
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
}

#[test]
fn invalid_local_member_edits_leave_shared_geometry_and_history_intact() {
    let mut document = seed_product_document();
    let component = document
        .convert_group_to_component(GROUP, "Assembly")
        .unwrap();
    let before = document.current();
    let key = before.local_occurrences().next().unwrap().key();
    let missing = LocalOccurrenceKey {
        local_id: LocalOccurrenceId(999),
        ..key
    };
    let create = CanonicalCommand::CreateLocalOccurrence {
        key,
        definition_id: CABINET,
        name: "Duplicate".into(),
        transform: Transform::identity(),
        parent: None,
        tags: Default::default(),
        visible: true,
    };
    let invalid = [
        create,
        CanonicalCommand::RepointLocalOccurrence {
            key,
            definition_id: component.component_definition_id,
        },
        CanonicalCommand::RepointLocalOccurrence {
            key,
            definition_id: DefinitionId(999),
        },
        CanonicalCommand::SetLocalOccurrenceParent {
            key,
            parent: Some(LocalGroupId(999)),
        },
        CanonicalCommand::DeleteLocalOccurrence { key: missing },
        CanonicalCommand::SetLocalOccurrenceParent {
            key: missing,
            parent: None,
        },
        CanonicalCommand::RepointLocalOccurrence {
            key: missing,
            definition_id: CABINET,
        },
    ];
    for command in invalid {
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetLocalOccurrenceTransform {
                        key,
                        transform: Transform::from_translation(30.0, 40.0, 50.0).unwrap(),
                    },
                    command,
                ]))
                .is_err()
        );
        assert_eq!(document.current().scene_query(), before.scene_query());
        assert_eq!(
            document.current().definitions().collect::<Vec<_>>(),
            before.definitions().collect::<Vec<_>>()
        );
    }
    document.undo().unwrap();
    assert!(document.current().group(GROUP).is_some());
    document.redo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
}

#[test]
fn local_member_transform_is_shared_atomic_and_undoable() {
    let mut document = seed_product_document();
    let component = document
        .convert_group_to_component(GROUP, "Assembly")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(90),
                definition_id: component.component_definition_id,
                name: "Copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let before = document.current();
    let key = before.local_occurrences().next().unwrap().key();
    let change = CanonicalCommand::SetLocalOccurrenceTransform {
        key,
        transform: Transform::from_translation(20.0, 30.0, 40.0).unwrap(),
    };
    let missing = LocalOccurrenceKey {
        local_id: LocalOccurrenceId(999),
        ..key
    };
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                change.clone(),
                CanonicalCommand::SetLocalOccurrenceTransform {
                    key: missing,
                    transform: Transform::identity(),
                }
            ]))
            .err(),
        Some(CanonicalError::LocalOccurrenceNotFound(missing))
    );
    assert_eq!(document.current().scene_query(), before.scene_query());
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![change]))
        .unwrap();
    let preview = document.preview_verified_proposal(&proposal).unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.commit_verified_proposal(&proposal).unwrap();
    let after = document.current();
    assert_eq!(after.scene_query(), preview.scene_query());
    assert!(matches!(
        document.validate_proposal(&proposal),
        ketchup_model::document::ProposalValidity::Stale { .. }
    ));
    let translations = after
        .scene_query()
        .iter()
        .map(|part| {
            let m = part.transform.matrix();
            (part.occurrence_id, [m[3], m[7], m[11]])
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        translations[&component.component_occurrence_id],
        [120.0, 30.0, 40.0]
    );
    assert_eq!(translations[&OccurrenceId(90)], [520.0, 30.0, 40.0]);
    assert_eq!(translations[&SECOND], [700.0, 0.0, 0.0]);
    assert_eq!(
        after.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    assert_eq!(
        after.definitions().collect::<Vec<_>>(),
        before.definitions().collect::<Vec<_>>()
    );
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.redo().unwrap();
    assert_eq!(document.current().scene_query(), after.scene_query());
}

#[test]
fn renaming_a_local_member_is_atomic_and_updates_all_copies() {
    let mut document = seed_product_document();
    let component = document
        .convert_group_to_component(GROUP, "Shared")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(90),
                definition_id: component.component_definition_id,
                name: "Copy".into(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let before = document.current();
    let member = before.local_occurrences().next().unwrap();
    let key = member.key();
    let rename = CanonicalCommand::RenameLocalOccurrence {
        key,
        name: "Renamed".into(),
    };
    for bad in [
        CanonicalCommand::RenameLocalOccurrence {
            key,
            name: "".into(),
        },
        CanonicalCommand::RenameLocalOccurrence {
            key: LocalOccurrenceKey {
                local_id: LocalOccurrenceId(999),
                ..key
            },
            name: "Missing".into(),
        },
    ] {
        assert!(
            document
                .apply_batch(&CommandBatch::new(vec![rename.clone(), bad]))
                .is_err()
        );
        assert_eq!(document.current().scene_query(), before.scene_query());
    }
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![rename]))
        .unwrap();
    let preview = document.preview_verified_proposal(&proposal).unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.commit_verified_proposal(&proposal).unwrap();
    let after = document.current();
    assert_eq!(after.scene_query(), preview.scene_query());
    assert_eq!(after.local_occurrence(key).unwrap().name(), "Renamed");
    let scene = after.scene_query();
    let renamed = scene
        .iter()
        .filter(|p| p.occurrence_name == "Renamed")
        .collect::<Vec<_>>();
    assert_eq!(renamed.len(), 2);
    for leaf in renamed {
        let old = before.resolve_instance_path(&leaf.instance_path).unwrap();
        assert_eq!(leaf.transform, old.world_transform);
        assert_eq!(leaf.definition_id, old.definition_id);
    }
    assert!(matches!(
        document.validate_proposal(&proposal),
        ketchup_model::document::ProposalValidity::Stale { .. }
    ));
    document.undo().unwrap();
    assert_eq!(document.current().scene_query(), before.scene_query());
    document.redo().unwrap();
    assert_eq!(document.current().scene_query(), scene);
}

#[test]
fn bound_nested_scene_query_blocks_stale_hidden_and_out_of_context_entities() {
    let mut document = seed_product_document();
    let inner = document
        .convert_group_to_component(GROUP, "Inner component")
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(31),
                name: "Outer group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: inner.component_occurrence_id,
                parent: Some(GroupId(31)),
            },
        ]))
        .unwrap();
    let converted = document
        .convert_group_to_component(GroupId(31), "Scoped component")
        .unwrap();
    let definition_id = converted.component_definition_id;
    let anchor_id = converted.component_occurrence_id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(anchor_id.0 + 1),
                definition_id,
                name: "Visible sibling".to_owned(),
                transform: Transform::from_translation(500.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(anchor_id.0 + 2),
                definition_id,
                name: "Hidden sibling".to_owned(),
                transform: Transform::from_translation(1000.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: false,
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let anchor = InstancePath::root(anchor_id);
    let query = snapshot
        .bind_scene_query(SceneQueryContext::Definition {
            definition_id,
            instance_path: anchor.clone(),
        })
        .unwrap();
    let scoped = snapshot.scene_query_in(&query).unwrap();
    assert!(scoped.iter().all(|item| item.visible));
    assert!(
        scoped
            .iter()
            .all(|item| item.instance_path.root_occurrence() == anchor_id)
    );
    assert!(scoped.iter().any(|item| !item.instance_path.is_root()));
    assert!(scoped.iter().all(|item| {
        item.instance_path
            .steps()
            .iter()
            .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
            .count()
            <= 1
    }));
    assert!(
        scoped.iter().all(|item| item.shared_occurrence_count == 1),
        "sharing metadata must be recomputed inside the permitted scope"
    );
    assert_eq!(
        snapshot.bind_scene_query(SceneQueryContext::Definition {
            definition_id,
            instance_path: InstancePath::root(OccurrenceId(anchor_id.0 + 2)),
        }),
        Err(SceneQueryError::InvalidContext)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: anchor_id,
                transform: Transform::from_translation(25.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(
        document.current().scene_query_in(&query),
        Err(SceneQueryError::SnapshotMismatch)
    );
}

#[test]
fn nested_conversion_mapping_sharing_unique_history_and_schema_three_round_trip() {
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(99),
                name: "sole product evaluator".to_owned(),
                dimension: height("12.5"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateTag {
                id: TagId(7),
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(31),
                name: "Nested".to_owned(),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
                parent: Some(GROUP),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(22),
                definition_id: CABINET,
                name: "Tagged nested cabinet".to_owned(),
                transform: Transform::from_translation(5.0, 0.0, 0.0).unwrap(),
                parent: Some(GroupId(31)),
                tags: [TagId(7)].into(),
                visible: false,
            },
        ]))
        .unwrap();

    let result = document
        .convert_group_to_component(GROUP, "Converted run")
        .unwrap();
    assert_eq!(result.component_definition_id, DefinitionId(2));
    assert_eq!(result.component_occurrence_id, OccurrenceId(23));
    assert_eq!(result.mappings.len(), 5);
    assert!(matches!(
        result.resolve_old_path(&WorldEntityPath {
            groups: vec![GROUP, GroupId(31)],
            occurrence: Some(OccurrenceId(22)),
        }),
        MappingResolution::Resolved {
            new_id: ConvertedEntityId::LocalOccurrence(LocalOccurrenceKey {
                definition_id: DefinitionId(2),
                local_id: LocalOccurrenceId(22),
            }),
            ref new_path,
        } if new_path.steps() == [
            InstancePathStep::Group(LocalGroupId(31)),
            InstancePathStep::Occurrence(LocalOccurrenceId(22)),
        ]
    ));
    assert!(matches!(
        result.resolve_old_path(&WorldEntityPath {
            groups: vec![],
            occurrence: Some(SECOND),
        }),
        MappingResolution::Unresolved {
            reason: UnresolvedMappingReason::NotInConvertedGroup,
        }
    ));
    assert_eq!(result.unresolved_mappings().count(), 1);

    let nested = document
        .current()
        .scene_query()
        .into_iter()
        .find(|item| {
            item.instance_path.root_occurrence() == OccurrenceId(23)
                && item.instance_path.steps().last()
                    == Some(&InstancePathStep::Occurrence(LocalOccurrenceId(22)))
        })
        .unwrap();
    assert_eq!(nested.transform.matrix()[3], 115.0);
    assert!(!nested.visible);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(24),
                definition_id: DefinitionId(2),
                name: "Converted copy".to_owned(),
                transform: Transform::from_translation(1000.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    assert_eq!(document.current().local_occurrences().count(), 2);
    assert_eq!(
        document
            .current()
            .scene_query()
            .iter()
            .filter(|item| {
                matches!(
                    item.instance_path.steps().last(),
                    Some(InstancePathStep::Occurrence(LocalOccurrenceId(22)))
                )
            })
            .count(),
        2
    );

    let before_unique = document.current().canonical_digest();
    document
        .make_unique(OccurrenceId(24), "Unique run")
        .unwrap();
    let unique_digest = document.current().canonical_digest();
    assert!(
        document
            .current()
            .local_group(LocalGroupKey {
                definition_id: DefinitionId(3),
                local_id: LocalGroupId(31),
            })
            .is_some()
    );
    let unique_local = document
        .current()
        .local_occurrence(LocalOccurrenceKey {
            definition_id: DefinitionId(3),
            local_id: LocalOccurrenceId(22),
        })
        .unwrap()
        .clone();
    assert!(unique_local.tags().iter().eq([&TagId(7)]));
    assert!(!unique_local.visible());
    assert_eq!(unique_local.transform().matrix()[3], 5.0);
    assert_eq!(document.undo().unwrap().canonical_digest(), before_unique);
    assert_eq!(document.redo().unwrap().canonical_digest(), unique_digest);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    let snapshot = reopened.snapshot();
    assert_eq!(snapshot.canonical_digest(), unique_digest);
    assert_eq!(snapshot.evaluator_node_count(), 1);
    assert!(snapshot.evaluator_node(NodeId(99)).is_some());
    let reopened_local = snapshot
        .local_occurrence(LocalOccurrenceKey {
            definition_id: DefinitionId(3),
            local_id: LocalOccurrenceId(22),
        })
        .unwrap();
    assert!(reopened_local.tags().iter().eq([&TagId(7)]));
    assert!(!reopened_local.visible());
    assert_eq!(reopened_local.transform().matrix()[3], 5.0);
}

#[test]
fn conversion_collision_and_local_ownership_cycle_fail_atomically() {
    let mut collision = seed_product_document();
    collision
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(31),
                name: "Nested".to_owned(),
                transform: Transform::identity(),
                parent: Some(GROUP),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(31),
                definition_id: CABINET,
                name: "Colliding local ID".to_owned(),
                transform: Transform::identity(),
                parent: Some(GroupId(31)),
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let before = collision.current().canonical_digest();
    assert!(matches!(
        collision.convert_group_to_component(GROUP, "Invalid"),
        Err(CanonicalError::InvalidLocalGraph)
    ));
    assert_eq!(collision.current().canonical_digest(), before);

    let mut ownership = seed_product_document();
    ownership
        .convert_group_to_component(GROUP, "Owner")
        .unwrap();
    // Make the local occurrence inside component 2 instantiate component 2 itself.
    let bytes = rewrite_saved_snapshot(&persistence::save(&ownership.current()), |saved| {
        let product = cbor_entry(saved, "product");
        let local_occurrences = cbor_entry(product, "local_occurrences")
            .as_map_mut()
            .expect("a CBOR map");
        let (_, occurrence) = local_occurrences
            .iter_mut()
            .find(|(key, _)| {
                let mut key = key.clone();
                *cbor_entry(&mut key, "definition_id") == Value::from(2)
                    && *cbor_entry(&mut key, "local_id") == Value::from(FIRST.0)
            })
            .expect("local occurrence record is present");
        assert_eq!(
            *cbor_entry(occurrence, "definition_id"),
            Value::from(CABINET.0)
        );
        *cbor_entry(occurrence, "definition_id") = Value::from(2);
    });
    assert!(matches!(
        persistence::load(&bytes),
        Err(persistence::PersistenceError::InvalidCanonicalData(
            CanonicalError::InvalidLocalGraph
        ))
    ));
}

#[test]
fn sketch_constraint_parameters_use_the_generic_binding_and_recompute_contract() {
    const WORKPLANE: FeatureId = FeatureId(30);
    const SKETCH: FeatureId = FeatureId(31);
    const SOURCE: NodeId = NodeId(501);
    const RULE: NodeId = NodeId(502);
    let segment = SlotSegment::new(RULE, "dimensions", "radius").unwrap();
    let target =
        FeatureParameterTarget::new(SKETCH, "constraints.1.value", ParameterValueType::Length)
            .unwrap();
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: CABINET,
                name: "Sketch plane".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: CABINET,
                name: "Constrained circle".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [0.0, 0.0],
                        radius_mm: 2.0,
                    }],
                    constraints: vec![
                        SketchConstraint {
                            id: SketchConstraintId(1),
                            kind: SketchConstraintKind::Radius {
                                entity: SketchEntityId(1),
                                value: height("2"),
                            },
                        },
                        SketchConstraint {
                            id: SketchConstraintId(2),
                            kind: SketchConstraintKind::FixedPoint {
                                point: SketchPointRef {
                                    entity: SketchEntityId(1),
                                    point: SketchPointKind::Center,
                                },
                                position_mm: [0.0, 0.0],
                            },
                        },
                    ],
                }),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: SOURCE,
                name: "Radius source".to_owned(),
                dimension: height("3"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE,
                name: "Sketch radius rule".to_owned(),
                expression: "$501".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: target.clone(),
                derived_from: DerivedIdentity::new(RULE, SlotPath::new(vec![segment]).unwrap())
                    .unwrap(),
            }),
        ]))
        .unwrap();

    assert!(document.current().has_feature_parameter(&target));
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
                scope: FeatureParameterRecomputeScope::All,
            },
        ]))
        .unwrap();
    let recomputed = document.current().canonical_digest();
    assert_ne!(recomputed, before);
    let current = document.current();
    let FeatureKind::Sketch(sketch) = current.feature(SKETCH).unwrap().kind() else {
        panic!("expected sketch");
    };
    assert!(matches!(
        &sketch.constraints[0].kind,
        SketchConstraintKind::Radius { value, .. }
            if value.source_token() == "3" && value.millimetres() == 3.0
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), recomputed);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(reopened_snapshot.canonical_digest(), recomputed);
    assert!(reopened_snapshot.has_feature_parameter(&target));
    let FeatureKind::Sketch(sketch) = reopened_snapshot.feature(SKETCH).unwrap().kind() else {
        panic!("expected reopened sketch");
    };
    assert!(matches!(
        &sketch.constraints[0].kind,
        SketchConstraintKind::Radius { value, .. }
            if value.source_token() == "3" && value.millimetres() == 3.0
    ));
}

#[test]
fn feature_parameter_bindings_are_canonical_persisted_and_never_recompute_on_open() {
    const SOURCE: NodeId = NodeId(201);
    const RULE: NodeId = NodeId(202);
    let segment = SlotSegment::new(RULE, "dimensions", "extrusion_height").unwrap();
    let derived_from =
        DerivedIdentity::new(RULE, SlotPath::new(vec![segment.clone()]).unwrap()).unwrap();
    let target =
        FeatureParameterTarget::new(PROFILE, "segments.1.start.x", ParameterValueType::Length)
            .unwrap();
    let binding = FeatureParameterBinding {
        target: target.clone(),
        derived_from: derived_from.clone(),
    };
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: SOURCE,
                name: "Rule source".to_owned(),
                dimension: height("21"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE,
                name: "Extrusion height rule".to_owned(),
                expression: "$201 * 2".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(segment, vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(binding.clone()),
        ]))
        .unwrap();

    let bound = document.current();
    let bound_digest = bound.canonical_digest();
    assert_eq!(bound.feature_parameter_binding(&target), Some(&binding));
    assert_eq!(bound.feature_parameter_bindings().count(), 1);
    let state = ketchup_model::state_view::encode_semantic_state(&bound);
    let binding_path = "feature_parameter_bindings.10:\"segments.1.start.x\":Length";
    for line in [
        "target.value_type=\"Length\"",
        "derived_from.root_rule_node_id=202",
        "derived_from.slot_path.0.output_port=\"dimensions\"",
        "derived_from.slot_path.0.semantic_key=\"extrusion_height\"",
    ] {
        assert!(
            state.complete().contains(&format!("{binding_path}.{line}")),
            "missing {line}"
        );
    }
    assert!(state.agent().contains(&format!(
        "{binding_path}={{target:{{feature_id:10,path:\"segments.1.start.x\",value_type:\"Length\"}},derived_from:{{root_rule_node_id:202,slot_path:[{{producer_rule_id:202,output_port:\"dimensions\",semantic_key:\"extrusion_height\"}}]}}}}"
    )));
    assert!(matches!(
        bound.feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm[1][0] == 600.0
    ));

    let invalid_target =
        FeatureParameterTarget::new(PROFILE, "constraints.1.value", ParameterValueType::Length)
            .unwrap();
    let undo_before_invalid = document.visible_undo_steps();
    let invalid_slot_error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
            target: invalid_target.clone(),
            derived_from: derived_from.clone(),
        }),
    ])) {
        Ok(_) => panic!("invalid feature parameter slot accepted"),
        Err(error) => error,
    };
    assert_eq!(
        invalid_slot_error,
        CanonicalError::InvalidFeatureParameterBinding(invalid_target)
    );
    assert_eq!(document.current().canonical_digest(), bound_digest);
    assert_eq!(document.visible_undo_steps(), undo_before_invalid);

    let invalid_type =
        FeatureParameterTarget::new(PROFILE, "segments.1.start.x", ParameterValueType::Angle)
            .unwrap();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: invalid_type.clone(),
                derived_from: derived_from.clone(),
            }),
        ])),
        Err(CanonicalError::InvalidFeatureParameterBinding(target)) if target == invalid_type
    ));
    assert_eq!(document.current().canonical_digest(), bound_digest);
    assert_eq!(document.visible_undo_steps(), undo_before_invalid);

    let unresolved = DerivedIdentity::new(
        RULE,
        SlotPath::new(vec![
            SlotSegment::new(RULE, "dimensions", "missing").unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let unresolved_error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
            target: target.clone(),
            derived_from: unresolved,
        }),
    ])) {
        Ok(_) => panic!("unresolved feature parameter binding accepted"),
        Err(error) => error,
    };
    assert_eq!(
        unresolved_error,
        CanonicalError::InvalidFeatureParameterBinding(target.clone())
    );
    assert_eq!(document.current().canonical_digest(), bound_digest);

    let saved_revision = document.current().revision_id();
    let saved_undo = document.visible_undo_steps();
    let loaded = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(loaded.snapshot().revision_id(), saved_revision);
    assert_eq!(loaded.snapshot().canonical_digest(), bound_digest);
    assert_eq!(
        loaded.snapshot().feature_parameter_binding(&target),
        Some(&binding)
    );
    assert!(matches!(
        loaded.snapshot().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm[1][0] == 600.0
    ));
    assert_eq!(document.visible_undo_steps(), saved_undo);

    document.undo().unwrap();
    assert!(
        document
            .current()
            .feature_parameter_binding(&target)
            .is_none()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().feature_parameter_binding(&target),
        Some(&binding)
    );
}

#[test]
fn explicit_feature_parameter_recompute_is_deterministic_undoable_and_identity_bound() {
    const SOURCE: NodeId = NodeId(201);
    const RULE: NodeId = NodeId(202);
    let segment = SlotSegment::new(RULE, "dimensions", "extrusion_height").unwrap();
    let target =
        FeatureParameterTarget::new(EXTRUSION, "extent.distance", ParameterValueType::Length)
            .unwrap();
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: SOURCE,
                name: "Rule source".to_owned(),
                dimension: height("21"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE,
                name: "Extrusion height rule".to_owned(),
                expression: "$201 * 2".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: target.clone(),
                derived_from: DerivedIdentity::new(RULE, SlotPath::new(vec![segment]).unwrap())
                    .unwrap(),
            }),
        ]))
        .unwrap();

    let identity = EvaluationIdentity::default();
    let recompute = CommandBatch::new(vec![CanonicalCommand::RecomputeFeatureParameters {
        identity: identity.clone(),
        scope: FeatureParameterRecomputeScope::All,
    }]);
    let alternate = CommandBatch::new(vec![CanonicalCommand::RecomputeFeatureParameters {
        identity: EvaluationIdentity {
            backend: Some("alternate-backend".to_owned()),
            ..identity.clone()
        },
        scope: FeatureParameterRecomputeScope::All,
    }]);
    assert_ne!(recompute.digest(), alternate.digest());

    let before = document.current().canonical_digest();
    let undo_before = document.visible_undo_steps();
    let revision = document.apply_batch(&recompute).unwrap();
    let recomputed = revision.snapshot().canonical_digest();
    assert_ne!(recomputed, before);
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    assert!(revision.recomputed_nodes().contains(&RULE));
    assert_eq!(revision.evaluation().unwrap().identity, identity);
    assert!(matches!(
        revision.snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "42" && height.millimetres() == 42.0
    ));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));
    assert_eq!(document.redo().unwrap().canonical_digest(), recomputed);
    let provenance = document
        .current()
        .feature_parameter_provenance(&target)
        .unwrap()
        .clone();
    assert_eq!(provenance.identity, identity);
    assert_eq!(provenance.applied_value_bits, 42.0_f64.to_bits());
    assert_eq!(
        document
            .current()
            .audit_feature_parameter_freshness(&identity)
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Current
    );

    let alternate_revision = document.apply_batch(&alternate).unwrap();
    assert_ne!(alternate_revision.snapshot().canonical_digest(), recomputed);
    assert_eq!(
        alternate_revision
            .snapshot()
            .feature_parameter_provenance(&target)
            .unwrap()
            .applied_value_bits,
        provenance.applied_value_bits
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        recomputed,
        "provenance identity must affect the canonical digest even when the applied value is equal"
    );

    let saved_digest = document.current().canonical_digest();
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), saved_digest);
    assert_eq!(
        reopened.audit().feature_parameter_freshness[0].freshness,
        FeatureParameterFreshness::Current
    );
    assert_eq!(
        reopened.snapshot().feature_parameter_provenance(&target),
        Some(&provenance)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: SOURCE,
                dimension: height("22"),
            },
        ]))
        .unwrap();
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "42" && height.millimetres() == 42.0
    ));
    assert_eq!(
        document
            .current()
            .audit_feature_parameter_freshness(&identity)
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Stale(FeatureParameterStaleReason::InputChanged)
    );
    let stale_digest = document.current().canonical_digest();
    let reopened_stale = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened_stale.snapshot().canonical_digest(), stale_digest);
    assert_eq!(
        reopened_stale.audit().feature_parameter_freshness[0].freshness,
        FeatureParameterFreshness::Stale(FeatureParameterStaleReason::InputChanged)
    );

    document.apply_batch(&recompute).unwrap();
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "44" && height.millimetres() == 44.0
    ));
    assert_eq!(
        document
            .current()
            .audit_feature_parameter_freshness(&identity)
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Current
    );
    assert_eq!(
        document
            .current()
            .audit_feature_parameter_freshness(&EvaluationIdentity {
                backend: Some("alternate-backend".to_owned()),
                ..identity
            })
            .unwrap()[0]
            .freshness,
        FeatureParameterFreshness::Stale(FeatureParameterStaleReason::BackendChanged)
    );
}

#[test]
fn feature_parameter_recompute_rolls_back_every_target_when_one_value_is_invalid() {
    const SECOND_EXTRUSION: FeatureId = FeatureId(12);
    const GOOD_SOURCE: NodeId = NodeId(201);
    const GOOD_RULE: NodeId = NodeId(202);
    const INVALID_SOURCE: NodeId = NodeId(203);
    const INVALID_RULE: NodeId = NodeId(204);
    let good_segment = SlotSegment::new(GOOD_RULE, "dimensions", "height").unwrap();
    let invalid_segment = SlotSegment::new(INVALID_RULE, "dimensions", "thickness").unwrap();
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: SECOND_EXTRUSION,
                definition_id: CABINET,
                name: "Second extrusion".to_owned(),
                kind: FeatureKind::extrusion(PROFILE, height("3")),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: GOOD_SOURCE,
                name: "Good source".to_owned(),
                dimension: height("21"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: GOOD_RULE,
                name: "Good rule".to_owned(),
                expression: "$201 * 2".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(good_segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: INVALID_SOURCE,
                name: "Invalid source".to_owned(),
                dimension: height("2"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: INVALID_RULE,
                name: "Invalid rule".to_owned(),
                expression: "$203 * 1000000".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(invalid_segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: FeatureParameterTarget::new(
                    EXTRUSION,
                    "extent.distance",
                    ParameterValueType::Length,
                )
                .unwrap(),
                derived_from: DerivedIdentity::new(
                    GOOD_RULE,
                    SlotPath::new(vec![good_segment]).unwrap(),
                )
                .unwrap(),
            }),
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: FeatureParameterTarget::new(
                    SECOND_EXTRUSION,
                    "extent.distance",
                    ParameterValueType::Length,
                )
                .unwrap(),
                derived_from: DerivedIdentity::new(
                    INVALID_RULE,
                    SlotPath::new(vec![invalid_segment]).unwrap(),
                )
                .unwrap(),
            }),
        ]))
        .unwrap();

    let before = document.current().canonical_digest();
    let undo_before = document.visible_undo_steps();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
                scope: FeatureParameterRecomputeScope::All,
            },
        ])),
        Err(CanonicalError::DimensionOutsideEnvelope)
    ));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_before);
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));
    assert!(matches!(
        document
            .current()
            .feature(SECOND_EXTRUSION)
            .unwrap()
            .kind(),
        FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(_), extent: FeatureExtent::Blind(height), operation: PadOperation::NewBody, .. })
            if height.source_token() == "3" && height.millimetres() == 3.0
    ));
}

#[test]
fn rectangle_numeric_constraints_are_persisted_dependent_only_and_atomic() {
    const WIDTH_SOURCE: NodeId = NodeId(301);
    const WIDTH_RULE: NodeId = NodeId(302);
    const HEIGHT_SOURCE: NodeId = NodeId(303);
    const HEIGHT_RULE: NodeId = NodeId(304);
    const UNRELATED_SOURCE: NodeId = NodeId(305);
    const UNRELATED_RULE: NodeId = NodeId(306);

    let width_segment = SlotSegment::new(WIDTH_RULE, "dimensions", "profile_width").unwrap();
    let height_segment = SlotSegment::new(HEIGHT_RULE, "dimensions", "profile_height").unwrap();
    let width_target =
        FeatureParameterTarget::new(PROFILE, "bounds.width", ParameterValueType::Length).unwrap();
    let height_target =
        FeatureParameterTarget::new(PROFILE, "bounds.height", ParameterValueType::Length).unwrap();
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: WIDTH_SOURCE,
                name: "Rectangle width".to_owned(),
                dimension: height("600"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: WIDTH_RULE,
                name: "Rectangle width constraint".to_owned(),
                expression: "$301".to_owned(),
                input_ports: vec![PortSpec::number("width").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(width_segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: HEIGHT_SOURCE,
                name: "Rectangle height".to_owned(),
                dimension: height("580"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: HEIGHT_RULE,
                name: "Rectangle height constraint".to_owned(),
                expression: "$303".to_owned(),
                input_ports: vec![PortSpec::number("height").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(height_segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: UNRELATED_SOURCE,
                name: "Unrelated source".to_owned(),
                dimension: height("10"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: UNRELATED_RULE,
                name: "Unrelated expression".to_owned(),
                expression: "$305 * 2".to_owned(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: width_target.clone(),
                derived_from: DerivedIdentity::new(
                    WIDTH_RULE,
                    SlotPath::new(vec![width_segment]).unwrap(),
                )
                .unwrap(),
            }),
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: height_target.clone(),
                derived_from: DerivedIdentity::new(
                    HEIGHT_RULE,
                    SlotPath::new(vec![height_segment]).unwrap(),
                )
                .unwrap(),
            }),
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
                scope: FeatureParameterRecomputeScope::All,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    let initial_digest = document.current().canonical_digest();
    let initial_height_provenance = document
        .current()
        .feature_parameter_provenance(&height_target)
        .unwrap()
        .clone();

    let revision = document
        .apply_batch(&CommandBatch::edit_evaluator_and_recompute_affected(
            EvaluatorParameterEdit::SetDimension {
                id: WIDTH_SOURCE,
                dimension: height("650"),
            },
            EvaluationIdentity::default(),
        ))
        .unwrap();
    let resized_digest = revision.snapshot().canonical_digest();
    assert_eq!(
        revision.recomputed_nodes(),
        &[WIDTH_SOURCE, WIDTH_RULE].into_iter().collect()
    );
    assert!(!revision.recomputed_nodes().contains(&HEIGHT_SOURCE));
    assert!(!revision.recomputed_nodes().contains(&HEIGHT_RULE));
    assert!(!revision.recomputed_nodes().contains(&UNRELATED_SOURCE));
    assert!(!revision.recomputed_nodes().contains(&UNRELATED_RULE));
    assert!(matches!(
        revision.snapshot().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm)
            if points_mm == &vec![[0.0, 0.0], [650.0, 0.0], [650.0, 580.0], [0.0, 580.0]]
    ));
    assert_eq!(
        revision
            .snapshot()
            .feature_parameter_provenance(&height_target),
        Some(&initial_height_provenance)
    );
    assert_eq!(
        revision
            .snapshot()
            .audit_feature_parameter_freshness(&EvaluationIdentity::default())
            .unwrap()
            .into_iter()
            .map(|audit| audit.freshness)
            .collect::<Vec<_>>(),
        vec![FeatureParameterFreshness::Current; 2]
    );

    let reopened = persistence::load(&persistence::save(revision.snapshot())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), resized_digest);
    assert_eq!(reopened.snapshot().feature_parameter_bindings().count(), 2);
    assert!(matches!(
        reopened.snapshot().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm[1][0] == 650.0 && points_mm[2][1] == 580.0
    ));

    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), resized_digest);

    let undo_before_invalid = document.visible_undo_steps();
    let invalid_dimension_error =
        match document.apply_batch(&CommandBatch::edit_evaluator_and_recompute_affected(
            EvaluatorParameterEdit::SetDimension {
                id: WIDTH_SOURCE,
                dimension: height("-1"),
            },
            EvaluationIdentity::default(),
        )) {
            Ok(_) => panic!("negative width constraint must fail"),
            Err(error) => error,
        };
    assert_eq!(
        invalid_dimension_error,
        CanonicalError::DimensionOutsideEnvelope
    );
    assert_eq!(document.current().canonical_digest(), resized_digest);
    assert_eq!(document.visible_undo_steps(), undo_before_invalid);

    let irregular_error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetProfilePoints {
            id: PROFILE,
            points_mm: vec![[0.0, 0.0], [650.0, 0.0], [600.0, 580.0], [0.0, 580.0]],
        },
    ])) {
        Ok(_) => panic!("constrained rectangle must reject non-rectangular points"),
        Err(error) => error,
    };
    assert_eq!(
        irregular_error,
        CanonicalError::InvalidFeatureParameterBinding(height_target)
    );
    assert_eq!(document.current().canonical_digest(), resized_digest);
}

#[test]
fn persistent_associative_dimensions_preserve_targets_units_and_unresolved_state() {
    const SOURCE: NodeId = NodeId(401);
    const RULE: NodeId = NodeId(402);
    const WIDTH_DIMENSION: PersistentDimensionId = PersistentDimensionId(1);
    const SLOT_DIMENSION: PersistentDimensionId = PersistentDimensionId(2);
    const EXACT_DIMENSION: PersistentDimensionId = PersistentDimensionId(3);

    let segment = SlotSegment::new(RULE, "dimensions", "profile_height").unwrap();
    let derived =
        DerivedIdentity::new(RULE, SlotPath::new(vec![segment.clone()]).unwrap()).unwrap();
    let mut document = seed_product_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: SOURCE,
                name: "Dimension source".to_owned(),
                dimension: height("580"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE,
                name: "Dimension rule".to_owned(),
                expression: "$401".to_owned(),
                input_ports: vec![PortSpec::number("height").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(segment, vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    WIDTH_DIMENSION,
                    "Profile width",
                    PersistentDimensionTarget::FeatureParameter(
                        FeatureParameterTarget::new(
                            PROFILE,
                            "segments.1.start.x",
                            ParameterValueType::Length,
                        )
                        .unwrap(),
                    ),
                    DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 1).unwrap(),
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    SLOT_DIMENSION,
                    "Rule height",
                    PersistentDimensionTarget::DerivedOutput(derived.clone()),
                    DimensionPresentation::new(DimensionDisplayUnit::Inches, 3).unwrap(),
                )
                .unwrap(),
            ),
            CanonicalCommand::UpsertPersistentDimension(
                PersistentDimension::new(
                    EXACT_DIMENSION,
                    "Exact height",
                    PersistentDimensionTarget::ExactFeatureParameter {
                        definition_id: CABINET,
                        producer_feature_id: EXTRUSION,
                        semantic_role: "top".to_owned(),
                        source_element_id: "face:top".to_owned(),
                        path: ParameterPath::new("extent.distance").unwrap(),
                        value_type: ParameterValueType::Length,
                    },
                    DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 2).unwrap(),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();

    let canonical = document.current();
    let state = encode_semantic_state(&canonical);
    for line in [
        "persistent_dimensions.1.target.FeatureParameter.feature_id=10",
        "persistent_dimensions.1.target.FeatureParameter.path=\"segments.1.start.x\"",
        "persistent_dimensions.1.target.FeatureParameter.value_type=\"Length\"",
        "persistent_dimensions.3.target.ExactFeatureParameter.value_type=\"Length\"",
    ] {
        assert!(state.complete().contains(line), "missing {line}");
    }
    assert!(state.agent().contains(
        "persistent_dimensions.1={id:1,name:\"Profile width\",target:{FeatureParameter:{feature_id:10,path:\"segments.1.start.x\",value_type:\"Length\"}},"
    ));
    let width = canonical
        .project_persistent_dimension(WIDTH_DIMENSION)
        .unwrap();
    assert_eq!(width.health, DimensionReferenceHealth::Resolved);
    assert_eq!(width.millimetres, Some(600.0));
    assert_eq!(width.display_value, Some(60.0));
    assert_eq!(width.display_text.as_deref(), Some("60.0 cm"));

    let slot = canonical
        .project_persistent_dimension(SLOT_DIMENSION)
        .unwrap();
    assert_eq!(slot.health, DimensionReferenceHealth::Resolved);
    assert_eq!(slot.millimetres, Some(580.0));
    assert_eq!(slot.display_text.as_deref(), Some("22.835 in"));

    let exact = canonical
        .project_persistent_dimension(EXACT_DIMENSION)
        .unwrap();
    assert_eq!(exact.health, DimensionReferenceHealth::Lost);
    assert_eq!(exact.millimetres, None);
    assert_eq!(exact.display_text, None);

    let before_invalid = canonical.canonical_digest();
    let invalid = PersistentDimension {
        id: PersistentDimensionId(4),
        name: "Invalid exact target".to_owned(),
        target: PersistentDimensionTarget::ExactFeatureParameter {
            definition_id: DefinitionId(0),
            producer_feature_id: EXTRUSION,
            semantic_role: String::new(),
            source_element_id: String::new(),
            path: ParameterPath::new("extent.distance").unwrap(),
            value_type: ParameterValueType::Length,
        },
        presentation: DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 2).unwrap(),
    };
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(invalid),
        ])),
        Err(CanonicalError::InvalidPersistentDimensionTarget)
    ));
    assert_eq!(document.current().canonical_digest(), before_invalid);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), before_invalid);
    assert_eq!(reopened.snapshot().persistent_dimensions().count(), 3);
    assert_eq!(
        reopened
            .snapshot()
            .persistent_dimension(WIDTH_DIMENSION)
            .unwrap()
            .target,
        PersistentDimensionTarget::FeatureParameter(
            FeatureParameterTarget::new(PROFILE, "segments.1.start.x", ParameterValueType::Length)
                .unwrap()
        )
    );
    assert_eq!(
        reopened
            .snapshot()
            .project_persistent_dimension(EXACT_DIMENSION)
            .unwrap()
            .health,
        DimensionReferenceHealth::Lost
    );

    document.discard_history_before_current();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: EXTRUSION },
            CanonicalCommand::DeleteFeature { id: PROFILE },
            CanonicalCommand::SetRuleOutputs {
                id: RULE,
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(RULE, "dimensions", "replacement").unwrap(),
                        vec![],
                    )
                    .unwrap(),
                ],
            },
        ]))
        .unwrap();
    let unresolved_digest = document.current().canonical_digest();
    assert_eq!(document.current().persistent_dimensions().count(), 3);
    assert_eq!(
        document
            .current()
            .project_persistent_dimension(WIDTH_DIMENSION)
            .unwrap()
            .health,
        DimensionReferenceHealth::Lost
    );
    assert_eq!(
        document
            .current()
            .project_persistent_dimension(SLOT_DIMENSION)
            .unwrap()
            .health,
        DimensionReferenceHealth::Lost
    );
    assert_eq!(
        document
            .current()
            .persistent_dimension(SLOT_DIMENSION)
            .unwrap()
            .target,
        PersistentDimensionTarget::DerivedOutput(derived)
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before_invalid);
    assert_eq!(
        document
            .current()
            .project_persistent_dimension(WIDTH_DIMENSION)
            .unwrap()
            .health,
        DimensionReferenceHealth::Resolved
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        unresolved_digest
    );
    let reopened_unresolved = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(
        reopened_unresolved
            .snapshot()
            .project_persistent_dimension(SLOT_DIMENSION)
            .unwrap()
            .health,
        DimensionReferenceHealth::Lost
    );
}

#[test]
fn saved_views_persist_follow_tag_deletion_and_keep_the_program() {
    const ROOF: TagId = TagId(80);
    const WALLS: TagId = TagId(81);
    const VIEW: SavedViewId = SavedViewId(90);
    let mut document = seed_product_document();
    let before = document.current().canonical_digest();
    let saved = persistence::save(&document.current());
    assert_eq!(
        persistence::load(&saved)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        before,
        "a document without views keeps its digest"
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: ROOF,
                name: "roof".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: WALLS,
                name: "walls".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: FIRST,
                tags: [ROOF].into(),
            },
        ]))
        .unwrap();
    document
        .bind_rule_program(RuleProgramSource {
            variants: Default::default(),
            file_name: "house.star".to_owned(),
            source: "box('roof', (1, 1, 1))".to_owned(),
            overrides: Default::default(),
        })
        .unwrap();
    let view = SavedView {
        id: VIEW,
        name: "Without roof".to_owned(),
        camera: SavedCamera {
            parallel: false,
            yaw_rad: 0.6,
            pitch_rad: 0.4,
            target_z_mm: 1200.0,
            zoom: 1.5,
            pan: [10.0, -5.0],
        },
        style: ["xray".to_owned()].into(),
        hidden_tags: [ROOF, WALLS].into(),
        section: SectionPlane::new([0.0, 0.0, 1500.0], [0.0, 0.0, 2.0]),
    };
    assert_eq!(view.section.unwrap().normal, [0.0, 0.0, 1.0]);
    assert_eq!(SectionPlane::new([0.0; 3], [0.0; 3]), None);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSavedView(
            view.clone(),
        )]))
        .unwrap();
    assert_eq!(document.current().saved_view(VIEW), Some(&view));
    assert!(
        document.current_rule_program().is_some(),
        "keeping a view does not detach the program"
    );
    assert_eq!(
        document.current().occurrence_effectively_visible(FIRST),
        Some(true),
        "a saved view changes nothing until it is activated"
    );

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().saved_view(VIEW), Some(&view));
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        document.current().canonical_digest()
    );

    // Another view may not take the same name, hide a tag that does not exist or cut
    // along a plane without a direction.
    for (invalid, problem) in [
        (
            SavedView {
                id: SavedViewId(91),
                name: "Flat cut".to_owned(),
                section: Some(SectionPlane {
                    point_mm: [0.0; 3],
                    normal: [0.0; 3],
                }),
                ..view.clone()
            },
            SavedViewProblem::InvalidSection,
        ),
        (
            SavedView {
                id: SavedViewId(91),
                ..view.clone()
            },
            SavedViewProblem::NameTaken(VIEW),
        ),
        (
            SavedView {
                id: SavedViewId(91),
                name: "Ghost".to_owned(),
                hidden_tags: [TagId(999)].into(),
                ..view.clone()
            },
            SavedViewProblem::MissingTag(TagId(999)),
        ),
    ] {
        let error = document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSavedView(
                invalid,
            )]))
            .err()
            .unwrap();
        assert_eq!(
            error,
            CanonicalError::InvalidSavedView {
                id: SavedViewId(91),
                problem: problem.clone(),
            }
        );
        // The message names what is wrong, not only which view.
        assert!(error.to_string().contains(&problem.to_string()), "{error}");
    }

    // Deleting a tag removes it from the views that hide it.
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::DeleteTag {
            id: WALLS,
        }]))
        .unwrap();
    assert!(document.current_rule_program().is_none());
    assert!(
        document
            .current()
            .saved_view(VIEW)
            .unwrap()
            .hidden_tags
            .iter()
            .eq([&ROOF])
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteSavedView { id: VIEW },
        ]))
        .unwrap();
    assert_eq!(document.current().saved_views().count(), 0);
    document.undo().unwrap();
    assert!(document.current().saved_view(VIEW).is_some());
}

#[test]
fn canonical_tags_drive_visibility_persist_and_roll_back_atomically() {
    const HIDDEN: TagId = TagId(70);
    const SHOWN: TagId = TagId(71);
    let mut document = seed_product_document();
    let before = document.current().canonical_digest();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: HIDDEN,
                name: "Hidden hardware".to_owned(),
                visible: false,
            },
            CanonicalCommand::CreateTag {
                id: SHOWN,
                name: "Concept".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: FIRST,
                tags: [HIDDEN, SHOWN].into(),
            },
            CanonicalCommand::SetOccurrenceTags {
                id: SECOND,
                tags: [SHOWN].into(),
            },
        ]))
        .unwrap();
    let tagged = document.current().canonical_digest();
    assert_ne!(tagged, before);
    assert_eq!(document.current().tags().count(), 2);
    assert_eq!(
        document
            .current()
            .occurrences_with_tag(SHOWN)
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND]
    );
    assert_eq!(
        document
            .current()
            .occurrences_with_tag(HIDDEN)
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>(),
        vec![FIRST]
    );
    assert_eq!(
        document.current().occurrence_effectively_visible(FIRST),
        Some(false)
    );
    let scene = document.current().scene_query();
    assert!(
        !scene
            .iter()
            .find(|item| item.occurrence_id == FIRST)
            .unwrap()
            .visible
    );
    assert!(
        scene
            .iter()
            .find(|item| item.occurrence_id == SECOND)
            .unwrap()
            .visible
    );

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), tagged);
    assert_eq!(
        reopened.snapshot().tag(HIDDEN).unwrap().name(),
        "Hidden hardware"
    );
    assert_eq!(
        reopened.snapshot().occurrence_effectively_visible(FIRST),
        Some(false)
    );

    assert!(
        reopened
            .snapshot()
            .occurrence(FIRST)
            .unwrap()
            .tags()
            .iter()
            .eq([&HIDDEN, &SHOWN])
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), tagged);

    // Showing one tag of a part is not enough: hiding any of its tags hides it.
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: HIDDEN,
                visible: true,
            },
            CanonicalCommand::SetTagVisibility {
                id: SHOWN,
                visible: false,
            },
        ]))
        .unwrap();
    let swapped = document.current();
    assert_eq!(swapped.occurrence_effectively_visible(FIRST), Some(false));
    assert_eq!(swapped.occurrence_effectively_visible(SECOND), Some(false));
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: SHOWN,
                visible: true,
            },
        ]))
        .unwrap();
    assert_eq!(
        document.current().occurrence_effectively_visible(FIRST),
        Some(true)
    );
    document.undo().unwrap();
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), tagged);

    let steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: HIDDEN,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: SECOND,
                tags: [SHOWN, TagId(999)].into(),
            },
        ]))
        .err()
        .unwrap();
    assert_eq!(error, CanonicalError::TagNotFound(TagId(999)));
    assert_eq!(document.current().canonical_digest(), tagged);
    assert_eq!(document.visible_undo_steps(), steps);
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::DeleteTag {
                id: HIDDEN,
            }]))
            .err()
            .unwrap(),
        CanonicalError::TagInUse {
            tag: HIDDEN,
            holder: TagHolder::Occurrence(FIRST),
            holders: 1,
        }
    );
    assert_eq!(document.current().canonical_digest(), tagged);
}

#[test]
fn canonical_collections_persist_query_and_roll_back_atomically() {
    const SELECTED: CollectionId = CollectionId(80);
    let mut document = seed_product_document();
    let before = document.current().canonical_digest();
    let first_parent = document.current().occurrence(FIRST).unwrap().parent();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: SELECTED,
                name: "Selected cabinets".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: SELECTED,
                occurrence_ids: vec![FIRST, SECOND],
            },
        ]))
        .unwrap();
    let collected = document.current().canonical_digest();
    assert_ne!(collected, before);
    assert_eq!(document.current().collections().count(), 1);
    assert_eq!(
        document.current().collection(SELECTED).unwrap().name(),
        "Selected cabinets"
    );
    assert_eq!(
        document
            .current()
            .occurrences_in_collection(SELECTED)
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND]
    );
    assert_eq!(
        document.current().occurrence(FIRST).unwrap().parent(),
        first_parent
    );

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), collected);
    assert_eq!(
        reopened
            .snapshot()
            .collection(SELECTED)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![FIRST, SECOND]
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), collected);
    let steps = document.visible_undo_steps();
    for (occurrence_ids, expected) in [
        (
            vec![SECOND, FIRST],
            CanonicalError::CollectionMembershipNotCanonical(SELECTED),
        ),
        (
            vec![FIRST, OccurrenceId(999)],
            CanonicalError::OccurrenceNotFound(OccurrenceId(999)),
        ),
    ] {
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteCollection { id: SELECTED },
                CanonicalCommand::CreateCollection {
                    id: SELECTED,
                    name: "Invalid replacement".to_owned(),
                },
                CanonicalCommand::SetCollectionOccurrences {
                    id: SELECTED,
                    occurrence_ids,
                },
            ]))
            .err()
            .unwrap();
        assert_eq!(error, expected);
        assert_eq!(document.current().canonical_digest(), collected);
        assert_eq!(document.visible_undo_steps(), steps);
    }
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteOccurrence { id: FIRST }
            ]))
            .err()
            .unwrap(),
        CanonicalError::OccurrenceInCollection(FIRST)
    );
    assert_eq!(document.current().canonical_digest(), collected);
}

#[test]
fn separate_occurrence_extrusions_use_graph_first_rigid_transform_and_distinct_heights() {
    let mut document = seed_separate_solid_tool_document(0.0, 0.0, 30.0);
    let angle = 30.0_f64.to_radians();
    let tool_transform = Transform::from_matrix([
        angle.cos(),
        -angle.sin(),
        0.0,
        25.0,
        angle.sin(),
        angle.cos(),
        0.0,
        10.0,
        0.0,
        0.0,
        1.0,
        5.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(203),
                dimension: height("70"),
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(302),
                transform: tool_transform,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
            separate_solid_tool_plan(BooleanOperation::Cut, true),
        )]))
        .unwrap();

    let snapshot = document.current();
    assert_eq!(
        snapshot
            .definition(DefinitionId(401))
            .unwrap()
            .feature_ids()
            .len(),
        7
    );
    assert!(matches!(
        snapshot.feature(FeatureId(407)).unwrap().kind(),
        FeatureKind::RigidTransform {
            target: FeatureId(406),
            transform,
        } if *transform == tool_transform
    ));
    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(401), FeatureId(408)).unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::RigidTransform { .. }))
            .count(),
        2
    );
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn separate_occurrence_subtract_is_one_canonical_undo_step_and_persists_stable_ids() {
    let mut document = seed_separate_solid_tool_document(20.0, 20.0, 30.0);
    let before = document.current().canonical_digest();
    let consume = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Cut, false),
    )]);
    let keep = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Cut, true),
    )]);
    assert_ne!(consume.digest(), keep.digest());

    document.apply_batch(&consume).unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(401)
    );
    assert!(document.current().occurrence(OccurrenceId(302)).is_none());
    assert_eq!(
        document
            .current()
            .definition(DefinitionId(401))
            .unwrap()
            .feature_ids(),
        &[
            FeatureId(402),
            FeatureId(403),
            FeatureId(404),
            FeatureId(405),
            FeatureId(406),
            FeatureId(407),
            FeatureId(408),
        ]
    );
    assert!(matches!(
        document.current().feature(FeatureId(407)).unwrap().kind(),
        FeatureKind::RigidTransform {
            target: FeatureId(406),
            transform,
        } if transform.matrix()[3] == 20.0 && transform.matrix()[7] == 20.0
    ));
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Cut,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Cut,
            ..
        }
    )));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(101)
    );
    assert!(document.current().occurrence(OccurrenceId(302)).is_some());
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), applied);
    assert_eq!(
        reopened
            .snapshot()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(401)
    );
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_none());
}
