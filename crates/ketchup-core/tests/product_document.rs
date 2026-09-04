use ketchup_core::bottle_m6::ExactRevolveRequest;
use ketchup_core::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BOTTLE_SHOULDER_EDGE_ROLE, BottleControlDimension,
    BottleEdgeFinishKind, StableEdgeRole, StableFaceRole,
};
use ketchup_core::document::{
    BooleanOperation, CanonicalCommand, CanonicalError, CollectionId, CommandBatch,
    ConvertedEntityId, DefinitionId, DerivedIdentity, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DimensionReferenceHealth, DocumentStore, EvaluationIdentity, FeatureId,
    FeatureKind, FeatureParameterBinding, FeatureParameterFreshness, FeatureParameterStaleReason,
    FeatureParameterTarget, GroupId, InstancePath, InstancePathStep, LocalGroupId, LocalGroupKey,
    LocalOccurrenceId, LocalOccurrenceKey, LoftSection, MappingResolution, NodeId, OccurrenceId,
    ParameterPath, ParameterPathError, ParameterValueType, PersistentDimension,
    PersistentDimensionId, PersistentDimensionTarget, PortSpec, ProfileSegment, RuleOutput,
    SceneQueryContext, SceneQueryError, SlotPath, SlotSegment, Snapshot, SolidToolPlan,
    SpatialPathSegment, TagId, Transform, UnresolvedMappingReason, WorldEntityPath,
};
use ketchup_core::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarLoop,
    ExactBRepPlanarSegment,
};
use ketchup_core::exact_product::{
    EXACT_CIRCLE_EVALUATOR_V1, ExactFeatureChainRequest, ExactPlanarOffsetRequest,
};
use ketchup_core::persistence;
#[cfg(not(feature = "named-product-fixtures"))]
use ketchup_core::persistence::{LegacyFeatureKind, PersistenceError};
use ketchup_core::sketch::{
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchEntityId, SketchPointKind, SketchPointRef, SketchSpec, WorkplaneSpec,
};
use ketchup_core::state_view::encode_semantic_state;

const CABINET: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const FIRST: OccurrenceId = OccurrenceId(20);
const SECOND: OccurrenceId = OccurrenceId(21);
const GROUP: GroupId = GroupId(30);

