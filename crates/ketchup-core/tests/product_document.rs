use ketchup_core::document::{
    BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand, CanonicalError, CommandBatch,
    ConvertedEntityId, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, GroupId,
    InstancePathStep, LocalGroupId, LocalGroupKey, LocalOccurrenceId, LocalOccurrenceKey,
    MappingResolution, NodeId, OccurrenceId, TagId, Transform, UnresolvedMappingReason,
    WorldEntityPath,
};
use ketchup_core::persistence;

const CABINET: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const FIRST: OccurrenceId = OccurrenceId(20);
const SECOND: OccurrenceId = OccurrenceId(21);
const GROUP: GroupId = GroupId(30);

fn height(value: &str) -> Dimension {
    Dimension::from_decimal(value).unwrap()
}

fn seed_product_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: CABINET,
                name: "Base Cabinet".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: CABINET,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [600.0, 0.0], [600.0, 580.0], [0.0, 580.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: CABINET,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: height("720"),
                },
            },
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Kitchen run".to_owned(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: CABINET,
                name: "Base Cabinet #1".to_owned(),
                transform: Transform::identity(),
                parent: Some(GROUP),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: CABINET,
                name: "Base Cabinet #2".to_owned(),
                transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document
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
        FeatureKind::Extrusion {
            profile: FeatureId(12),
            ..
        }
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

    assert_eq!(loaded.source_schema(), 9);
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

    let mut schema_five = persistence::save(&expected);
    schema_five[10..12].copy_from_slice(&5_u16.to_le_bytes());
    let legacy_current = persistence::load(&schema_five).unwrap();
    assert_eq!(legacy_current.source_schema(), 5);
    assert_eq!(
        legacy_current.snapshot().canonical_digest(),
        expected.canonical_digest()
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
        document.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &resized
    ));
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert!(matches!(
        document.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm[1][0] == 600.0
    ));
}

#[test]
fn m6_bottle_profile_and_revolve_are_canonical_persisted_and_undoable() {
    const BOTTLE: DefinitionId = DefinitionId(40);
    const BOTTLE_PROFILE: FeatureId = FeatureId(41);
    const BOTTLE_REVOLVE: FeatureId = FeatureId(42);
    const BODY_RADIUS: NodeId = NodeId(100);
    const BODY_HEIGHT: NodeId = NodeId(101);
    let initial_profile = vec![
        [0.0, 0.0],
        [30.0, 0.0],
        [30.0, 110.0],
        [12.0, 130.0],
        [12.0, 155.0],
        [0.0, 155.0],
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: BODY_RADIUS,
                name: "Bottle body radius".to_owned(),
                dimension: height("30"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: BODY_HEIGHT,
                name: "Bottle body height".to_owned(),
                dimension: height("110"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateDefinition {
                id: BOTTLE,
                name: "Rotational bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_PROFILE,
                definition_id: BOTTLE,
                name: "Bottle half-profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: initial_profile.clone(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOTTLE_REVOLVE,
                definition_id: BOTTLE,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::Revolve {
                    profile: BOTTLE_PROFILE,
                },
            },
        ]))
        .unwrap();
    let initial_digest = document.current().canonical_digest();
    document.discard_history_before_current();

    let changed_profile = vec![
        [0.0, 0.0],
        [27.0, 0.0],
        [27.0, 130.0],
        [12.0, 150.0],
        [12.0, 175.0],
        [0.0, 175.0],
    ];
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: BODY_RADIUS,
                dimension: height("27"),
            },
            CanonicalCommand::SetEvaluatorDimension {
                id: BODY_HEIGHT,
                dimension: height("130"),
            },
            CanonicalCommand::SetProfilePoints {
                id: BOTTLE_PROFILE,
                points_mm: changed_profile.clone(),
            },
        ]))
        .unwrap();
    let changed_digest = document.current().canonical_digest();
    assert_ne!(changed_digest, initial_digest);
    assert!(matches!(
        document.current().feature(BOTTLE_REVOLVE).unwrap().kind(),
        FeatureKind::Revolve {
            profile: BOTTLE_PROFILE
        }
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let loaded = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(loaded.source_schema(), 9);
    assert!(loaded.migration_losses().is_empty());
    assert_eq!(loaded.snapshot().canonical_digest(), changed_digest);
    assert!(matches!(
        loaded.snapshot().feature(BOTTLE_PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &changed_profile
    ));
}

#[test]
fn m6_shell_thickness_is_canonical_undoable_persisted_and_fail_closed() {
    const BOTTLE: DefinitionId = DefinitionId(45);
    const PROFILE: FeatureId = FeatureId(46);
    const REVOLVE: FeatureId = FeatureId(47);
    const SHELL: FeatureId = FeatureId(48);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE,
                name: "Shell bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: BOTTLE,
                name: "Bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: BOTTLE,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::Revolve { profile: PROFILE },
            },
            CanonicalCommand::CreateFeature {
                id: SHELL,
                definition_id: BOTTLE,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: REVOLVE,
                    thickness: height("2"),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    let initial_digest = document.current().canonical_digest();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: SHELL,
                dimension: height("2.5"),
            },
        ]))
        .unwrap();
    let changed_digest = document.current().canonical_digest();
    assert_ne!(changed_digest, initial_digest);
    assert!(matches!(
        document.current().feature(SHELL).unwrap().kind(),
        FeatureKind::Shell { target: REVOLVE, thickness }
            if thickness.source_token() == "2.5" && thickness.millimetres() == 2.5
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), 9);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), changed_digest);

    let before_invalid = document.current().canonical_digest();
    let undo_before_invalid = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: SHELL,
                dimension: height("6"),
            },
        ]))
        .err()
        .expect("half-neck-radius shell must fail before exact evaluation");
    assert_eq!(error, CanonicalError::InvalidFeatureOwnership(SHELL));
    assert_eq!(document.current().canonical_digest(), before_invalid);
    assert_eq!(document.visible_undo_steps(), undo_before_invalid);
}

