use ketchup_core::document::{
    BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand, CanonicalError, CollectionId,
    CommandBatch, ConvertedEntityId, DefinitionId, DerivedIdentity, Dimension,
    DimensionDisplayUnit, DimensionPresentation, DimensionReferenceHealth, DocumentStore,
    EvaluationIdentity, FeatureId, FeatureKind, FeatureParameterBinding, FeatureParameterFreshness,
    FeatureParameterSlot, FeatureParameterStaleReason, FeatureParameterTarget, GroupId,
    InstancePath, InstancePathStep, LocalGroupId, LocalGroupKey, LocalOccurrenceId,
    LocalOccurrenceKey, MappingResolution, NodeId, OccurrenceId, PersistentDimension,
    PersistentDimensionId, PersistentDimensionTarget, PortSpec, RuleOutput, SceneQueryContext,
    SceneQueryError, SlotPath, SlotSegment, TagId, Transform, UnresolvedMappingReason,
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

    assert_eq!(loaded.source_schema(), 17);
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

    let mut schema_fifteen = persistence::save(&expected);
    let manifest_length = u32::from_le_bytes(schema_fifteen[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    schema_fifteen.truncate(schema_fifteen.len() - 8);
    schema_fifteen[10..12].copy_from_slice(&15_u16.to_le_bytes());
    let payload_length = (schema_fifteen.len() - payload_offset) as u64;
    schema_fifteen[16..24].copy_from_slice(&payload_length.to_le_bytes());
    let checksum = ketchup_core::graph::sha256_bytes(&schema_fifteen[payload_offset..]);
    schema_fifteen[24..56].copy_from_slice(&checksum);
    let previous_current = persistence::load(&schema_fifteen).unwrap();
    assert_eq!(previous_current.source_schema(), 15);
    assert_eq!(
        previous_current.snapshot().canonical_digest(),
        expected.canonical_digest()
    );

    let mut schema_nine = persistence::save(&expected);
    let manifest_length = u32::from_le_bytes(schema_nine[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    schema_nine.drain(payload_offset + 25..payload_offset + 29);
    schema_nine.truncate(schema_nine.len() - 20);
    schema_nine[10..12].copy_from_slice(&9_u16.to_le_bytes());
    let payload_length = (schema_nine.len() - payload_offset) as u64;
    schema_nine[16..24].copy_from_slice(&payload_length.to_le_bytes());
    let checksum = ketchup_core::graph::sha256_bytes(&schema_nine[payload_offset..]);
    schema_nine[24..56].copy_from_slice(&checksum);
    let legacy_current = persistence::load(&schema_nine).unwrap();
    assert_eq!(legacy_current.source_schema(), 9);
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
        document.current().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &exact_points
    ));

    let valid_digest = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let invalid_profiles = [
        vec![[0.0, 0.0], [10.0, 0.0]],
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]],
        vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]],
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
    assert_eq!(loaded.source_schema(), 17);
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
    assert_eq!(reopened.source_schema(), 17);
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
    assert_eq!(reopened.source_schema(), 17);
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
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(anchor_id.0 + 2),
                definition_id,
                name: "Hidden sibling".to_owned(),
                transform: Transform::from_translation(1000.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
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
    assert_eq!(reopened.source_schema(), 17);
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

#[test]
fn feature_parameter_bindings_are_canonical_persisted_and_never_recompute_on_open() {
    const SOURCE: NodeId = NodeId(201);
    const RULE: NodeId = NodeId(202);
    let segment = SlotSegment::new(RULE, "dimensions", "extrusion_height").unwrap();
    let derived_from =
        DerivedIdentity::new(RULE, SlotPath::new(vec![segment.clone()]).unwrap()).unwrap();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
    };
    let binding = FeatureParameterBinding {
        target,
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
    assert_eq!(bound.feature_parameter_binding(target), Some(&binding));
    assert_eq!(bound.feature_parameter_bindings().count(), 1);
    let state = ketchup_core::state_view::encode_semantic_state(&bound);
    for view in [state.complete_v1(), state.agent_v1()] {
        assert!(view.contains("parameter_binding.11.height.derived_from.root=202"));
        assert!(view.contains(
            "parameter_binding.11.height.derived_from.slot_path=202:\"dimensions\":\"extrusion_height\""
        ));
    }
    assert!(matches!(
        bound.feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));

    let invalid_target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Thickness,
    };
    let undo_before_invalid = document.visible_undo_steps();
    let invalid_slot_error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
            target: invalid_target,
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
            target,
            derived_from: unresolved,
        }),
    ])) {
        Ok(_) => panic!("unresolved feature parameter binding accepted"),
        Err(error) => error,
    };
    assert_eq!(
        unresolved_error,
        CanonicalError::InvalidFeatureParameterBinding(target)
    );
    assert_eq!(document.current().canonical_digest(), bound_digest);

    let saved_revision = document.current().revision_id();
    let saved_undo = document.visible_undo_steps();
    let loaded = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(loaded.source_schema(), 17);
    assert_eq!(loaded.snapshot().revision_id(), saved_revision);
    assert_eq!(loaded.snapshot().canonical_digest(), bound_digest);
    assert_eq!(
        loaded.snapshot().feature_parameter_binding(target),
        Some(&binding)
    );
    assert!(matches!(
        loaded.snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));
    assert_eq!(document.visible_undo_steps(), saved_undo);

    document.undo().unwrap();
    assert!(
        document
            .current()
            .feature_parameter_binding(target)
            .is_none()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().feature_parameter_binding(target),
        Some(&binding)
    );
}

