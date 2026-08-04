use ketchup_exact::{
    ExactBackend, RectangleExtrudeSpec, ReferenceResolution, capture_guaranteed_references,
    has_complete_manifold_adjacency, resolve_subshape_reference,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let output_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: ketchup-a0-reference-producer <new-output-path>");
    let backend = ExactBackend::new();
    let output = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .expect("preregistered producer fixture must succeed");
    assert!(
        has_complete_manifold_adjacency(&output.body.topology),
        "producer output lacks complete reciprocal adjacency"
    );
    let references = capture_guaranteed_references(&output, "a0-document", "extrusion-001")
        .expect("producer Guaranteed references must be capturable");

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .expect("producer evidence path must be new and writable");
    writeln!(file, "strengthened-a0-reference-fixture-v1").expect("fixture header must write");
    writeln!(file, "backend\t{}", output.backend_fingerprint).expect("backend must write");
    writeln!(
        file,
        "topology\t{}\t{}",
        output.body.topology.face_count, output.body.topology.edge_count
    )
    .expect("topology must write");

    for reference in references {
        let face_ordinal = match resolve_subshape_reference(&reference, &output) {
            ReferenceResolution::Resolved { face_ordinal, .. } => face_ordinal,
            other => panic!("producer reference did not resolve: {other:?}"),
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == face_ordinal)
            .expect("producer resolved face must exist");
        let boundary = face
            .edge_ordinals
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            file,
            "reference\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            reference.document_id,
            reference.producer_feature_id,
            reference.semantic_role,
            reference.source_element_id,
            reference.expected_type,
            reference.backend_fingerprint,
            reference.lineage_digest,
            reference.corroborating_geometry_fingerprint,
            face_ordinal,
            boundary
        )
        .expect("reference must write");
    }

    let retired_role = "legacy.extrusion.side(profile_edge=north)";
    let retired_source = "profile.edge.north";
    let retired_face = output
        .body
        .topology
        .faces
        .iter()
        .find(|face| (face.centroid_mm.y - 60.0).abs() <= 1.0e-9 && face.normal.y > 0.9)
        .expect("prior build must contain the declared legacy north face");
    assert_eq!(retired_face.edge_ordinals.len(), 4);
    let retired_boundary = retired_face
        .edge_ordinals
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let retired_lineage = stable_digest(&format!(
        "a0-document:extrusion-001:{retired_role}:{retired_source}:planar_face"
    ));
    writeln!(
        file,
        "retired_reference\ta0-document\textrusion-001\t{}\t{}\tplanar_face\t{}\t{}\t{}\t{}\t{}",
        retired_role,
        retired_source,
        output.backend_fingerprint,
        retired_lineage,
        retired_face.geometric_fingerprint,
        retired_face.ordinal,
        retired_boundary
    )
    .expect("retired reference must write");
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}