fn height(value: &str) -> Dimension {
    Dimension::from_decimal(value).unwrap()
}

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
    let profile = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [4.0, 2.0]],
    };
    assert_eq!(
        profile
            .parameter_descriptors()
            .iter()
            .map(|descriptor| (descriptor.path().as_str(), descriptor.value_type()))
            .collect::<Vec<_>>(),
        vec![
            ("points.0.x", ParameterValueType::Length),
            ("points.0.y", ParameterValueType::Length),
            ("points.1.x", ParameterValueType::Length),
            ("points.1.y", ParameterValueType::Length),
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

fn body_contract_tail_len(snapshot: &Snapshot) -> usize {
    4 + snapshot
        .definitions()
        .map(|definition| {
            let bodies = definition
                .bodies()
                .map(|body| {
                    8 + 4
                        + body.name().len()
                        + 1
                        + 1
                        + usize::from(body.consumed_by().is_some()) * 8
                })
                .sum::<usize>();
            let ownership = definition
                .feature_ids()
                .iter()
                .map(|feature_id| {
                    let ownership = definition.feature_body_ownership(*feature_id).unwrap();
                    8 + 4
                        + ownership.input_body_ids().len() * 8
                        + 1
                        + usize::from(ownership.output_body_id().is_some()) * 8
                })
                .sum::<usize>();
            8 + 4 + bodies + 8 + 4 + ownership
        })
        .sum::<usize>()
}

/// Product sections appended after schema 34, each written as a u32 count followed
/// by its entries. This fixture holds none of them, so only the counts are removed.
const EMPTY_TRAILING_SECTION_COUNTS: usize = 6;

fn strip_schema_34_tail(bytes: &mut Vec<u8>, snapshot: &Snapshot) {
    bytes.truncate(
        bytes.len() - body_contract_tail_len(snapshot) - 4 * EMPTY_TRAILING_SECTION_COUNTS,
    );
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

    let mut schema_fifteen = persistence::save(&expected);
    let manifest_length = u32::from_le_bytes(schema_fifteen[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    strip_schema_34_tail(&mut schema_fifteen, &expected);
    schema_fifteen.truncate(schema_fifteen.len() - 32);
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
    strip_schema_34_tail(&mut schema_nine, &expected);
    schema_nine.drain(payload_offset + 25..payload_offset + 29);
    schema_nine.truncate(schema_nine.len() - 44);
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
                kind: FeatureKind::full_revolve(BOTTLE_PROFILE),
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
    let changed_snapshot = document.current();
    let changed_digest = changed_snapshot.canonical_digest();
    let request = ExactRevolveRequest::from_snapshot(&changed_snapshot, BOTTLE).unwrap();
    assert_eq!(
        request
            .canonical_input_digest_for_envelope(request.source_revision, &request.source_digest,),
        request.canonical_input_digest
    );
    assert_ne!(changed_digest, initial_digest);
    assert!(matches!(
        document.current().feature(BOTTLE_REVOLVE).unwrap().kind(),
        FeatureKind::Revolve {
            profile: BOTTLE_PROFILE,
            ..
        }
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let loaded = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(loaded.migration_losses().is_empty());
    assert_eq!(loaded.snapshot().canonical_digest(), changed_digest);
    assert!(matches!(
        loaded.snapshot().feature(BOTTLE_PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &changed_profile
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
                kind: FeatureKind::Profile {
                    points_mm: profile.clone(),
                },
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
                tag: None,
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
                    kind: FeatureKind::Profile {
                        points_mm: profile.clone(),
                    },
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
fn general_shell_and_edge_finish_roles_are_canonical_undoable_unique_and_persisted() {
    const DEFINITION: DefinitionId = DefinitionId(40);
    const PROFILE: FeatureId = FeatureId(41);
    const EXTRUSION: FeatureId = FeatureId(42);
    const SHELL: FeatureId = FeatureId(43);
    const FINISH: FeatureId = FeatureId(44);
    const OCCURRENCE: OccurrenceId = OccurrenceId(45);

    let removed_face = StableFaceRole::new("extrusion.top").unwrap();
    let edge = StableEdgeRole::new("shell.edge.top-east").unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "General shell".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 50.0], [0.0, 50.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: height("30"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: SHELL,
                definition_id: DEFINITION,
                name: "Open top".to_owned(),
                kind: FeatureKind::Shell {
                    target: EXTRUSION,
                    removed_faces: vec![removed_face.clone()],
                    thickness: height("2"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FINISH,
                definition_id: DEFINITION,
                name: "Top edge finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: SHELL,
                    edges: vec![edge.clone()],
                    kind: BottleEdgeFinishKind::Fillet,
                    amount: height("1"),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Shelled body".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
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
    let edited_digest = document.current().canonical_digest();
    assert_ne!(edited_digest, initial_digest);
    assert!(matches!(
        document.current().feature(SHELL).unwrap().kind(),
        FeatureKind::Shell { removed_faces, thickness, .. }
            if removed_faces == std::slice::from_ref(&removed_face) && thickness.millimetres() == 2.5
    ));
    assert!(matches!(
        document.current().feature(FINISH).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            edges,
            kind: BottleEdgeFinishKind::Chamfer,
            amount,
            ..
        } if edges == std::slice::from_ref(&edge) && amount.millimetres() == 1.5
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), edited_digest);

    for kind in [
        FeatureKind::Shell {
            target: EXTRUSION,
            removed_faces: vec![removed_face.clone(), removed_face.clone()],
            thickness: height("2"),
        },
        FeatureKind::BottleEdgeFinish {
            target: SHELL,
            edges: Vec::new(),
            kind: BottleEdgeFinishKind::Fillet,
            amount: height("1"),
        },
    ] {
        let before = document.current().canonical_digest();
        let undo_steps = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(99),
                definition_id: DEFINITION,
                name: "Invalid stable selection".to_owned(),
                kind,
            }]))
            .err()
            .expect("non-canonical stable roles must reject the batch");
        assert_eq!(error, CanonicalError::SubshapeRolesNotCanonical);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), undo_steps);
    }

    let before_cross_definition = document.current().canonical_digest();
    let undo_before_cross_definition = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(98),
                name: "Wrong owner".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(98),
                definition_id: DefinitionId(98),
                name: "Cross-definition shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: EXTRUSION,
                    removed_faces: vec![StableFaceRole::new("extrusion.top").unwrap()],
                    thickness: height("2"),
                },
            },
        ]))
        .err()
        .expect("cross-definition stable role target must reject the batch");
    assert_eq!(
        error,
        CanonicalError::InvalidFeatureOwnership(FeatureId(98))
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_cross_definition
    );
    assert_eq!(document.visible_undo_steps(), undo_before_cross_definition);

    document.make_unique(OCCURRENCE, "Unique shell").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    let unique_shell = unique_features[2];
    let unique_finish = unique_features[3];
    assert!(matches!(
        unique.feature(unique_shell).unwrap().kind(),
        FeatureKind::Shell { removed_faces, .. } if removed_faces == &[removed_face]
    ));
    assert!(matches!(
        unique.feature(unique_finish).unwrap().kind(),
        FeatureKind::BottleEdgeFinish { edges, .. } if edges == &[edge]
    ));

    let loaded = persistence::load(&persistence::save(&unique));
    #[cfg(not(feature = "named-product-fixtures"))]
    assert!(matches!(
        loaded,
        Err(PersistenceError::LegacyFeatureRequiresMigration {
            feature_id,
            kind: LegacyFeatureKind::RoleStringShell,
        }) if feature_id == SHELL
    ));
    #[cfg(feature = "named-product-fixtures")]
    {
        let unique_digest = unique.canonical_digest();
        let loaded = loaded.unwrap();
        assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
        assert!(loaded.migration_losses().is_empty());
        assert_eq!(loaded.snapshot().canonical_digest(), unique_digest);
        assert!(matches!(
            loaded.snapshot().feature(unique_shell).unwrap().kind(),
            FeatureKind::Shell { removed_faces, .. }
                if removed_faces[0].as_str() == "extrusion.top"
        ));
        assert!(matches!(
            loaded.snapshot().feature(unique_finish).unwrap().kind(),
            FeatureKind::BottleEdgeFinish { edges, .. }
                if edges[0].as_str() == "shell.edge.top-east"
        ));
    }
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
                kind: FeatureKind::full_revolve(PROFILE),
            },
            CanonicalCommand::CreateFeature {
                id: SHELL,
                definition_id: BOTTLE,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: REVOLVE,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap(),
                    ],
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
        FeatureKind::Shell { target: REVOLVE, thickness, .. }
            if thickness.source_token() == "2.5" && thickness.millimetres() == 2.5
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let reopened = persistence::load(&persistence::save(&document.current()));
    #[cfg(not(feature = "named-product-fixtures"))]
    assert!(matches!(
        reopened,
        Err(PersistenceError::LegacyFeatureRequiresMigration {
            feature_id: SHELL,
            kind: LegacyFeatureKind::RoleStringShell,
        })
    ));
    #[cfg(feature = "named-product-fixtures")]
    {
        let reopened = reopened.unwrap();
        assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
        assert!(reopened.migration_losses().is_empty());
        assert_eq!(reopened.snapshot().canonical_digest(), changed_digest);
    }

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
                kind: FeatureKind::full_revolve(CONTROL),
            },
            CanonicalCommand::CreateFeature {
                id: SHELL,
                definition_id: BOTTLE,
                name: "Bottle shell".to_owned(),
                kind: FeatureKind::Shell {
                    target: REVOLVE,
                    removed_faces: vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE).unwrap(),
                    ],
                    thickness: height("2"),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FINISH,
                definition_id: BOTTLE,
                name: "Shoulder finish".to_owned(),
                kind: FeatureKind::BottleEdgeFinish {
                    target: SHELL,
                    edges: vec![StableEdgeRole::new(BOTTLE_SHOULDER_EDGE_ROLE).unwrap()],
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
            ..
        } if amount.source_token() == "1.5"
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), initial_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), changed_digest);

    let reopened = persistence::load(&persistence::save(&document.current()));
    #[cfg(not(feature = "named-product-fixtures"))]
    assert!(matches!(
        reopened,
        Err(PersistenceError::LegacyFeatureRequiresMigration {
            feature_id: CONTROL,
            kind: LegacyFeatureKind::BottleProfileControl,
        })
    ));
    #[cfg(feature = "named-product-fixtures")]
    {
        let reopened = reopened.unwrap();
        assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
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
    }

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
        FeatureParameterTarget::new(PROFILE, "points.1.x", ParameterValueType::Length).unwrap();
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
    let state = ketchup_core::state_view::encode_semantic_state(&bound);
    for view in [state.complete_v1(), state.agent_v1()] {
        assert!(view.contains("parameter_binding.10.points.1.x.value_type=length"));
        assert!(view.contains("parameter_binding.10.points.1.x.derived_from.root=202"));
        assert!(view.contains(
            "parameter_binding.10.points.1.x.derived_from.slot_path=202:\"dimensions\":\"extrusion_height\""
        ));
    }
    assert!(matches!(
        bound.feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm[1][0] == 600.0
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
        FeatureParameterTarget::new(PROFILE, "points.1.x", ParameterValueType::Angle).unwrap();
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
        loaded.snapshot().feature(PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm[1][0] == 600.0
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
        FeatureParameterTarget::new(EXTRUSION, "height", ParameterValueType::Length).unwrap();
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
                target: FeatureParameterTarget::new(
                    EXTRUSION,
                    "height",
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
                    "height",
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
                            "points.1.x",
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
                        path: ParameterPath::new("height").unwrap(),
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
    for view in [state.complete_v1(), state.agent_v1()] {
        assert!(view.contains("persistent_dimension.1.target=feature:10:points.1.x"));
        assert!(view.contains("persistent_dimension.1.value_type=length"));
        assert!(view.contains("persistent_dimension.3.value_type=length"));
    }
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
            path: ParameterPath::new("height").unwrap(),
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
            FeatureParameterTarget::new(PROFILE, "points.1.x", ParameterValueType::Length).unwrap()
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

fn seed_separate_solid_tool_document(
    tool_x_mm: f64,
    tool_y_mm: f64,
    tool_depth_mm: f64,
) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(101),
                name: "Solid target".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(102),
                definition_id: DefinitionId(101),
                name: "Target profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(103),
                definition_id: DefinitionId(101),
                name: "Target body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(102),
                    height: height("50"),
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(201),
                name: "Solid tool".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(202),
                definition_id: DefinitionId(201),
                name: "Tool profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [0.0, 0.0],
                        [40.0, 0.0],
                        [40.0, tool_depth_mm],
                        [0.0, tool_depth_mm],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(203),
                definition_id: DefinitionId(201),
                name: "Tool body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(202),
                    height: height("50"),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(301),
                definition_id: DefinitionId(101),
                name: "Target occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(302),
                definition_id: DefinitionId(201),
                name: "Tool occurrence".to_owned(),
                transform: Transform::from_translation(tool_x_mm, tool_y_mm, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn separate_solid_tool_plan(operation: BooleanOperation, keep_tool: bool) -> SolidToolPlan {
    SolidToolPlan {
        operation,
        target_occurrence_id: OccurrenceId(301),
        target_feature_id: FeatureId(103),
        tool_occurrence_id: OccurrenceId(302),
        tool_feature_id: FeatureId(203),
        result_definition_id: DefinitionId(401),
        result_feature_ids: vec![
            FeatureId(402),
            FeatureId(403),
            FeatureId(404),
            FeatureId(405),
            FeatureId(406),
            FeatureId(407),
            FeatureId(408),
        ],
        result_definition_name: "Solid Tool result".to_owned(),
        result_feature_name: "Boolean result".to_owned(),
        keep_tool,
    }
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

#[test]
fn separate_occurrence_union_keep_tool_preserves_both_occurrence_identities() {
    let mut document = seed_separate_solid_tool_document(80.0, 0.0, 80.0);
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
            separate_solid_tool_plan(BooleanOperation::Union, true),
        )]))
        .unwrap();
    let applied = document.current().canonical_digest();

    let snapshot = document.current();
    let target = snapshot.occurrence(OccurrenceId(301)).unwrap();
    let tool = snapshot.occurrence(OccurrenceId(302)).unwrap();
    assert_eq!(target.id(), OccurrenceId(301));
    assert_eq!(target.definition_id(), DefinitionId(401));
    assert_eq!(tool.id(), OccurrenceId(302));
    assert_eq!(tool.definition_id(), DefinitionId(201));
    assert_eq!(tool.transform().matrix()[3], 80.0);
    assert!(matches!(
        document.current().feature(FeatureId(407)).unwrap().kind(),
        FeatureKind::RigidTransform {
            target: FeatureId(406),
            transform,
        } if transform.matrix()[3] == 80.0
    ));
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Union,
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
            operation: ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    assert_eq!(document.visible_undo_steps(), 1);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), applied);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_some());
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);
}

