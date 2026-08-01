use ketchup_exact::{
    BoxSpec, CutMode, ExactBackend, ExactOpOutput, FaceEvidence, GeneratedOperation, GeometryError,
    GeometryErrorCode, Point3, RectangleExtrudeSpec, ReferenceResolution, Size3,
    StructuredGenerator, SubshapeRef, capture_guaranteed_references, resolve_subshape_reference,
    validate_closed_planar_profile,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ACTIVE_LOCK_SHA256: &str = "0a47c3d0b6d6a24201f64d221b8850892926f7786a3c23ba117c13df881c3d58";
const SEEDS: [u64; 20] = [
    1, 7, 19, 31, 43, 61, 73, 101, 151, 211, 307, 401, 503, 601, 701, 809, 907, 1009, 1201, 1601,
];

#[derive(Default)]
struct Metrics {
    fixed_passes: usize,
    fixed_total: usize,
    ffi_fuzz_calls: usize,
    ffi_fuzz_passes: usize,
    expected_valid_passes: usize,
    expected_valid_total: usize,
    adversarial_structural_diagnoses: usize,
    adversarial_non_passes: usize,
    expected_rejection_passes: usize,
    expected_rejection_total: usize,
    silent_invalid_shapes: usize,
    guaranteed_correct: usize,
    guaranteed_total: usize,
    guaranteed_history_complete: usize,
    silent_wrong_identities: usize,
    step_passes: usize,
    step_total: usize,
    migration_resolved: usize,
    migration_quarantined: usize,
}

#[test]
fn gate_a0() {
    let repository = repository_root();
    validate_active_freeze(&repository);

    let backend = ExactBackend::new();
    let mut metrics = Metrics::default();
    run_fixed_corpus(&backend, &mut metrics);
    run_guaranteed_mutations_and_migration(&backend, &mut metrics);
    run_adversarial_corpus(&backend, &mut metrics);
    run_external_step_corpus(&backend, &repository, &mut metrics);
    run_structure_aware_fuzz(&backend, &mut metrics);

    let failures = gate_failures(&metrics);
    write_evidence(&repository, &metrics, &failures);
    assert!(
        failures.is_empty(),
        "A0 NO-GO under r0-v5: {}",
        failures.join("; ")
    );
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate must remain below crates/")
        .to_path_buf()
}

fn validate_active_freeze(repository: &Path) {
    let script = repository.join("scripts/windows/validate-r0-v5-preregistration.ps1");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .output()
        .expect("R0 v2 validator must run before A0 observations");
    assert!(
        output.status.success(),
        "R0 v2 integrity failed before A0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_fixed_corpus(backend: &ExactBackend, metrics: &mut Metrics) {
    let fixed = [
        fixed_extrude(backend, 100.0, 60.0, 20.0, 120_000.0),
        fixed_extrude(backend, 10.0, 20.0, 30.0, 6_000.0),
        fixed_cut(
            backend,
            Size3 {
                x: 40.0,
                y: 30.0,
                z: 10.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 10.0,
                    y: 10.0,
                    z: -5.0,
                },
                size_mm: Size3 {
                    x: 20.0,
                    y: 10.0,
                    z: 20.0,
                },
            },
            CutMode::ThroughAll,
            10_000.0,
        ),
        fixed_cut(
            backend,
            Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 30.0,
                    y: 20.0,
                    z: 10.0,
                },
                size_mm: Size3 {
                    x: 40.0,
                    y: 20.0,
                    z: 15.0,
                },
            },
            CutMode::BlindPlanar,
            112_000.0,
        ),
    ];
    metrics.fixed_total = fixed.len();
    metrics.fixed_passes = fixed.iter().filter(|passed| **passed).count();
}

fn fixed_extrude(backend: &ExactBackend, width: f64, depth: f64, height: f64, volume: f64) -> bool {
    backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: width,
            depth_mm: depth,
            height_mm: height,
        })
        .is_ok_and(|output| accepted(&output) && close(output.body.topology.volume_mm3, volume))
}