#[test]
fn m6_controlled_profile_and_edge_finish_are_atomic_persisted_and_fail_closed() {
    const BOTTLE: DefinitionId = DefinitionId(60);
    const PROFILE: FeatureId = FeatureId(61);
    const CONTROL: FeatureId = FeatureId(62);
    const REVOLVE: FeatureId = FeatureId(63);
    const SHELL: FeatureId = FeatureId(64);
    const FINISH: FeatureId = FeatureId(65);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: BOTTLE,
                name: "Controlled bottle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: BOTTLE,
                name: "Bottle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [30.0, 0.0],
                        [30.0, 110.0],
                        [12.0, 130.0],
                        [12.0, 155.0],
                        [0.0, 155.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: CONTROL,
                definition_id: BOTTLE,
                name: "Bottle controls".to_owned(),
                kind: FeatureKind::BottleProfileControl {
                    profile: PROFILE,
                    body_radius: height("30"),
                    body_height: height("110"),
                    shoulder_rise: height("20"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: BOTTLE,
                name: "Bottle revolve".to_owned(),
                kind: FeatureKind::Revolve { profile: CONTROL },
            },
            CanonicalCommand::CreateFeature {
                id: SHELL,
                definition_id: BOTTLE,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: REVOLVE,
                    thickness: height("2"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FINISH,
                definition_id: BOTTLE,
                name: "Shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: SHELL,
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: height("2"),
                },
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    let initial_digest = document.current().canonical_digest();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBottleControlDimension {
                id: CONTROL,
                control: BottleControlDimension::BodyRadius,
                dimension: height("32"),
            },
            CanonicalCommand::SetBottleControlDimension {
                id: CONTROL,
                control: BottleControlDimension::BodyHeight,
                dimension: height("120"),
            },
            CanonicalCommand::SetBottleControlDimension {
                id: CONTROL,
                control: BottleControlDimension::ShoulderRise,
                dimension: height("16"),
            },
            CanonicalCommand::SetFeatureDimension {
                id: FINISH,
                dimension: height("1.5"),
            },
            CanonicalCommand::SetBottleEdgeFinishKind {
                id: FINISH,
                kind: BottleEdgeFinishKind::Chamfer,
            },
        ]))
        .unwrap();
    let changed_digest = document.current().canonical_digest();
    assert_ne!(changed_digest, initial_digest);
    assert!(matches!(
        document.current().feature(CONTROL).unwrap().kind(),
        FeatureKind::BottleProfileControl {
            profile: PROFILE,
            body_radius,
            body_height,
            shoulder_rise,
        } if body_radius.millimetres() == 32.0
            && body_height.millimetres() == 120.0
            && shoulder_rise.millimetres() == 16.0
    ));
    assert!(matches!(
        document.current().feature(FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            target: SHELL,
            kind: BottleEdgeFinishKind::Chamfer,
            amount,
        } if amount.source_token() == "1.5"
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), 9);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), changed_digest);
    assert!(matches!(
        reopened.snapshot().feature(FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Chamfer,
            amount,
            ..
        } if amount.millimetres() == 1.5
    ));

    for (command, invalid_feature) in [
        (
            CanonicalCommand::SetBottleControlDimension {
                id: CONTROL,
                control: BottleControlDimension::BodyRadius,
                dimension: height("10"),
            },
            CONTROL,
        ),
        (
            CanonicalCommand::SetFeatureDimension {
                id: FINISH,
                dimension: height("8"),
            },
            FINISH,
        ),
    ] {
        let before_invalid = document.current().canonical_digest();
        let undo_before_invalid = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![command]))
            .err()
            .expect("invalid controlled bottle edit must fail");
        assert_eq!(
            error,
            CanonicalError::InvalidFeatureOwnership(invalid_feature)
        );
        assert_eq!(document.current().canonical_digest(), before_invalid);
        assert_eq!(document.visible_undo_steps(), undo_before_invalid);
    }
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[1.0, 0.0], [20.0, 0.0], [20.0, 100.0], [1.0, 100.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: REVOLVE,
                definition_id: BOTTLE,
                name: "Invalid revolve".to_owned(),
                kind: FeatureKind::Revolve { profile: PROFILE },
            },
        ]))
        .err()
        .expect("off-axis revolve must fail");
    assert_eq!(error, CanonicalError::InvalidFeatureOwnership(REVOLVE));
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
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [20.0, 100.0], [20.0, 0.0], [0.0, 100.0]],
            },
        }]))
        .err()
        .expect("self-intersecting profile must fail");
    assert_eq!(error, CanonicalError::InvalidProfile);
    assert_eq!(document.current().canonical_digest(), before_profile);
    assert!(document.current().feature(PROFILE).is_none());
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
                tag: Some(TagId(7)),
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
                tag: None,
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
    assert_eq!(unique_local.tag(), Some(TagId(7)));
    assert!(!unique_local.visible());
    assert_eq!(unique_local.transform().matrix()[3], 5.0);
    assert_eq!(document.undo().unwrap().canonical_digest(), before_unique);
    assert_eq!(document.redo().unwrap().canonical_digest(), unique_digest);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), 9);
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
    assert_eq!(reopened_local.tag(), Some(TagId(7)));
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
                tag: None,
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
    let mut bytes = persistence::save(&ownership.current());
    let mut marker = Vec::new();
    marker.extend_from_slice(&DefinitionId(2).0.to_le_bytes());
    marker.extend_from_slice(&LocalOccurrenceId(FIRST.0).0.to_le_bytes());
    marker.extend_from_slice(&CABINET.0.to_le_bytes());
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("local occurrence record is present");
    bytes[offset + 16..offset + 24].copy_from_slice(&DefinitionId(2).0.to_le_bytes());
    let manifest_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    let checksum = ketchup_core::graph::sha256_bytes(&bytes[payload_offset..]);
    bytes[24..56].copy_from_slice(&checksum);
    assert!(matches!(
        persistence::load(&bytes),
        Err(persistence::PersistenceError::InvalidCanonicalData(
            CanonicalError::InvalidLocalGraph
        ))
    ));
}