#[test]
fn separate_occurrence_intersect_is_canonical_exact_unique_and_persistent() {
    let mut document = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let before = document.current().canonical_digest();
    let intersect = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Intersect, false),
    )]);
    let union = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Union, false),
    )]);
    assert_ne!(intersect.digest(), union.digest());

    document.apply_batch(&intersect).unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    let state = encode_semantic_state(&document.current());
    assert!(
        state
            .complete_v1()
            .contains("feature.408.operation=intersect")
    );
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(301), "Unique intersection")
        .unwrap();
    let unique = document.current();
    let unique_definition = unique
        .occurrence(OccurrenceId(301))
        .unwrap()
        .definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    let [_, _, unique_target, _, _, unique_tool, unique_result] = unique_features else {
        panic!("Intersect Make Unique must preserve the graph-first chain");
    };
    assert!(matches!(
        unique.feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_none());
    assert!(matches!(
        reopened.snapshot().feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let mut touching = seed_separate_solid_tool_document(100.0, 0.0, 30.0);
    let touching_digest = touching.current().canonical_digest();
    assert_eq!(
        touching
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Intersect, true),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(touching.current().canonical_digest(), touching_digest);
    assert_eq!(touching.visible_undo_steps(), 0);
}

#[test]
fn separate_occurrence_split_is_stable_unique_persistent_and_exact_ready() {
    let mut document = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let before = document.current().canonical_digest();
    let split = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Split, true),
    )]);
    let intersect = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Intersect, true),
    )]);
    assert_ne!(split.digest(), intersect.digest());

    document.apply_batch(&split).unwrap();
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
    let applied_snapshot = document.current();
    let splitter = applied_snapshot.occurrence(OccurrenceId(302)).unwrap();
    assert_eq!(splitter.definition_id(), DefinitionId(201));
    assert_eq!(splitter.transform().matrix()[3], 70.0);
    assert_eq!(splitter.transform().matrix()[7], 20.0);
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    assert!(
        encode_semantic_state(&document.current())
            .complete_v1()
            .contains("feature.408.operation=split")
    );
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Split,
            ..
        }
    )));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(301), "Unique split")
        .unwrap();
    let unique = document.current();
    let unique_definition = unique
        .occurrence(OccurrenceId(301))
        .unwrap()
        .definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    let [_, _, unique_target, _, _, unique_tool, unique_result] = unique_features else {
        panic!("Split Make Unique must preserve the graph-first chain");
    };
    assert!(matches!(
        unique.feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_some());
    assert!(matches!(
        reopened.snapshot().feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let mut consume = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let consume_before = consume.current().canonical_digest();
    assert_eq!(
        consume
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Split, false),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(consume.current().canonical_digest(), consume_before);
    assert_eq!(consume.visible_undo_steps(), 0);

    let mut touching = seed_separate_solid_tool_document(100.0, 0.0, 30.0);
    let touching_digest = touching.current().canonical_digest();
    assert_eq!(
        touching
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Split, true),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(touching.current().canonical_digest(), touching_digest);
    assert_eq!(touching.visible_undo_steps(), 0);
}

#[test]
fn bounded_planar_offset_is_dimensioned_validated_undoable_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(701);
    const PROFILE: FeatureId = FeatureId(702);
    const OFFSET: FeatureId = FeatureId(703);
    const OCCURRENCE: OccurrenceId = OccurrenceId(704);

    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Offset profile".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Source rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[10.0, 20.0], [110.0, 20.0], [110.0, 100.0], [10.0, 100.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Planar offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: PROFILE,
                    distance: Dimension::new("5.000", 5.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Offset occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let outward = document.current().canonical_digest();
    assert_ne!(outward, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(OFFSET).unwrap().kind(),
        FeatureKind::PlanarOffset { profile: PROFILE, distance }
            if distance.source_token() == "5.000" && distance.millimetres() == 5.0
    ));
    let state = encode_semantic_state(&document.current());
    assert!(
        state
            .complete_v1()
            .contains("feature.703.kind=planar_offset")
    );
    assert!(
        state
            .complete_v1()
            .contains("feature.703.distance.source=\"5.000\"")
    );
    assert!(state.agent_v1().contains("kind:planar_offset"));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("100000.001", 100_000.001).unwrap(),
            },
        ]))
        .unwrap();
    let large_rectangle_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert!(large_rectangle_request.is_rectangle());
    assert!(large_rectangle_request.has_valid_basic_inputs());
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-7.5", -7.5).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    assert_ne!(inward, outward);
    assert_eq!(document.visible_undo_steps(), 2);
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    for invalid_distance in [0.0, 0.009, -0.009, -40.0, 1_000_000.0] {
        let before = document.current().canonical_digest();
        let undo_steps = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new(invalid_distance.to_string(), invalid_distance)
                        .unwrap(),
                },
            ]))
            .err()
            .expect("invalid offset must reject the whole batch");
        assert_eq!(error, CanonicalError::InvalidPlanarOffset);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), undo_steps);
    }

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: vec![
                    [33_839.523_822_158_46, 20.0],
                    [33_842.954_297_017_83, 20.0],
                    [33_842.954_297_017_83, 100.0],
                    [33_839.523_822_158_46, 100.0],
                ],
            },
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-1.7102374296856577", -1.710_237_429_685_657_7).unwrap(),
            },
        ]))
        .err()
        .expect("floating-point boundary mismatch must fail closed");
    assert_eq!(error, CanonicalError::InvalidPlanarOffset);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    document.make_unique(OCCURRENCE, "Unique offset").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_profile, unique_offset] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Planar Offset Make Unique must preserve the two-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_offset).unwrap().kind(),
        FeatureKind::PlanarOffset { profile, distance }
            if profile == unique_profile
                && distance.source_token() == "-7.5"
                && distance.millimetres() == -7.5
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_offset).unwrap().kind(),
        FeatureKind::PlanarOffset { profile, distance }
            if profile == unique_profile
                && distance.source_token() == "-7.5"
                && distance.millimetres() == -7.5
    ));
}

