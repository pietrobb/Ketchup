//! Test doubles for code that needs exact evaluation results without running
//! the OCCT worker. Enabled by the `testing` feature; never used by the product.

use crate::document::{DefinitionId, FeatureId, Snapshot};
use crate::exact_brep_graph::ExactBRepGraph;
use crate::exact_product::{
    ExactBRepGraphFaceEvidence, ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence,
    ExactBodyPackage, ExactFaceRole, ExactProductError,
};
use crate::import::{StepImportMesh, StepMeshTriangle};

/// Faces a box result can name, with the axis and side they lie on.
const NAMED_BOX_FACES: [(ExactFaceRole, usize, bool); 4] = [
    (ExactFaceRole::Top, 2, true),
    (ExactFaceRole::Bottom, 2, false),
    (ExactFaceRole::East, 0, true),
    (ExactFaceRole::West, 0, false),
];

/// Stands in for the exact worker: the result of `producer_feature_id` as the
/// axis-aligned box that fills its graph's bounds, with the planar `faces`
/// named the way the evaluator names them. `result_fingerprint` tells results
/// apart.
///
/// # Errors
/// Fails when the producer does not compile to an exact graph, or when a face
/// role is not one of top, bottom, east or west.
pub fn box_package(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    result_fingerprint: &str,
    faces: &[ExactFaceRole],
) -> Result<ExactBodyPackage, ExactProductError> {
    let graph = ExactBRepGraph::from_snapshot(snapshot, definition_id, producer_feature_id)
        .map_err(|_| ExactProductError::UnsupportedDefinition)?;
    let bounds_mm = graph
        .producer_bounds_mm()
        .ok()
        .flatten()
        .ok_or(ExactProductError::UnsupportedDefinition)?;
    let [minimum, maximum] = bounds_mm;
    let size = [0, 1, 2].map(|axis| maximum[axis] - minimum[axis]);
    let corner = |x: bool, y: bool, z: bool| {
        [
            if x { maximum[0] } else { minimum[0] },
            if y { maximum[1] } else { minimum[1] },
            if z { maximum[2] } else { minimum[2] },
        ]
    };
    let vertices_mm = (0..8)
        .map(|index| corner(index & 1 != 0, index & 2 != 0, index & 4 != 0))
        .collect::<Vec<_>>();
    // Face ordinal 2 * axis + side, two outward-wound triangles each.
    let quads: [[u32; 4]; 6] = [
        [0, 4, 6, 2],
        [1, 3, 7, 5],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 2, 3, 1],
        [4, 5, 7, 6],
    ];
    let triangles = quads
        .iter()
        .enumerate()
        .flat_map(|(ordinal, [a, b, c, d])| {
            [[*a, *b, *c], [*a, *c, *d]].map(|vertex_indices| StepMeshTriangle {
                vertex_indices,
                face_ordinal: ordinal as u32,
            })
        })
        .collect();
    let faces = faces
        .iter()
        .map(|role| {
            let (_, axis, positive) = NAMED_BOX_FACES
                .iter()
                .copied()
                .find(|(named, _, _)| named == role)
                .ok_or(ExactProductError::UnsupportedDefinition)?;
            let mut centroid_mm = [0, 1, 2].map(|index| (minimum[index] + maximum[index]) / 2.0);
            centroid_mm[axis] = if positive {
                maximum[axis]
            } else {
                minimum[axis]
            };
            let mut unit_normal = [0.0; 3];
            unit_normal[axis] = if positive { 1.0 } else { -1.0 };
            Ok(ExactBRepGraphFaceEvidence {
                semantic_role: role.semantic_role().to_owned(),
                source_element_id: role.source_element_id().to_owned(),
                face_ordinal: (2 * axis + usize::from(positive)) as u32,
                surface_kind: "plane".to_owned(),
                corroborating_geometry_fingerprint: format!("fixture-{role:?}"),
                centroid_mm,
                unit_normal,
                axis_origin_mm: None,
                unit_axis_direction: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = ExactBRepGraphWorkerEvidence {
        exact_input_digest: "fixture-input".to_owned(),
        result_fingerprint: result_fingerprint.to_owned(),
        volume_mm3: size[0] * size[1] * size[2],
        // Solid results report volume only.
        area_mm2: 0.0,
        topology_counts: [8, 12, 6, 1, 1],
        wire_count: None,
        bounds_mm,
        backend: "fixture-backend".to_owned(),
        tolerance: "fixture-tolerance".to_owned(),
        faces,
        edges: Vec::new(),
    };
    ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        evidence,
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .map(ExactBodyPackage::Graph)
}
