use ketchup_exact::{
    ExactBackend, FaceEvidence, RectangleExtrudeSpec, ReferenceResolution, StabilityClass,
    SubshapeRef, capture_guaranteed_references, has_complete_manifold_adjacency,
    resolve_subshape_reference,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let mode = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .expect("usage: ketchup-a0-diagnostic-probe <produce|consume> <fixture-path>");
    let fixture_path = arguments
        .next()
        .expect("usage: ketchup-a0-diagnostic-probe <produce|consume> <fixture-path>");
    assert!(
        arguments.next().is_none(),
        "unexpected diagnostic probe argument"
    );

    match mode.as_str() {
        "produce" => produce(Path::new(&fixture_path)),
        "consume" => consume(Path::new(&fixture_path)),
        _ => panic!("unsupported diagnostic probe mode: {mode}"),
    }
}

fn produce(output_path: &Path) {
    println!("native_observation=entered");
    let backend = ExactBackend::new();
    let output = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .expect("diagnostic producer fixture must succeed");
    println!("mode=produce");
    println!("backend={}", output.backend_fingerprint);
    println!(
        "topology=faces:{},edges:{}",
        output.body.topology.face_count, output.body.topology.edge_count
    );
    assert!(
        has_complete_manifold_adjacency(&output.body.topology),
        "diagnostic producer output lacks complete reciprocal adjacency"
    );
    let references = capture_guaranteed_references(&output, "a0-document", "extrusion-001")
        .expect("diagnostic producer Guaranteed references must be capturable");

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .expect("diagnostic producer fixture path must be new and writable");
    writeln!(file, "a0-diagnostic-reference-fixture-v1").expect("fixture header must write");
    writeln!(file, "backend\t{}", output.backend_fingerprint).expect("backend must write");
    writeln!(
        file,
        "topology\t{}\t{}",
        output.body.topology.face_count, output.body.topology.edge_count
    )
    .expect("topology must write");

    for reference in references {
        let face_ordinal = match resolve_subshape_reference(&reference, &output) {
            ReferenceResolution::Resolved {
                face_ordinal,
                migrated_backend: false,
            } => face_ordinal,
            other => panic!("diagnostic producer reference did not resolve locally: {other:?}"),
        };
        let face = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal == face_ordinal)
            .expect("diagnostic producer resolved face must exist");
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
        .expect("diagnostic reference must write");
        println!("captured={}", reference.semantic_role);
    }

    let retired_role = "legacy.extrusion.side(profile_edge=north)";
    let retired_source = "profile.edge.north";
    let retired_face = output
        .body
        .topology
        .faces
        .iter()
        .find(|face| (face.centroid_mm.y - 60.0).abs() <= 1.0e-9)
        .expect("diagnostic producer must contain the north face");
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
    .expect("diagnostic retired reference must write");
    println!("diagnostic_result=PASS");
}

fn consume(fixture_path: &Path) {
    println!("native_observation=entered");
    let (producer_backend, references, retired_reference) = load_references(fixture_path);
    let backend = ExactBackend::new();
    let output = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .expect("diagnostic consumer fixture must succeed");
    assert!(
        has_complete_manifold_adjacency(&output.body.topology),
        "diagnostic consumer output lacks complete reciprocal adjacency"
    );
    let migrated = producer_backend != output.backend_fingerprint;
    println!("mode=consume");
    println!("producer_backend={producer_backend}");
    println!("consumer_backend={}", output.backend_fingerprint);
    println!("migrated_backend={migrated}");

    let mut active_failures = Vec::new();
    for reference in &references {
        match resolve_subshape_reference(reference, &output) {
            ReferenceResolution::Resolved {
                face_ordinal,
                migrated_backend,
            } => {
                let face = output
                    .body
                    .topology
                    .faces
                    .iter()
                    .find(|face| face.ordinal == face_ordinal)
                    .expect("diagnostic consumer resolved face must exist");
                if migrated_backend != migrated
                    || !expected_semantic_face(reference, face)
                    || face.edge_ordinals.len() != 4
                {
                    println!(
                        "active_reference_wrong={}@{}",
                        reference.semantic_role, face_ordinal
                    );
                    active_failures.push(format!(
                        "wrong or incomplete semantic face for {}",
                        reference.semantic_role
                    ));
                } else {
                    println!("resolved={}@{}", reference.semantic_role, face_ordinal);
                }
            }
            other => {
                println!(
                    "active_reference_failure={}: {other:?}",
                    reference.semantic_role
                );
                active_failures.push(format!(
                    "did not resolve {}: {other:?}",
                    reference.semantic_role
                ));
            }
        }
    }

    let retired = resolve_subshape_reference(&retired_reference, &output);
    if migrated {
        assert!(
            matches!(retired, ReferenceResolution::QuarantinedMigration { .. }),
            "cross-build retired reference must be quarantined, got {retired:?}"
        );
        println!("retired_reference=quarantined");
    } else {
        assert_eq!(
            retired,
            ReferenceResolution::Lost,
            "same-build retired reference must be lost without migration quarantine"
        );
        println!("retired_reference=lost");
    }
    assert!(
        active_failures.is_empty(),
        "diagnostic consumer active-reference failures: {}",
        active_failures.join("; ")
    );
    println!("diagnostic_result=PASS");
}