fn fixed_cut(
    backend: &ExactBackend,
    base_size: Size3,
    tool: BoxSpec,
    mode: CutMode,
    volume: f64,
) -> bool {
    let Ok(base) = backend.make_box(BoxSpec {
        origin_mm: Point3::ORIGIN,
        size_mm: base_size,
    }) else {
        return false;
    };
    backend
        .cut_box(&base.body, tool, mode)
        .is_ok_and(|output| accepted(&output) && close(output.body.topology.volume_mm3, volume))
}

fn run_guaranteed_mutations_and_migration(backend: &ExactBackend, metrics: &mut Metrics) {
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .expect("Guaranteed producer must succeed");
    let references = capture_guaranteed_references(&base, "a0-document", "extrusion-001")
        .expect("Guaranteed references must be capturable");
    let mutations = [
        (100.0, 60.0, 35.0),
        (100.0, 60.0, 5.0),
        (140.0, 60.0, 20.0),
        (40.0, 60.0, 20.0),
        (100.0, 90.0, 20.0),
        (100.0, 25.0, 20.0),
        (140.0, 90.0, 35.0),
        (40.0, 25.0, 5.0),
    ];

    for (width, depth, height) in mutations {
        let output = backend
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: width,
                depth_mm: depth,
                height_mm: height,
            })
            .expect("Frozen Guaranteed mutation must produce a valid extrusion");
        for reference in &references {
            metrics.guaranteed_total += 1;
            match resolve_subshape_reference(reference, &output) {
                ReferenceResolution::Resolved { face_ordinal, .. } => {
                    let face = output
                        .body
                        .topology
                        .faces
                        .iter()
                        .find(|face| face.ordinal == face_ordinal)
                        .expect("resolved face ordinal must exist");
                    if expected_semantic_face(reference, face, width, height) {
                        metrics.guaranteed_correct += 1;
                    } else {
                        metrics.silent_wrong_identities += 1;
                    }
                    if output.topology_history.iter().any(|entry| {
                        entry.semantic_role.as_deref() == Some(reference.semantic_role.as_str())
                            && entry.source_element_id == reference.source_element_id
                            && entry.output_face_ordinal == Some(face_ordinal)
                            && !entry.relation.is_empty()
                    }) {
                        metrics.guaranteed_history_complete += 1;
                    }
                }
                ReferenceResolution::Ambiguous { .. }
                | ReferenceResolution::Lost
                | ReferenceResolution::QuarantinedMigration { .. } => {}
            }
        }
    }

    let current = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 120.0,
            depth_mm: 70.0,
            height_mm: 25.0,
        })
        .expect("migration target must succeed");
    for reference in &references {
        let mut prior_backend_reference = reference.clone();
        prior_backend_reference.backend_fingerprint = "occt-prior-build:test-fixture".to_owned();
        if matches!(
            resolve_subshape_reference(&prior_backend_reference, &current),
            ReferenceResolution::Resolved {
                migrated_backend: true,
                ..
            }
        ) {
            metrics.migration_resolved += 1;
        }
    }
    let mut unresolved = references[0].clone();
    unresolved.backend_fingerprint = "occt-prior-build:test-fixture".to_owned();
    unresolved.semantic_role = "extrusion.removed-role".to_owned();
    unresolved.source_element_id = "profile.removed-element".to_owned();
    if matches!(
        resolve_subshape_reference(&unresolved, &current),
        ReferenceResolution::QuarantinedMigration { .. }
    ) {
        metrics.migration_quarantined += 1;
    }
}

fn expected_semantic_face(
    reference: &SubshapeRef,
    face: &FaceEvidence,
    width: f64,
    height: f64,
) -> bool {
    match reference.semantic_role.as_str() {
        "extrusion.top" => close(face.centroid_mm.z, height),
        "extrusion.bottom" => close(face.centroid_mm.z, 0.0),
        "extrusion.side(profile_edge=east)" => close(face.centroid_mm.x, width),
        _ => false,
    }
}

