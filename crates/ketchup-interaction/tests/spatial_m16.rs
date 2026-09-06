use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_interaction::exact_projection::ExactInteractionProjection;
use ketchup_interaction::mesh_projection::MeshInteractionProjection;
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_interaction::spatial::{
    SPATIAL_INDEX_V1, SpatialQueryError, overlapping_bounds_for_sources,
    overlapping_bounds_for_sources_with_cancellation, overlapping_bounds_pairs,
};
use ketchup_interaction::{Ray, Vec3};
use std::sync::{Arc, atomic::AtomicBool};

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(1);
const BODY: FeatureId = FeatureId(2);
const OCCURRENCES: usize = 1_024;

fn grid_transform(index: usize) -> Transform {
    Transform::from_translation((index % 64) as f64 * 20.0, (index / 64) as f64 * 20.0, 0.0)
        .unwrap()
}

fn box_document() -> DocumentStore {
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Indexed box".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: BODY,
            definition_id: DEFINITION,
            name: "Extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
    ];
    commands.extend(
        (0..OCCURRENCES).map(|index| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(index as u64 + 1),
            definition_id: DEFINITION,
            name: format!("Occurrence {}", index + 1),
            transform: grid_transform(index),
            parent: None,
            tag: None,
            visible: true,
        }),
    );
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
}

fn target_ray(index: usize) -> Ray {
    let transform = grid_transform(index);
    let matrix = transform.matrix();
    Ray::new(
        Vec3::new(matrix[3] + 2.0, matrix[7] + 2.0, 30.0),
        Vec3::new(0.0, 0.0, -1.0),
    )
    .unwrap()
}

fn assert_sublinear(stats: ketchup_interaction::spatial::SpatialQueryStats) {
    assert_eq!(stats.indexed_items, OCCURRENCES);
    assert!(stats.candidate_count <= 4, "{stats:?}");
    assert!(stats.bounds_tested < OCCURRENCES / 8, "{stats:?}");
}

fn exact_package(snapshot: &ketchup_core::document::Snapshot) -> ExactBodyPackage {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                request.document_id,
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}"),
        )
    });
    build_box_render_package(
        &request,
        "exact-input".to_owned(),
        "result".to_owned(),
        "test-backend".to_owned(),
        "test-tolerance".to_owned(),
        [[0.0; 3], request.dimensions_mm()],
        evidence,
    )
    .unwrap()
    .into()
}

fn mesh_document() -> DocumentStore {
    let mesh = MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
        authority: MeshAuthority::Authored {
            provenance: "spatial-m16-fixture".to_owned(),
        },
    };
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Indexed mesh".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: BODY,
            definition_id: DEFINITION,
            name: "Tetrahedron".to_owned(),
            kind: FeatureKind::MeshBody(mesh),
        },
    ];
    commands.extend(
        (0..OCCURRENCES).map(|index| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(index as u64 + 1),
            definition_id: DEFINITION,
            name: format!("Mesh occurrence {}", index + 1),
            transform: grid_transform(index),
            parent: None,
            tag: None,
            visible: true,
        }),
    );
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
}

#[test]
fn broad_phase_pairs_are_deterministic_boundary_safe_and_sparse_at_10k() {
    let touching = [
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        [[10.0, 0.0, 0.0], [20.0, 10.0, 10.0]],
        [[30.0, 0.0, 0.0], [40.0, 10.0, 10.0]],
    ];
    let first = overlapping_bounds_pairs(&touching).unwrap();
    assert_eq!(first.0, vec![(0, 1)]);
    assert_eq!(overlapping_bounds_pairs(&touching).unwrap(), first);
    assert_eq!(
        overlapping_bounds_for_sources(&touching, &[0], 1)
            .unwrap()
            .0,
        vec![(0, 1)]
    );
    assert_eq!(
        overlapping_bounds_for_sources(&touching, &[3], 1),
        Err(SpatialQueryError::InvalidSourceIndex)
    );
    let dense = vec![[[0.0; 3], [1.0; 3]]; 32];
    assert_eq!(
        overlapping_bounds_for_sources(&dense, &[0], 10),
        Err(SpatialQueryError::CandidateLimitExceeded)
    );
    assert_eq!(
        overlapping_bounds_for_sources(&dense, &[0], 0),
        Err(SpatialQueryError::CandidateLimitExceeded)
    );
    assert_eq!(
        overlapping_bounds_for_sources_with_cancellation(&dense, &[0], 10, &AtomicBool::new(true),),
        Err(SpatialQueryError::Cancelled)
    );

    let sparse = (0..10_000)
        .map(|index| {
            let x = index as f64 * 20.0;
            [[x, 0.0, 0.0], [x + 10.0, 10.0, 10.0]]
        })
        .collect::<Vec<_>>();
    let (pairs, stats) = overlapping_bounds_pairs(&sparse).unwrap();
    assert!(pairs.is_empty());
    assert_eq!(stats.indexed_items, 10_000);
    assert_eq!(stats.candidate_count, 0);
    assert!(stats.bounds_tested < 10_000 * 64, "{stats:?}");

    for invalid in [
        [[1.0, 0.0, 0.0], [0.0, 1.0, 1.0]],
        [[0.0, 0.0, 0.0], [f64::NAN, 1.0, 1.0]],
    ] {
        assert_eq!(
            overlapping_bounds_pairs(&[invalid]),
            Err(SpatialQueryError::InvalidBounds)
        );
    }
}