#[test]
fn circular_planar_offset_is_persistent_undoable_and_fail_closed() {
    const DEFINITION: DefinitionId = DefinitionId(706);
    const WORKPLANE: FeatureId = FeatureId(707);
    const CIRCLE: FeatureId = FeatureId(708);
    const OFFSET: FeatureId = FeatureId(709);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Circular offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: CIRCLE,
                definition_id: DEFINITION,
                name: "Circle sketch".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [12.0, -8.0],
                        radius_mm: 20.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Planar offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: CIRCLE,
                    distance: Dimension::new("3.000", 3.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let outward = document.current().canonical_digest();
    let outward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert_eq!(
        outward_request.source_bounds_mm(),
        [-8.0, -28.0, 32.0, 12.0]
    );
    assert_eq!(outward_request.distance_mm(), 3.0);
    assert_eq!(
        f64::from_bits(outward_request.circle_profile().unwrap().radius_bits),
        20.0
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-3", -3.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("-20", -20.0).unwrap(),
                },
            ]))
            .err()
            .expect("collapsed circular offset must fail closed"),
        CanonicalError::InvalidPlanarOffset
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    let reopened_request =
        ExactPlanarOffsetRequest::from_snapshot(&reopened.snapshot(), DEFINITION).unwrap();
    assert_eq!(reopened_request, inward_request);
}