fn run_adversarial_corpus(backend: &ExactBackend, metrics: &mut Metrics) {
    let expected_valid = vec![
        backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 0.01,
            depth_mm: 0.01,
            height_mm: 0.01,
        }),
        backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100_000.0,
            depth_mm: 100_000.0,
            height_mm: 0.01,
        }),
        backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 0.01,
            depth_mm: 100_000.0,
            height_mm: 100.0,
        }),
        adversarial_cut(
            backend,
            Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 0.001,
                    y: 0.001,
                    z: -1.0,
                },
                size_mm: Size3 {
                    x: 99.998,
                    y: 59.998,
                    z: 22.0,
                },
            },
            CutMode::ThroughAll,
        ),
        adversarial_cut(
            backend,
            Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 20.0,
                    y: 10.0,
                    z: 0.001,
                },
                size_mm: Size3 {
                    x: 60.0,
                    y: 40.0,
                    z: 21.0,
                },
            },
            CutMode::BlindPlanar,
        ),
        adversarial_cut(
            backend,
            Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 30.0,
                    y: 20.0,
                    z: -1.0,
                },
                size_mm: Size3 {
                    x: 40.0,
                    y: 20.0,
                    z: 22.0,
                },
            },
            CutMode::ThroughAll,
        ),
        adversarial_cut(
            backend,
            Size3 {
                x: 100.001,
                y: 60.001,
                z: 20.001,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 100.001 / 3.0,
                    y: 60.001 / 3.0,
                    z: -1.0,
                },
                size_mm: Size3 {
                    x: 100.001 / 3.0,
                    y: 60.001 / 3.0,
                    z: 22.001,
                },
            },
            CutMode::ThroughAll,
        ),
        mutation_round_trip(backend),
        two_sequential_cuts(backend),
        adversarial_cut(
            backend,
            Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
            BoxSpec {
                origin_mm: Point3 {
                    x: 30.0,
                    y: 20.0,
                    z: -1000.0,
                },
                size_mm: Size3 {
                    x: 40.0,
                    y: 20.0,
                    z: 2020.0,
                },
            },
            CutMode::ThroughAll,
        ),
    ];
    metrics.expected_valid_total = expected_valid.len();
    for outcome in expected_valid {
        match outcome {
            Ok(output) if accepted(&output) => metrics.expected_valid_passes += 1,
            Ok(_) => metrics.silent_invalid_shapes += 1,
            Err(error) => {
                metrics.adversarial_non_passes += 1;
                if !error.code.as_str().is_empty() && !error.diagnostic.is_empty() {
                    metrics.adversarial_structural_diagnoses += 1;
                }
            }
        }
    }

    let base = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 10.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .expect("adversarial rejection base must succeed");
    let rejections = [
        error_code(backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 0.0,
            depth_mm: 10.0,
            height_mm: 10.0,
        })),
        error_code(backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 10.0,
            depth_mm: 10.0,
            height_mm: -1.0,
        })),
        validate_closed_planar_profile(&[
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 10.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 10.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        ])
        .expect_err("self-intersecting profile must be rejected")
        .code,
        error_code(backend.extrude_rectangle(RectangleExtrudeSpec {
            width_mm: f64::NAN,
            depth_mm: 10.0,
            height_mm: 10.0,
        })),
        error_code(backend.cut_box(
            &base.body,
            BoxSpec {
                origin_mm: Point3 {
                    x: 20.0,
                    y: 20.0,
                    z: 20.0,
                },
                size_mm: Size3 {
                    x: 2.0,
                    y: 2.0,
                    z: 2.0,
                },
            },
            CutMode::BlindPlanar,
        )),
        error_code(backend.cut_box(
            &base.body,
            BoxSpec {
                origin_mm: Point3 {
                    x: 10.0,
                    y: 2.0,
                    z: 2.0,
                },
                size_mm: Size3 {
                    x: 2.0,
                    y: 2.0,
                    z: 2.0,
                },
            },
            CutMode::BlindPlanar,
        )),
    ];
    let expected = [
        GeometryErrorCode::InvalidParameter,
        GeometryErrorCode::InvalidParameter,
        GeometryErrorCode::InvalidProfile,
        GeometryErrorCode::NonFiniteParameter,
        GeometryErrorCode::NoGeometricChange,
        GeometryErrorCode::DegenerateOperation,
    ];
    metrics.expected_rejection_total = expected.len();
    metrics.expected_rejection_passes = rejections
        .iter()
        .zip(expected)
        .filter(|(actual, expected)| **actual == *expected)
        .count();
}