fn load_references(path: &Path) -> (String, Vec<SubshapeRef>, SubshapeRef) {
    let raw = fs::read_to_string(path).expect("diagnostic reference fixture must be readable");
    let mut lines = raw.lines();
    assert_eq!(
        lines.next(),
        Some("a0-diagnostic-reference-fixture-v1"),
        "unexpected diagnostic fixture schema"
    );
    let backend = lines
        .next()
        .and_then(|line| line.strip_prefix("backend\t"))
        .expect("diagnostic fixture must record its producer backend")
        .to_owned();
    assert!(
        !backend.is_empty(),
        "diagnostic producer backend must not be empty"
    );
    assert_eq!(
        lines.next(),
        Some("topology\t6\t12"),
        "diagnostic producer fixture must be a closed cuboid"
    );

    let mut references = Vec::new();
    let mut retired_reference = None;
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 11, "malformed diagnostic reference row");
        assert!(
            matches!(fields[0], "reference" | "retired_reference"),
            "unexpected diagnostic row kind"
        );
        assert_eq!(fields[1], "a0-document");
        assert_eq!(fields[2], "extrusion-001");
        assert_eq!(fields[5], "planar_face");
        assert_eq!(fields[6], backend, "diagnostic reference/backend mismatch");
        assert!(fields[7].starts_with("fnv1a64:"));
        assert!(fields[8].starts_with("fnv1a64:"));
        let face_ordinal = fields[9]
            .parse::<u32>()
            .expect("diagnostic producer face ordinal must be numeric");
        assert!(
            face_ordinal < 6,
            "diagnostic producer face ordinal outside fixture"
        );
        let boundary = fields[10]
            .split(',')
            .map(|value| value.parse::<u32>().expect("edge ordinal must be numeric"))
            .collect::<Vec<_>>();
        assert_eq!(
            boundary.len(),
            4,
            "diagnostic producer boundary must be complete"
        );
        let reference = SubshapeRef {
            document_id: fields[1].to_owned(),
            producer_feature_id: fields[2].to_owned(),
            semantic_role: fields[3].to_owned(),
            source_element_id: fields[4].to_owned(),
            expected_type: fields[5].to_owned(),
            stability_class: StabilityClass::Guaranteed,
            backend_fingerprint: fields[6].to_owned(),
            lineage_digest: fields[7].to_owned(),
            corroborating_geometry_fingerprint: fields[8].to_owned(),
        };
        if fields[0] == "reference" {
            references.push(reference);
        } else {
            assert!(
                retired_reference.replace(reference).is_none(),
                "diagnostic producer emitted multiple retired references"
            );
        }
    }
    let mut active_keys = references
        .iter()
        .map(|reference| {
            (
                reference.semantic_role.as_str(),
                reference.source_element_id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    active_keys.sort_unstable();
    assert_eq!(
        active_keys,
        vec![
            ("extrusion.bottom", "profile.face"),
            ("extrusion.side(profile_edge=east)", "profile.edge.east"),
            ("extrusion.top", "profile.face"),
        ],
        "diagnostic producer did not emit the complete active reference set"
    );
    (
        backend,
        references,
        retired_reference.expect("diagnostic producer must emit one retired reference"),
    )
}

fn expected_semantic_face(reference: &SubshapeRef, face: &FaceEvidence) -> bool {
    match reference.semantic_role.as_str() {
        "extrusion.top" => close(face.centroid_mm.z, 20.0),
        "extrusion.bottom" => close(face.centroid_mm.z, 0.0),
        "extrusion.side(profile_edge=east)" => close(face.centroid_mm.x, 100.0),
        _ => false,
    }
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1.0e-9
}

fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}