#[test]
fn cubic_planar_offset_request_is_persistent_undoable_and_forgery_resistant() {
    const DEFINITION: DefinitionId = DefinitionId(710);
    const WORKPLANE: FeatureId = FeatureId(711);
    const SKETCH: FeatureId = FeatureId(712);
    const OFFSET: FeatureId = FeatureId(713);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Cubic planar offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Line cubic sketch".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -15.0],
                            end_mm: [20.0, -15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -15.0],
                            end_mm: [20.0, 15.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 15.0],
                            control_1_mm: [10.0, 25.0],
                            control_2_mm: [-10.0, 25.0],
                            end_mm: [-20.0, 15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 15.0],
                            end_mm: [-20.0, -15.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Cubic offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("2.000", 2.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let outward = document.current().canonical_digest();
    let outward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert!(outward_request.has_valid_basic_inputs());
    assert_eq!(
        outward_request.source_bounds_mm(),
        [-20.0, -15.0, 20.0, 22.5]
    );
    assert!(matches!(
        outward_request.mixed_profile().unwrap().segments.as_slice(),
        [
            ExactBRepPlanarSegment::Line { .. },
            ExactBRepPlanarSegment::Line { .. },
            ExactBRepPlanarSegment::CubicBezier { .. },
            ExactBRepPlanarSegment::Line { .. }
        ]
    ));
    let undo_steps = document.visible_undo_steps();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("0", 0.0).unwrap(),
            },
        ])),
        Err(CanonicalError::InvalidPlanarOffset)
    ));
    assert_eq!(document.current().canonical_digest(), outward);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let mut forged = outward_request.clone();
    let ExactBRepPlanarSegment::CubicBezier { control_1_bits, .. } =
        &mut forged.profile.as_mut().unwrap().segments[2]
    else {
        panic!("fixture must preserve its cubic segment");
    };
    control_1_bits[1] = 24.0_f64.to_bits();
    assert!(!forged.has_valid_basic_inputs());

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-2", -2.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        ExactPlanarOffsetRequest::from_snapshot(&reopened.snapshot(), DEFINITION).unwrap(),
        inward_request
    );
}

#[test]
fn compound_planar_offset_request_is_persistent_undoable_and_forgery_resistant() {
    const DEFINITION: DefinitionId = DefinitionId(714);
    const WORKPLANE: FeatureId = FeatureId(715);
    const SKETCH: FeatureId = FeatureId(716);
    const OFFSET: FeatureId = FeatureId(717);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Compound planar offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Line-cubic region with hole".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -15.0],
                            end_mm: [20.0, -15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -15.0],
                            end_mm: [20.0, 15.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 15.0],
                            control_1_mm: [10.0, 25.0],
                            control_2_mm: [-10.0, 25.0],
                            end_mm: [-20.0, 15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 15.0],
                            end_mm: [-20.0, -15.0],
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(5),
                            center_mm: [0.0, 0.0],
                            radius_mm: 5.0,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Compound offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("2.000", 2.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let outward = document.current().canonical_digest();
    let outward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert!(outward_request.has_valid_basic_inputs());
    assert_eq!(
        outward_request.source_bounds_mm(),
        [-20.0, -15.0, 20.0, 22.5]
    );
    let region = outward_request.region_profile().unwrap();
    assert!(matches!(
        &region.outer,
        ExactBRepPlanarLoop::Boundary { segments }
            if segments.iter().any(|segment| matches!(
                segment,
                ExactBRepPlanarSegment::CubicBezier { .. }
            ))
    ));
    assert!(matches!(
        region.holes.as_slice(),
        [ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        }] if center_bits.map(f64::from_bits) == [0.0, 0.0]
            && f64::from_bits(*radius_bits) == 5.0
    ));

    let mut forged = outward_request.clone();
    let ExactBRepPlanarLoop::Circle { radius_bits, .. } =
        &mut forged.region.as_mut().unwrap().holes[0]
    else {
        panic!("fixture must preserve its circular hole");
    };
    *radius_bits = 4.0_f64.to_bits();
    assert!(!forged.has_valid_basic_inputs());

    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("5", 5.0).unwrap(),
                },
            ]))
            .err()
            .expect("collapsed hole must fail closed"),
        CanonicalError::InvalidPlanarOffset
    );
    assert_eq!(document.current().canonical_digest(), outward);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-2", -2.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request =
        ExactPlanarOffsetRequest::from_snapshot(&document.current(), DEFINITION).unwrap();
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        ExactPlanarOffsetRequest::from_snapshot(&reopened.snapshot(), DEFINITION).unwrap(),
        inward_request
    );
}