fn adversarial_cut(
    backend: &ExactBackend,
    base_size: Size3,
    tool: BoxSpec,
    mode: CutMode,
) -> Result<ExactOpOutput, GeometryError> {
    let base = backend.make_box(BoxSpec {
        origin_mm: Point3::ORIGIN,
        size_mm: base_size,
    })?;
    backend.cut_box(&base.body, tool, mode)
}

fn mutation_round_trip(backend: &ExactBackend) -> Result<ExactOpOutput, GeometryError> {
    let sequence = [35.0, 5.0, 140.0, 40.0, 90.0, 25.0, 35.0, 20.0];
    let mut last = None;
    for (index, value) in sequence.into_iter().enumerate() {
        let spec = match index {
            0 | 1 | 6 | 7 => RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: value,
            },
            2 | 3 => RectangleExtrudeSpec {
                width_mm: value,
                depth_mm: 60.0,
                height_mm: 20.0,
            },
            _ => RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: value,
                height_mm: 20.0,
            },
        };
        last = Some(backend.extrude_rectangle(spec)?);
    }
    Ok(last.expect("mutation sequence is not empty"))
}

fn two_sequential_cuts(backend: &ExactBackend) -> Result<ExactOpOutput, GeometryError> {
    let base = backend.make_box(BoxSpec {
        origin_mm: Point3::ORIGIN,
        size_mm: Size3 {
            x: 100.0,
            y: 60.0,
            z: 20.0,
        },
    })?;
    let first = backend.cut_box(
        &base.body,
        BoxSpec {
            origin_mm: Point3 {
                x: 10.0,
                y: 10.0,
                z: -1.0,
            },
            size_mm: Size3 {
                x: 15.0,
                y: 15.0,
                z: 22.0,
            },
        },
        CutMode::ThroughAll,
    )?;
    backend.cut_box(
        &first.body,
        BoxSpec {
            origin_mm: Point3 {
                x: 70.0,
                y: 35.0,
                z: -1.0,
            },
            size_mm: Size3 {
                x: 15.0,
                y: 15.0,
                z: 22.0,
            },
        },
        CutMode::ThroughAll,
    )
}

fn error_code(result: Result<ExactOpOutput, GeometryError>) -> GeometryErrorCode {
    result.expect_err("fixture must be rejected").code
}

fn run_external_step_corpus(backend: &ExactBackend, repository: &Path, metrics: &mut Metrics) {
    let fixtures = [
        ("self-authored-box.step", 6_000.0),
        ("self-authored-through-cut.step", 10_000.0),
        ("self-authored-l-bracket.step", 6_000.0),
    ];
    metrics.step_total = fixtures.len();
    for (name, expected_volume) in fixtures {
        let path = repository.join("corpora/r0/step").join(name);
        let passed = path.to_str().is_some_and(|path| {
            backend.import_step(path).is_ok_and(|output| {
                accepted(&output)
                    && close(output.body.topology.volume_mm3, expected_volume)
                    && output.body.topology.solid_count == 1
            })
        });
        if passed {
            metrics.step_passes += 1;
        }
    }
}

fn run_structure_aware_fuzz(backend: &ExactBackend, metrics: &mut Metrics) {
    for _repeat in 0..10 {
        for seed in SEEDS {
            let generator = StructuredGenerator::new(seed);
            for case_index in 0..50 {
                let case = generator.case(case_index);
                let replayed =
                    StructuredGenerator::new(case.replay.seed).case(case.replay.case_index);
                assert_eq!(case, replayed, "generator replay changed");
                metrics.ffi_fuzz_calls += 1;
                match backend.execute_generated(&case) {
                    Ok(output) if accepted(&output) => metrics.ffi_fuzz_passes += 1,
                    Ok(_) => metrics.silent_invalid_shapes += 1,
                    Err(error) => panic!(
                        "supported generated case {} failed structurally: {} ({})",
                        case.replay,
                        error,
                        generated_kind(&case.operation)
                    ),
                }
            }
        }
    }
}