#[test]
fn canonical_bvh_is_deterministic_sublinear_and_stale_safe() {
    let mut store = box_document();
    let before = store.current();
    let scene = CanonicalInteractionProjection::from_snapshot(&before)
        .scene()
        .unwrap();
    let target = OCCURRENCES - 1;
    let (hit, stats) = scene.exact_pick_with_stats(target_ray(target), 0.01);
    assert_eq!(scene.spatial_index_schema(), SPATIAL_INDEX_V1);
    assert_eq!(
        hit.unwrap().primary.reference.instance_path,
        InstancePath::root(OccurrenceId(target as u64 + 1))
    );
    assert_sublinear(stats);

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(target as u64 + 1),
                transform: Transform::from_translation(5_000.0, 5_000.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let after = store.current();
    assert_eq!(
        scene.exact_pick_current(&after, target_ray(target), 0.01),
        Err(SpatialQueryError::StaleProjection)
    );
    let rebuilt = CanonicalInteractionProjection::from_snapshot(&after)
        .scene()
        .unwrap();
    assert!(rebuilt.is_current(&after));
    assert!(rebuilt.exact_pick(target_ray(target), 0.01).is_none());
}

#[test]
fn exact_bvh_preserves_durable_identity_with_sublinear_candidates() {
    let mut store = box_document();
    let before = store.current();
    let package = Arc::new(exact_package(&before));
    let registry = ExactResultRegistry::accept(&before, [package]).unwrap();
    let projection = ExactInteractionProjection::from_snapshot(&before, &registry);
    let target = OCCURRENCES / 2;
    let (hit, stats) = projection.exact_surface_pick_with_stats(target_ray(target));
    let hit = hit.unwrap();
    assert_eq!(projection.spatial_index_schema(), SPATIAL_INDEX_V1);
    assert_eq!(
        hit.instance_path,
        InstancePath::root(OccurrenceId(target as u64 + 1))
    );
    assert_eq!(
        hit.durable_target.unwrap().body.role(),
        Some(ExactFaceRole::Top)
    );
    assert_sublinear(stats);

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(2_000.0, 2_000.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(
        projection.exact_surface_pick_current(&store.current(), target_ray(target)),
        Err(SpatialQueryError::StaleProjection)
    );
}

#[test]
fn canonical_mesh_projection_shares_geometry_and_uses_the_same_stale_safe_bvh() {
    let mut store = mesh_document();
    let before = store.current();
    let projection = MeshInteractionProjection::from_snapshot(&before);
    let target = OCCURRENCES / 3;
    let (hit, stats) = projection.exact_surface_pick_with_stats(target_ray(target));
    let hit = hit.unwrap();
    assert_eq!(projection.spatial_index_schema(), SPATIAL_INDEX_V1);
    assert_eq!(projection.occurrence_count(), OCCURRENCES);
    assert_eq!(projection.shared_geometry_count(), 1);
    assert_eq!(hit.definition_id, DEFINITION);
    assert_eq!(hit.feature_id, BODY);
    assert_eq!(
        hit.instance_path,
        InstancePath::root(OccurrenceId(target as u64 + 1))
    );
    assert_sublinear(stats);

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let after = store.current();
    assert_eq!(
        projection.exact_surface_pick_current(&after, target_ray(target)),
        Err(SpatialQueryError::StaleProjection)
    );
    let rebuilt = MeshInteractionProjection::from_snapshot(&after);
    assert!(rebuilt.is_current(&after));
    assert_eq!(rebuilt.occurrence_count(), OCCURRENCES - 1);
}