#[test]
fn compound_planar_offset_rejects_inter_loop_collision_before_history() {
    const DEFINITION: DefinitionId = DefinitionId(718);
    const WORKPLANE: FeatureId = FeatureId(719);
    const SKETCH: FeatureId = FeatureId(720);
    const OFFSET: FeatureId = FeatureId(721);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Close-hole compound offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Region with close holes".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -20.0],
                            end_mm: [20.0, -20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -20.0],
                            end_mm: [20.0, 20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 20.0],
                            end_mm: [-20.0, 20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 20.0],
                            end_mm: [-20.0, -20.0],
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(5),
                            center_mm: [-3.0, 0.0],
                            radius_mm: 2.0,
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(6),
                            center_mm: [3.0, 0.0],
                            radius_mm: 2.0,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Safe outward material offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("1", 1.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("-1", -1.0).unwrap(),
                },
            ]))
            .err(),
        Some(CanonicalError::InvalidPlanarOffset)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);
}

#[test]
fn bounded_multisegment_profile_sweep_is_validated_undoable_visible_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(711);
    const PROFILE: FeatureId = FeatureId(712);
    const PATH: FeatureId = FeatureId(713);
    const SWEEP: FeatureId = FeatureId(714);
    const OCCURRENCE: OccurrenceId = OccurrenceId(715);

    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Sweep definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangular section".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Tangent line-arc path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [50.0, 0.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [50.0, 0.0],
                            end_mm: [75.0, 25.0],
                            center_mm: [50.0, 25.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [75.0, 25.0],
                            end_mm: [75.0, 50.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Bounded sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Sweep occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let swept = document.current().canonical_digest();
    assert_ne!(swept, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty);
    assert_eq!(document.redo().unwrap().canonical_digest(), swept);
    assert!(matches!(
        document.current().feature(SWEEP).unwrap().kind(),
        FeatureKind::Sweep {
            profile: PROFILE,
            path: PATH,
        }
    ));
    let state = encode_semantic_state(&document.current());
    assert!(state.complete_v1().contains("feature.714.kind=sweep"));
    assert!(state.complete_v1().contains("feature.714.profile=712"));
    assert!(state.complete_v1().contains("feature.714.path=713"));
    assert!(state.agent_v1().contains("kind:sweep"));

    document.make_unique(OCCURRENCE, "Unique sweep").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_profile, unique_path, unique_sweep] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Sweep Make Unique must preserve the three-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_sweep).unwrap().kind(),
        FeatureKind::Sweep { profile, path }
            if profile == unique_profile && path == unique_path
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_sweep).unwrap().kind(),
        FeatureKind::Sweep { profile, path }
            if profile == unique_profile && path == unique_path
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Invalid sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Bent path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [0.0, 50.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [0.0, 50.0],
                            end_mm: [25.0, 50.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Unsupported bent sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ]))
        .err()
        .expect("bounded Sweep must reject a multi-segment path atomically");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Degenerate segment sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Sub-tolerance path segment".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [5.0e-8, 0.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [5.0e-8, 0.0],
                            end_mm: [10.0, 0.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Degenerate segment sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ]))
        .err()
        .expect("Sweep must reject a sub-tolerance path segment atomically");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Unsupported spline sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Spline section".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Straight path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [0.0, 125.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Unsupported spline sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ]))
        .err()
        .expect("Sweep must reject a spline profile before exact evaluation");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn bounded_spline_profile_loft_is_validated_undoable_visible_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(721);
    const LOWER: FeatureId = FeatureId(722);
    const UPPER: FeatureId = FeatureId(723);
    const LOFT: FeatureId = FeatureId(724);
    const OCCURRENCE: OccurrenceId = OccurrenceId(725);

    let lower_points = vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]];
    let upper_points = vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]];
    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Loft definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Lower spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: lower_points.clone(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: UPPER,
                definition_id: DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: upper_points.clone(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: LOFT,
                definition_id: DEFINITION,
                name: "Bounded loft".to_owned(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: LOWER,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: UPPER,
                            elevation_mm: 80.0,
                        },
                    ],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Loft occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let lofted = document.current().canonical_digest();
    assert_ne!(lofted, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty);
    assert_eq!(document.redo().unwrap().canonical_digest(), lofted);
    let state = encode_semantic_state(&document.current());
    assert!(
        state
            .complete_v1()
            .contains("feature.722.kind=spline_profile")
    );
    assert!(state.complete_v1().contains("feature.724.kind=loft"));
    assert!(
        state
            .complete_v1()
            .contains("feature.724.section.1=profile:723")
    );
    assert!(state.agent_v1().contains("kind:spline_profile"));
    assert!(state.agent_v1().contains("kind:loft"));

    document.make_unique(OCCURRENCE, "Unique loft").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_lower, unique_upper, unique_loft] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Loft Make Unique must preserve the three-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_lower).unwrap().kind(),
        FeatureKind::SplineProfile { control_points_mm } if control_points_mm == &lower_points
    ));
    assert!(matches!(
        unique.feature(*unique_upper).unwrap().kind(),
        FeatureKind::SplineProfile { control_points_mm } if control_points_mm == &upper_points
    ));
    assert!(matches!(
        unique.feature(*unique_loft).unwrap().kind(),
        FeatureKind::Loft { sections }
            if sections[0].profile == *unique_lower
                && sections[0].elevation_mm == 0.0
                && sections[1].profile == *unique_upper
                && sections[1].elevation_mm == 80.0
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_loft).unwrap().kind(),
        FeatureKind::Loft { sections }
            if sections[0].profile == *unique_lower
                && sections[1].profile == *unique_upper
    ));

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(726),
            definition_id: unique_definition,
            name: "Invalid unordered loft".to_owned(),
            kind: FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: *unique_lower,
                        elevation_mm: 80.0,
                    },
                    LoftSection {
                        profile: *unique_upper,
                        elevation_mm: 0.0,
                    },
                ],
            },
        }]))
        .err()
        .expect("unordered Loft sections must reject atomically");
    assert_eq!(error, CanonicalError::InvalidLoft);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let mut invalid_spline = DocumentStore::new();
    let empty = invalid_spline.current().canonical_digest();
    let error = invalid_spline
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Invalid spline definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Underspecified spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
                },
            },
        ]))
        .err()
        .expect("underspecified spline profile must reject atomically");
    assert_eq!(error, CanonicalError::InvalidSplineProfile);
    assert_eq!(invalid_spline.current().canonical_digest(), empty);
    assert_eq!(invalid_spline.visible_undo_steps(), 0);
}