fn generated_kind(operation: &GeneratedOperation) -> &'static str {
    match operation {
        GeneratedOperation::Extrude(_) => "extrude",
        GeneratedOperation::Cut { .. } => "cut",
    }
}

fn accepted(output: &ExactOpOutput) -> bool {
    output.tolerance_report.shape_valid
        && output.tolerance_report.accepted_exact_solid
        && output.body.topology.solid_count == 1
        && output.body.topology.volume_mm3.is_finite()
        && output.body.topology.volume_mm3 > 0.0
        && !output.input_digest.is_empty()
        && !output.body.result_fingerprint.is_empty()
        && !output.backend_fingerprint.is_empty()
}

fn close(actual: f64, expected: f64) -> bool {
    let absolute = (actual - expected).abs();
    absolute <= 1.0e-6 || absolute <= expected.abs() * 1.0e-10
}

fn gate_failures(metrics: &Metrics) -> Vec<String> {
    let mut failures = Vec::new();
    if metrics.fixed_passes != metrics.fixed_total {
        failures.push(format!(
            "baseline expected-valid {}/{}",
            metrics.fixed_passes, metrics.fixed_total
        ));
    }
    if metrics.ffi_fuzz_calls < 10_000 || metrics.ffi_fuzz_passes != metrics.ffi_fuzz_calls {
        failures.push(format!(
            "structure-aware FFI fuzz {}/{}",
            metrics.ffi_fuzz_passes, metrics.ffi_fuzz_calls
        ));
    }
    if metrics.expected_valid_passes * 100 < metrics.expected_valid_total * 90 {
        failures.push(format!(
            "adversarial expected-valid {}/{}",
            metrics.expected_valid_passes, metrics.expected_valid_total
        ));
    }
    if metrics.adversarial_structural_diagnoses != metrics.adversarial_non_passes {
        failures.push("adversarial structural diagnosis below 100%".to_owned());
    }
    if metrics.expected_rejection_passes != metrics.expected_rejection_total {
        failures.push(format!(
            "expected rejections {}/{}",
            metrics.expected_rejection_passes, metrics.expected_rejection_total
        ));
    }
    if metrics.silent_invalid_shapes != 0 {
        failures.push(format!(
            "{} silently accepted invalid shapes",
            metrics.silent_invalid_shapes
        ));
    }
    if metrics.guaranteed_correct != metrics.guaranteed_total || metrics.guaranteed_total != 24 {
        failures.push(format!(
            "Guaranteed identity {}/{}",
            metrics.guaranteed_correct, metrics.guaranteed_total
        ));
    }
    if metrics.silent_wrong_identities != 0 {
        failures.push(format!(
            "{} silent wrong identities",
            metrics.silent_wrong_identities
        ));
    }
    if metrics.guaranteed_history_complete != metrics.guaranteed_total {
        failures.push(format!(
            "Guaranteed history {}/{}",
            metrics.guaranteed_history_complete, metrics.guaranteed_total
        ));
    }
    if metrics.step_passes != metrics.step_total {
        failures.push(format!(
            "external STEP {}/{}",
            metrics.step_passes, metrics.step_total
        ));
    }
    if metrics.migration_resolved != 3 || metrics.migration_quarantined != 1 {
        failures.push(format!(
            "migration resolved={}, quarantined={}",
            metrics.migration_resolved, metrics.migration_quarantined
        ));
    }
    failures
}

