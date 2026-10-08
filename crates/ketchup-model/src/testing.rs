//! Test doubles for code that needs exact evaluation results without running
//! the OCCT worker, a way to write invalid saved documents, and a way to compare a
//! model built by commands with the same model read from a committed file. Enabled by
//! the `testing` feature; never used by the product.

use crate::document::{DefinitionId, DocumentId, DocumentStore, FeatureId, Snapshot};
use crate::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use crate::exact_product::{
    ExactBRepGraphFaceEvidence, ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence,
    ExactBodyPackage, ExactFaceRole, ExactProductError,
};
use crate::import::{StepImportMesh, StepMeshTriangle};

/// Stands in for the exact worker: the result of `producer_feature_id` as the
/// axis-aligned box that fills its graph's bounds, with the planar `faces`
/// named the way the evaluator names them: the caps of the extrusion and the
/// side of its profile's first line. `result_fingerprint` tells results apart.
///
/// # Errors
/// Fails when the producer does not compile to an exact graph, or when a named
/// face is not an axis-aligned planar face of its extrusion.
pub fn box_package(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    result_fingerprint: &str,
    faces: &[ExactFaceRole],
) -> Result<ExactBodyPackage, ExactProductError> {
    let graph = ExactBRepGraph::from_snapshot(snapshot, definition_id, producer_feature_id)
        .map_err(|error| ExactProductError::UnsupportedDefinition.because(error))?;
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
    let profile_feature_id = graph
        .nodes
        .iter()
        .find_map(|node| match &node.operation {
            ExactBRepOperation::Extrude { profile, .. } => graph.profiles.get(profile.0 as usize),
            _ => None,
        })
        .ok_or(ExactProductError::UnsupportedDefinition)?
        .source_feature_id;
    let faces = faces
        .iter()
        .map(|role| {
            let normal = graph
                .extrusion_face_frame(profile_feature_id, role.semantic_role())
                .ok_or(ExactProductError::UnsupportedDefinition)?
                .normal;
            let axis = (0..3)
                .find(|axis| normal[*axis].abs() == 1.0)
                .ok_or(ExactProductError::UnsupportedDefinition)?;
            let positive = normal[axis] > 0.0;
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

/// Returns `snapshot` as if it belonged to document `document_id`. Every new document
/// gets its own identity, so a model built by commands in a test matches a committed
/// file only once both carry the identity stored in that file.
///
/// # Panics
/// Panics when `snapshot` stops being a valid document, which a new identity cannot cause.
#[must_use]
pub fn with_document_id(snapshot: &Snapshot, document_id: DocumentId) -> Snapshot {
    let mut product = snapshot.product().clone();
    product.document_id = document_id;
    DocumentStore::from_product(snapshot.revision_id(), product)
        .expect("a document stays valid under another identity")
        .current()
}

/// Edits the snapshot stored in `saved` (the output of `persistence::save`), a CBOR map
/// `{revision_id, product}`, and reseals the checksum, so a test can hand the loader data
/// that no command would produce. Map keys are the serde field names of the document types.
#[must_use]
pub fn rewrite_saved_snapshot(saved: &[u8], edit: impl FnOnce(&mut ciborium::Value)) -> Vec<u8> {
    crate::persistence::snapshot_codec::rewrite(saved, edit)
}

/// The current revision of `document` saved as a native container whose `history.bin`
/// is `history`, so a test can hand the loader an Undo history no writer produced.
#[must_use]
pub fn save_with_history_entry(document: &DocumentStore, history: Vec<u8>) -> Vec<u8> {
    crate::persistence::save_with_history_entry(document, history)
}

/// Returns the entry of a CBOR map by text key or integer key.
///
/// # Panics
/// Panics when `value` is not a map or has no such entry.
pub fn cbor_entry(
    value: &mut ciborium::Value,
    key: impl Into<ciborium::Value>,
) -> &mut ciborium::Value {
    let key = key.into();
    value
        .as_map_mut()
        .expect("a CBOR map")
        .iter_mut()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("CBOR map has no entry {key:?}"))
}