#[test]
fn separate_occurrence_solid_tool_defers_geometry_validity_to_exact_worker() {
    let mut document = seed_separate_solid_tool_document(100.0, 0.0, 80.0);
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
            separate_solid_tool_plan(BooleanOperation::Union, true),
        )]))
        .unwrap();

    assert_ne!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408),)
            .is_ok()
    );
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(401)
    );
    assert!(document.current().occurrence(OccurrenceId(302)).is_some());
}

fn circular_profile_segments(clockwise: bool) -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::CircularArc {
            start_mm: [10.0, 0.0],
            end_mm: [-10.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: [-10.0, 0.0],
            end_mm: [10.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise,
        },
    ]
}

#[test]
fn segment_profile_is_canonical_undoable_persistent_and_exact_for_circle() {
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(501),
                name: "Circle definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(502),
                definition_id: DefinitionId(501),
                name: "Exact circle".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circular_profile_segments(false),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(503),
                definition_id: DefinitionId(501),
                name: "Pending exact cylinder".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(502),
                    height: height("25"),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(504),
                definition_id: DefinitionId(501),
                name: "Circle occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(FeatureId(502)).unwrap().kind(),
        FeatureKind::SegmentProfile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));
    let exact_request =
        ExactFeatureChainRequest::from_snapshot(&document.current(), DefinitionId(501)).unwrap();
    assert_eq!(exact_request.evaluator(), EXACT_CIRCLE_EVALUATOR_V1);
    assert_eq!(
        exact_request.expected_bounds_mm(),
        [[-10.0, -10.0, 0.0], [10.0, 10.0, 25.0]]
    );
    let circle = exact_request.circle.expect("analytic circle request");
    assert_eq!(f64::from_bits(circle.radius_bits), 10.0);
    assert_eq!(f64::from_bits(circle.center_x_bits), 0.0);
    assert!(!circle.clockwise);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), applied);
    assert!(matches!(
        reopened.snapshot().feature(FeatureId(502)).unwrap().kind(),
        FeatureKind::SegmentProfile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(504), "Unique circle")
        .unwrap();
    let unique_snapshot = document.current();
    let unique_definition_id = unique_snapshot
        .occurrence(OccurrenceId(504))
        .unwrap()
        .definition_id();
    assert_ne!(unique_definition_id, DefinitionId(501));
    let cloned_segment_profile = unique_snapshot
        .definition(unique_definition_id)
        .unwrap()
        .feature_ids()
        .iter()
        .filter_map(|feature_id| unique_snapshot.feature(*feature_id))
        .find(|feature| matches!(feature.kind(), FeatureKind::SegmentProfile { .. }))
        .expect("make unique must clone the segment-authoritative profile");
    assert!(matches!(
        cloned_segment_profile.kind(),
        FeatureKind::SegmentProfile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));

    let mut opposite = DocumentStore::new();
    opposite
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(501),
                name: "Circle definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(502),
                definition_id: DefinitionId(501),
                name: "Exact circle".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: circular_profile_segments(true),
                    closed: true,
                },
            },
        ]))
        .unwrap();
    assert_ne!(
        opposite.current().canonical_digest(),
        reopened.snapshot().canonical_digest(),
        "arc direction is authoritative canonical geometry"
    );
}

#[test]
fn segment_profile_rejects_discontinuity_and_radius_mismatch_atomically() {
    for segments in [
        vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [6.0, 0.0],
                end_mm: [0.0, 0.0],
            },
        ],
        vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [5.0 + 1.0e-12, 0.0],
                end_mm: [0.0, 0.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [10.0, 0.0],
                end_mm: [-9.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [-9.0, 0.0],
                end_mm: [10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
        ],
    ] {
        let mut document = DocumentStore::new();
        let before = document.current().canonical_digest();
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(601),
                    name: "Invalid profile definition".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(602),
                    definition_id: DefinitionId(601),
                    name: "Invalid segment profile".to_owned(),
                    kind: FeatureKind::SegmentProfile {
                        segments,
                        closed: true,
                    },
                },
            ]))
            .err()
            .expect("invalid segment profile must fail");
        assert_eq!(error, CanonicalError::InvalidProfile);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), 0);
    }
}