fn write_evidence(repository: &Path, metrics: &Metrics, failures: &[String]) {
    let artifact_dir = repository.join("artifacts/gate-a0");
    let Ok(run_id) = std::env::var("KETCHUP_A0_RUN_ID") else {
        return;
    };
    let run_dir = artifact_dir.join("runs").join(&run_id);
    assert!(
        !run_dir.exists(),
        "A0 run evidence already exists: {run_id}"
    );
    fs::create_dir_all(&run_dir).expect("A0 run artifact directory must be writable");
    let decision = if failures.is_empty() { "GO" } else { "NO-GO" };
    let raw = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"freeze_id\": \"r0-v5\",\n",
            "  \"lock_sha256\": \"{}\",\n",
            "  \"decision\": \"{}\",\n",
            "  \"fixed\": {{\"passed\": {}, \"total\": {}}},\n",
            "  \"ffi_fuzz\": {{\"passed\": {}, \"calls\": {}}},\n",
            "  \"adversarial_expected_valid\": {{\"passed\": {}, \"total\": {}}},\n",
            "  \"adversarial_structural_diagnoses\": {{\"diagnosed\": {}, \"non_passes\": {}}},\n",
            "  \"expected_rejections\": {{\"passed\": {}, \"total\": {}}},\n",
            "  \"silent_invalid_shapes\": {},\n",
            "  \"guaranteed_identity\": {{\"correct\": {}, \"total\": {}, \"history_complete\": {}}},\n",
            "  \"silent_wrong_identities\": {},\n",
            "  \"external_step\": {{\"passed\": {}, \"total\": {}}},\n",
            "  \"migration\": {{\"resolved\": {}, \"quarantined_unresolved\": {}}}\n",
            "}}\n"
        ),
        ACTIVE_LOCK_SHA256,
        decision,
        metrics.fixed_passes,
        metrics.fixed_total,
        metrics.ffi_fuzz_passes,
        metrics.ffi_fuzz_calls,
        metrics.expected_valid_passes,
        metrics.expected_valid_total,
        metrics.adversarial_structural_diagnoses,
        metrics.adversarial_non_passes,
        metrics.expected_rejection_passes,
        metrics.expected_rejection_total,
        metrics.silent_invalid_shapes,
        metrics.guaranteed_correct,
        metrics.guaranteed_total,
        metrics.guaranteed_history_complete,
        metrics.silent_wrong_identities,
        metrics.step_passes,
        metrics.step_total,
        metrics.migration_resolved,
        metrics.migration_quarantined,
    );
    fs::write(run_dir.join("metrics.json"), &raw)
        .expect("immutable A0 raw evidence must be written");
    fs::write(artifact_dir.join("metrics.json"), raw)
        .expect("current A0 raw evidence must be written");

    let failure_text = if failures.is_empty() {
        "None.".to_owned()
    } else {
        failures
            .iter()
            .map(|failure| format!("- {failure}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let report = format!(
        "# Gate A0 Report\n\n- Run: `{run_id}`\n- Freeze: `r0-v5`\n- Lock SHA-256: `{ACTIVE_LOCK_SHA256}`\n- **Decision: {decision}**\n\n## Results\n\n| Contract | Result |\n|---|---:|\n| Fixed baseline valid | {}/{} |\n| Structure-aware FFI fuzz | {}/{} |\n| Adversarial expected-valid | {}/{} |\n| Adversarial non-pass structural diagnosis | {}/{} |\n| Expected typed rejections | {}/{} |\n| Silent invalid shapes | {} |\n| Guaranteed correct identity | {}/{} |\n| Guaranteed history evidence | {}/{} |\n| Silent wrong identity | {} |\n| External STEP fixtures | {}/{} |\n| Prior-backend references migrated | {} |\n| Unresolved migration references quarantined | {} |\n\n## Failure consequences\n\n{failure_text}\n\nThe report was produced by `cargo test -p ketchup-exact --test gate_a0` against the active immutable lock. Geometry fingerprints were used only as corroborating evidence; Guaranteed identity was resolved from producer role, source element lineage, and backend history.\n",
        metrics.fixed_passes,
        metrics.fixed_total,
        metrics.ffi_fuzz_passes,
        metrics.ffi_fuzz_calls,
        metrics.expected_valid_passes,
        metrics.expected_valid_total,
        metrics.adversarial_structural_diagnoses,
        metrics.adversarial_non_passes,
        metrics.expected_rejection_passes,
        metrics.expected_rejection_total,
        metrics.silent_invalid_shapes,
        metrics.guaranteed_correct,
        metrics.guaranteed_total,
        metrics.guaranteed_history_complete,
        metrics.guaranteed_total,
        metrics.silent_wrong_identities,
        metrics.step_passes,
        metrics.step_total,
        metrics.migration_resolved,
        metrics.migration_quarantined,
    );
    fs::write(run_dir.join("report.md"), &report).expect("immutable A0 report must be written");
    fs::write(artifact_dir.join("report.md"), report).expect("current A0 report must be written");
}