#[test]
fn explicit_feature_parameter_recompute_is_deterministic_undoable_and_identity_bound() {
    const SOURCE: NodeId = NodeId(201);
    const RULE: NodeId = NodeId(202);
    let segment = SlotSegment::new(RULE, "dimensions", "extrusion_height").unwrap();
    let target = FeatureParameterTarget {
        feature_id: EXTRUSION,
        slot: FeatureParameterSlot::Height,
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
                outputs: vec![RuleOutput::new(segment.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target,
                derived_from: DerivedIdentity::new(RULE, SlotPath::new(vec![segment]).unwrap())
                    .unwrap(),
            }),
        ]))
        .unwrap();

    let identity = EvaluationIdentity::default();
    let recompute = CommandBatch::new(vec![CanonicalCommand::RecomputeFeatureParameters {
        identity: identity.clone(),
    }]);
    let alternate = CommandBatch::new(vec![CanonicalCommand::RecomputeFeatureParameters {
        identity: EvaluationIdentity {
            backend: Some("alternate-backend".to_owned()),
            ..identity.clone()
        },
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
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "42" && height.millimetres() == 42.0
    ));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));
    assert_eq!(document.redo().unwrap().canonical_digest(), recomputed);
    let provenance = document
        .current()
        .feature_parameter_provenance(target)
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

    let saved_digest = document.current().canonical_digest();
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), saved_digest);
    assert_eq!(
        reopened.audit().feature_parameter_freshness[0].freshness,
        FeatureParameterFreshness::Current
    );
    assert_eq!(
        reopened.snapshot().feature_parameter_provenance(target),
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
        FeatureKind::Extrusion { height, .. }
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
        FeatureKind::Extrusion { height, .. }
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
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: height("3"),
                },
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
                target: FeatureParameterTarget {
                    feature_id: EXTRUSION,
                    slot: FeatureParameterSlot::Height,
                },
                derived_from: DerivedIdentity::new(
                    GOOD_RULE,
                    SlotPath::new(vec![good_segment]).unwrap(),
                )
                .unwrap(),
            }),
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: FeatureParameterTarget {
                    feature_id: SECOND_EXTRUSION,
                    slot: FeatureParameterSlot::Height,
                },
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
            },
        ])),
        Err(CanonicalError::DimensionOutsideEnvelope)
    ));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_before);
    assert!(matches!(
        document.current().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "720" && height.millimetres() == 720.0
    ));
    assert!(matches!(
        document
            .current()
            .feature(SECOND_EXTRUSION)
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { height, .. }
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
    let width_target = FeatureParameterTarget {
        feature_id: PROFILE,
        slot: FeatureParameterSlot::ProfileWidth,
    };
    let height_target = FeatureParameterTarget {
        feature_id: PROFILE,
        slot: FeatureParameterSlot::ProfileHeight,
    };
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
                target: width_target,
                derived_from: DerivedIdentity::new(
                    WIDTH_RULE,
                    SlotPath::new(vec![width_segment]).unwrap(),
                )
                .unwrap(),
            }),
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: height_target,
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
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    let initial_digest = document.current().canonical_digest();
    let initial_height_provenance = document
        .current()
        .feature_parameter_provenance(height_target)
        .unwrap()
        .clone();

    let revision = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH_SOURCE,
                dimension: height("650"),
            },
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
            },
        ]))
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
        revision.snapshot().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[0.0, 0.0], [650.0, 0.0], [650.0, 580.0], [0.0, 580.0]]
    ));
    assert_eq!(
        revision
            .snapshot()
            .feature_parameter_provenance(height_target),
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
    assert_eq!(reopened.source_schema(), 17);
    assert_eq!(reopened.snapshot().canonical_digest(), resized_digest);
    assert_eq!(reopened.snapshot().feature_parameter_bindings().count(), 2);
    assert!(matches!(
        reopened.snapshot().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm[1][0] == 650.0 && points_mm[2][1] == 580.0
    ));

    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), resized_digest);

    let undo_before_invalid = document.visible_undo_steps();
    let invalid_dimension_error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetEvaluatorDimension {
            id: WIDTH_SOURCE,
            dimension: height("-1"),
        },
        CanonicalCommand::RecomputeFeatureParameters {
            identity: EvaluationIdentity::default(),
        },
    ])) {
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
        CanonicalError::InvalidFeatureParameterBinding(width_target)
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
                    PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
                        feature_id: PROFILE,
                        slot: FeatureParameterSlot::ProfileWidth,
                    }),
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
                        slot: FeatureParameterSlot::Height,
                    },
                    DimensionPresentation::new(DimensionDisplayUnit::Millimetres, 2).unwrap(),
                )
                .unwrap(),
            ),
        ]))
        .unwrap();

    let canonical = document.current();
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
            slot: FeatureParameterSlot::Height,
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
    assert_eq!(reopened.source_schema(), 17);
    assert_eq!(reopened.snapshot().canonical_digest(), before_invalid);
    assert_eq!(reopened.snapshot().persistent_dimensions().count(), 3);
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
fn canonical_tags_drive_visibility_persist_and_roll_back_atomically() {
    const HIDDEN: TagId = TagId(70);
    let mut document = seed_product_document();
    let before = document.current().canonical_digest();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: HIDDEN,
                name: "Hidden hardware".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: FIRST,
                tag: Some(HIDDEN),
            },
        ]))
        .unwrap();
    let tagged = document.current().canonical_digest();
    assert_ne!(tagged, before);
    assert_eq!(document.current().tags().count(), 1);
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

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), tagged);

    let steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: HIDDEN,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: SECOND,
                tag: Some(TagId(999)),
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
        CanonicalError::TagInUse(HIDDEN)
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
    assert_eq!(reopened.source_schema(), 17);
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