#[test]
fn v11_cubic_sweep_is_canonical_visible_persistent_and_fail_closed() {
    const DEFINITION: DefinitionId = DefinitionId(800);
    const PROFILE: FeatureId = FeatureId(801);
    const PATH: FeatureId = FeatureId(802);
    const SWEEP: FeatureId = FeatureId(803);

    let path_segments = vec![
        ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [25.0, 0.0],
        },
        ProfileSegment::CubicBezier {
            start_mm: [25.0, 0.0],
            control_1_mm: [35.0, 0.0],
            control_2_mm: [45.0, 10.0],
            end_mm: [45.0, 20.0],
        },
        ProfileSegment::Line {
            start_mm: [45.0, 20.0],
            end_mm: [45.0, 45.0],
        },
    ];
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "V11 cubic sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Small rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Line-cubic-line C1 path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "V11 sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ])
    };

    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document
        .apply_batch(&commands(path_segments.clone()))
        .unwrap();
    let swept_digest = document.current().canonical_digest();
    assert_ne!(swept_digest, empty_digest);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), swept_digest);
    assert!(
        encode_semantic_state(&document.current())
            .complete_v1()
            .contains("feature.802.segment.1=cubic_bezier")
    );

    let bytes = persistence::save(&document.current());
    assert_eq!(
        u16::from_le_bytes([bytes[10], bytes[11]]),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), swept_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::SegmentProfile { segments, closed: false } if segments == &path_segments
    ));

    let mut invalid_segments = path_segments;
    invalid_segments[1] = ProfileSegment::CubicBezier {
        start_mm: [25.0, 0.0],
        control_1_mm: [25.0, 0.0],
        control_2_mm: [45.0, 10.0],
        end_mm: [45.0, 20.0],
    };
    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    assert!(matches!(
        invalid.apply_batch(&commands(invalid_segments)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let adjacent_self_intersection = vec![
        ProfileSegment::CubicBezier {
            start_mm: [0.0, 0.0],
            control_1_mm: [10.0, -5.0],
            control_2_mm: [9.0, 10.0],
            end_mm: [10.0, 10.0],
        },
        ProfileSegment::CubicBezier {
            start_mm: [10.0, 10.0],
            control_1_mm: [11.0, 10.0],
            control_2_mm: [-10.0, -20.0],
            end_mm: [20.0, 0.0],
        },
    ];
    assert!(matches!(
        invalid.apply_batch(&commands(adjacent_self_intersection)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn spatial_sweep_path_is_canonical_visible_persistent_and_bounded() {
    const DEFINITION: DefinitionId = DefinitionId(900);
    const PATH: FeatureId = FeatureId(902);
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Spatial sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Spatial path".to_owned(),
                kind: FeatureKind::SpatialPath { segments },
            },
        ])
    };
    let path = vec![SpatialPathSegment::Line {
        start_mm: [2.0, 3.0, 4.0],
        end_mm: [2.0, 3.0, 24.0],
    }];
    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document.apply_batch(&commands(path.clone())).unwrap();
    let digest = document.current().canonical_digest();
    assert_ne!(digest, empty_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), digest);
    assert!(
        encode_semantic_state(&document.current())
            .complete_v1()
            .contains("feature.902.segment.0=line")
    );

    let bytes = persistence::save(&document.current());
    assert_eq!(
        u16::from_le_bytes([bytes[10], bytes[11]]),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments == &path
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let segments = (0..65)
        .map(|index| SpatialPathSegment::Line {
            start_mm: [f64::from(index), 0.0, 0.0],
            end_mm: [f64::from(index + 1), 0.0, 0.0],
        })
        .collect();
    assert!(matches!(
        invalid.apply_batch(&commands(segments)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn v12_spatial_sweep_is_canonical_persistent_and_rejects_uncompilable_bounds() {
    const DEFINITION: DefinitionId = DefinitionId(910);
    const PROFILE: FeatureId = FeatureId(911);
    const PATH: FeatureId = FeatureId(912);
    const SWEEP: FeatureId = FeatureId(913);
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "V12 spatial sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Small rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Spatial path".to_owned(),
                kind: FeatureKind::SpatialPath { segments },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Spatial sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ])
    };
    let path = vec![
        SpatialPathSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [10.0, 0.0, 0.0],
        },
        SpatialPathSegment::CircularArc {
            start_mm: [10.0, 0.0, 0.0],
            end_mm: [20.0, 10.0, 0.0],
            center_mm: [10.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [20.0, 10.0, 0.0],
            control_1_mm: [20.0, 15.0, 0.0],
            control_2_mm: [20.0, 20.0, 5.0],
            end_mm: [20.0, 20.0, 10.0],
        },
    ];

    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document.apply_batch(&commands(path.clone())).unwrap();
    let sweep_digest = document.current().canonical_digest();
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), sweep_digest);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), sweep_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments == &path
    ));
    assert!(matches!(
        reopened.snapshot().feature(SWEEP).unwrap().kind(),
        FeatureKind::Sweep {
            profile: PROFILE,
            path: PATH
        }
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let outside_compilable_envelope = vec![SpatialPathSegment::Line {
        start_mm: [999_999.5, 0.0, 0.0],
        end_mm: [999_999.5, 0.0, 10.0],
    }];
    assert!(matches!(
        invalid.apply_batch(&commands(outside_compilable_envelope)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let adjacent_self_intersection = vec![
        SpatialPathSegment::CubicBezier {
            start_mm: [0.0, 0.0, 0.0],
            control_1_mm: [10.0, -5.0, 0.0],
            control_2_mm: [9.0, 10.0, 0.0],
            end_mm: [10.0, 10.0, 0.0],
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [10.0, 10.0, 0.0],
            control_1_mm: [11.0, 10.0, 0.0],
            control_2_mm: [-10.0, -20.0, 0.0],
            end_mm: [20.0, 0.0, 0.0],
        },
    ];
    assert!(matches!(
        invalid.apply_batch(&commands(adjacent_self_intersection)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}
