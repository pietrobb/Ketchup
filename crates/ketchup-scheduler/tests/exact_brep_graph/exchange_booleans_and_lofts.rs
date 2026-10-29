use super::*;

#[test]
fn exact_worker_converts_verified_step_to_iges_and_reinspects_exact_evidence() {
    let _turn = crate::integration_support::file_turn();
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let iges = directory.path().join("box.iges");
    let cancelled = AtomicBool::new(false);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    supervisor
        .convert_step_to_iges_with_cancellation(&source, &iges, 1, &cancelled)
        .unwrap();
    let bytes = std::fs::read(&iges).unwrap();
    let evidence = supervisor
        .inspect_iges_import_with_cancellation(&iges, &sha256_hex(&bytes), &cancelled)
        .unwrap();

    assert_eq!(
        evidence.source_sha256,
        ketchup_model::graph::sha256_bytes(&bytes)
    );
    assert_eq!(evidence.source_byte_len, bytes.len() as u64);
    assert_eq!(
        evidence.source_unit,
        ketchup_model::import::ImportLengthUnit::Millimetre
    );
    assert_eq!(evidence.solid_count, 1);
    assert!((evidence.volume_mm3 - 6_000.0).abs() <= 1.0e-7);
    assert_eq!(evidence.topology_counts, [8, 12, 6, 1, 1]);
    let mesh_path = directory.path().join("box.mesh");
    let mesh = supervisor
        .tessellate_iges_import_with_cancellation(
            &iges,
            &sha256_hex(&bytes),
            &evidence.result_fingerprint,
            &mesh_path,
            &cancelled,
        )
        .unwrap();
    assert!(!mesh.vertices_mm.is_empty());
    assert!(!mesh.triangles.is_empty());
    assert!(mesh_path.is_file());

    std::fs::write(&iges, b"not an IGES model").unwrap();
    let malformed = std::fs::read(&iges).unwrap();
    assert!(
        supervisor
            .inspect_iges_import_with_cancellation(&iges, &sha256_hex(&malformed), &cancelled,)
            .is_err()
    );
}

#[test]
fn exact_graph_volume_mesh_is_identity_bound_bounded_and_cancellable() {
    let _turn = crate::integration_support::file_turn();
    let graph = simple_extrusion_graph();
    let options = ExactVolumeMeshWireOptions {
        surface_deflection_mm: 0.1,
        angular_deflection_rad: 0.2,
        max_tetrahedra: 64,
        max_relative_volume_error: 1.0e-10,
        min_tetrahedron_quality: 1.0e-5,
    };
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor
        .evaluate_exact_brep_graph_volume_mesh(&graph, options)
        .unwrap();
    let repeated = supervisor
        .evaluate_exact_brep_graph_volume_mesh(&graph, options)
        .unwrap();

    assert_eq!(package.mesh.graph_digest, graph.graph_digest);
    assert_eq!(
        package.mesh.source_result_fingerprint,
        package.source.identity.result_fingerprint
    );
    assert_eq!(package.mesh.tetrahedra.len(), 12);
    assert_eq!(package.mesh.boundary_triangles.len(), 12);
    assert_eq!(package.mesh.vertices_mm.len(), 9);
    assert!((package.mesh.exact_volume_mm3 - 240.0).abs() <= 1.0e-9);
    assert!(package.mesh.relative_volume_error <= options.max_relative_volume_error);
    assert!(package.mesh.minimum_signed_volume_mm3 > 0.0);
    assert!(package.mesh.minimum_quality >= options.min_tetrahedron_quality);
    assert_eq!(
        package.mesh.mesh_fingerprint,
        repeated.mesh.mesh_fingerprint
    );

    let mut too_small = options;
    too_small.max_tetrahedra = 4;
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph_volume_mesh(&graph, too_small)
            .unwrap_err(),
        "invalid_shape",
    );

    let cancelled = AtomicBool::new(true);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph_volume_mesh_with_cancellation(&graph, options, &cancelled,)
            .unwrap_err(),
        WorkerError::Cancelled
    );
}

#[test]
fn exact_volume_mesh_builds_occurrence_bound_fea_and_rejects_stale_or_unknown_faces() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, extrusion, occurrence) = simple_extrusion_document();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, extrusion).unwrap();
    let options = ExactVolumeMeshWireOptions {
        surface_deflection_mm: 0.1,
        angular_deflection_rad: 0.2,
        max_tetrahedra: 64,
        max_relative_volume_error: 1.0e-10,
        min_tetrahedron_quality: 1.0e-5,
    };
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor
        .evaluate_exact_brep_graph_volume_mesh(&graph, options)
        .unwrap();
    assert!(package.is_current(&snapshot));
    let bottom = package
        .source
        .face_evidence
        .iter()
        .min_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap()
        .face_ordinal;
    let top = package
        .source
        .face_evidence
        .iter()
        .max_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap()
        .face_ordinal;
    assert_ne!(bottom, top);
    let setup = ExactFeaSetup {
        case_id: "occurrence-bound-box-pressure".into(),
        instance_path: InstancePath::root(occurrence),
        material: FeaMaterial {
            id: 1,
            youngs_modulus_mpa: 200_000.0,
            poisson_ratio: 0.3,
            yield_strength_mpa: Some(250.0),
        },
        constrained_face_ordinals: vec![bottom],
        face_tractions: vec![ExactFeaFaceTraction {
            face_ordinal: top,
            traction_local_n_per_mm2: [0.0, 0.0, 1.0],
        }],
    };
    let bound = package.occurrence_bound_model(&snapshot, &setup).unwrap();
    assert_eq!(bound.instance_path, setup.instance_path);
    assert_eq!(bound.source_revision, snapshot.revision_id());
    assert_eq!(bound.source_digest, snapshot.canonical_digest());
    assert_eq!(bound.graph_digest, graph.graph_digest);
    assert_eq!(bound.mesh_fingerprint, package.mesh.mesh_fingerprint);
    assert_eq!(bound.model.nodes.len(), 9);
    assert_eq!(bound.model.elements.len(), 12);
    assert!(!bound.model.constraints.is_empty());
    assert!(!bound.model.loads.is_empty());
    let solution = bound.model.solve(FeaSolveSettings::default()).unwrap();
    assert!(solution.maximum_displacement_mm > 0.0);
    assert!(solution.maximum_free_dof_residual_n <= 1.0e-8);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() <= 1.0e-8)
    );

    let mut unknown_face = setup.clone();
    unknown_face.face_tractions[0].face_ordinal = u32::MAX;
    assert_eq!(
        package
            .occurrence_bound_model(&snapshot, &unknown_face)
            .unwrap_err(),
        ExactFeaSetupError::InvalidBoundarySelection
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(91),
                name: "Unrelated later edit".into(),
            },
        ]))
        .unwrap();
    let stale_snapshot = document.current();
    assert!(!package.is_current(&stale_snapshot));
    assert_eq!(
        package
            .occurrence_bound_model(&stale_snapshot, &setup)
            .unwrap_err(),
        ExactFeaSetupError::StaleGeometry
    );
}

#[test]
fn cam_simulation_removes_stock_exactly_and_reports_fixture_gouge_and_stale_refusals() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, plan) = cam_simulation_document();
    let face = [CamOperation::Face {
        id: 1,
        minimum_mm: [1.0, 1.0],
        maximum_mm: [19.0, 9.0],
        target_z_mm: 5.0,
    }];
    let toolpath = CamToolpath::plan(&document.current(), &plan, &face).unwrap();
    let cancelled = AtomicBool::new(false);
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let clean = worker
        .simulate_cam(&document.current(), &plan, &toolpath, &[], &cancelled)
        .unwrap();
    assert_eq!(clean.motion_count, toolpath.motions.len());
    assert!(clean.cutting_motion_count > 0);
    assert!((clean.stock_before_mm3 - 1_400.0).abs() <= 1.0e-6);
    assert!((clean.stock_after_mm3 - 1_000.0).abs() <= 1.0e-5);
    assert!((clean.removed_stock_mm3 - 400.0).abs() <= 1.0e-5);
    assert!(clean.residual_stock_mm3 <= 1.0e-5, "{clean:?}");
    assert!(clean.gouge_mm3 <= 1.0e-5, "{clean:?}");
    assert!(clean.collisions.is_empty(), "{clean:?}");
    let iso = CamPostprocessorOutput::generate(
        &document.current(),
        &plan,
        &toolpath,
        &clean,
        CamPostprocessorDialect::IsoMetricGCode,
    )
    .unwrap();
    let neutral = CamPostprocessorOutput::generate(
        &document.current(),
        &plan,
        &toolpath,
        &clean,
        CamPostprocessorDialect::ControllerNeutralJson,
    )
    .unwrap();
    assert_eq!(iso.parse().unwrap(), neutral.parse().unwrap());
    iso.verify(&document.current(), &plan, &toolpath, &clean)
        .unwrap();
    neutral
        .verify(&document.current(), &plan, &toolpath, &clean)
        .unwrap();

    let fixtures = [
        CamFixture {
            id: 1,
            minimum_mm: [-0.5, -0.5, 9.0],
            maximum_mm: [0.5, 0.5, 12.0],
        },
        CamFixture {
            id: 2,
            minimum_mm: [-0.5, -0.5, 19.0],
            maximum_mm: [0.5, 0.5, 22.0],
        },
    ];
    let collision = worker
        .simulate_cam(&document.current(), &plan, &toolpath, &fixtures, &cancelled)
        .unwrap();
    assert!(collision.collisions.iter().any(|entry| {
        entry.motion_kind == CamMotionKind::Rapid
            && entry.participant == CamCollisionParticipant::Cutter
            && entry.target == CamCollisionTarget::Fixture(1)
    }));
    assert!(collision.collisions.iter().any(|entry| {
        entry.motion_kind == CamMotionKind::Plunge
            && entry.participant == CamCollisionParticipant::Cutter
            && entry.target == CamCollisionTarget::Fixture(1)
    }));
    assert!(collision.collisions.iter().any(|entry| {
        entry.motion_kind == CamMotionKind::Rapid
            && entry.participant == CamCollisionParticipant::Holder
            && entry.target == CamCollisionTarget::Fixture(2)
    }));
    assert_eq!(
        CamPostprocessorOutput::generate(
            &document.current(),
            &plan,
            &toolpath,
            &collision,
            CamPostprocessorDialect::IsoMetricGCode,
        ),
        Err(CamPostprocessorError::UnsafeSimulation)
    );

    let gouging_operations = [
        CamOperation::Pocket {
            id: 2,
            minimum_mm: [5.0, 3.0],
            maximum_mm: [15.0, 7.0],
            top_z_mm: 5.0,
            bottom_z_mm: 3.0,
        },
        CamOperation::Contour {
            id: 3,
            center_path: CamPath2d {
                start_mm: [12.0, 5.0],
                segments: vec![
                    CamPathSegment2d::Arc {
                        to_mm: [8.0, 5.0],
                        center_mm: [10.0, 5.0],
                        clockwise: false,
                    },
                    CamPathSegment2d::Arc {
                        to_mm: [12.0, 5.0],
                        center_mm: [10.0, 5.0],
                        clockwise: false,
                    },
                ],
            },
            top_z_mm: 5.0,
            bottom_z_mm: 3.0,
            applied_radial_allowance_mm: 0.0,
        },
    ];
    let gouging_path = CamToolpath::plan(&document.current(), &plan, &gouging_operations).unwrap();
    let gouging = worker
        .simulate_cam(&document.current(), &plan, &gouging_path, &[], &cancelled)
        .unwrap();
    assert!(gouging.gouge_mm3 > 0.0);
    assert_eq!(
        CamPostprocessorOutput::generate(
            &document.current(),
            &plan,
            &gouging_path,
            &gouging,
            CamPostprocessorDialect::ControllerNeutralJson,
        ),
        Err(CamPostprocessorError::UnsafeSimulation)
    );

    let mut tampered = toolpath.clone();
    tampered.toolpath_digest.replace_range(..1, "0");
    assert!(
        worker
            .simulate_cam(&document.current(), &plan, &tampered, &[], &cancelled)
            .is_err()
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(951),
                dimension: dimension(6.0),
            },
        ]))
        .unwrap();
    assert!(
        worker
            .simulate_cam(&document.current(), &plan, &toolpath, &[], &cancelled)
            .is_err()
    );
}

#[test]
fn exact_pair_batch_reuses_graphs_and_handles_contact_containment_transforms_and_failure() {
    let _turn = crate::integration_support::file_turn();
    use ketchup_scheduler::pair_query::EXACT_PAIR_IDENTITY;
    use ketchup_scheduler::{ExactPairCandidate, ExactPairRelation};
    let graph = simple_extrusion_graph();
    let candidate = |dx: f64, dy: f64, dz: f64, scale: f64| {
        let mut matrix = EXACT_PAIR_IDENTITY;
        matrix[0] = scale;
        matrix[5] = scale;
        matrix[10] = scale;
        matrix[3] = dx;
        matrix[7] = dy;
        matrix[11] = dz;
        ExactPairCandidate {
            left_graph: 0,
            right_graph: 1,
            left_transform: EXACT_PAIR_IDENTITY,
            right_transform: matrix,
        }
    };
    let candidates = vec![
        candidate(20.0, 0.0, 0.0, 1.0),
        candidate(4.0, 0.0, 0.0, 1.0),
        candidate(1.0, 1.0, 1.0, 0.25),
        candidate(8.0, 0.0, 0.0, 1.0),
        candidate(8.0, 6.0, 0.0, 1.0),
        candidate(8.0, 6.0, 5.0, 1.0),
    ];
    let graphs = vec![graph.clone(), graph]; // Deduplicated; worker rejects duplicate loads.
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let results = worker
        .query_exact_brep_pairs(&graphs, &candidates, &BTreeMap::new(), 1e-7)
        .unwrap();
    let measured = results
        .iter()
        .map(|r| r.as_ref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        measured.iter().map(|r| r.relation).collect::<Vec<_>>(),
        vec![
            ExactPairRelation::Separated,
            ExactPairRelation::Penetrating,
            ExactPairRelation::Penetrating,
            ExactPairRelation::Touching,
            ExactPairRelation::Touching,
            ExactPairRelation::Touching
        ]
    );
    assert!((measured[0].distance_mm - 12.0).abs() < 1e-7);
    assert!((measured[1].common_volume_mm3 - 120.0).abs() < 1e-7);
    assert!((measured[2].common_volume_mm3 - 3.75).abs() < 1e-7);
    assert_eq!(
        results,
        worker
            .query_exact_brep_pairs(&graphs, &candidates, &BTreeMap::new(), 1e-7)
            .unwrap()
    );
    let mut rotated = candidate(6.0, 0.0, 0.0, 1.0);
    rotated.right_transform[0] = 0.0;
    rotated.right_transform[1] = -1.0;
    rotated.right_transform[4] = 1.0;
    rotated.right_transform[5] = 0.0;
    let result = worker
        .query_exact_brep_pairs(&graphs, &[rotated], &BTreeMap::new(), 1e-7)
        .unwrap();
    assert!((result[0].as_ref().unwrap().common_volume_mm3 - 180.0).abs() < 1e-7);
    // Parallel rotated boxes have overlapping world AABBs but a 1 mm gap.
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let rotation = [
        c, -c, 0.0, 0.0, c, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut disjoint = candidate(0.0, 0.0, 0.0, 1.0);
    disjoint.left_transform = rotation;
    disjoint.right_transform = rotation;
    disjoint.right_transform[3] = -7.0 * c;
    disjoint.right_transform[7] = 7.0 * c;
    let result = worker
        .query_exact_brep_pairs(&graphs, &[disjoint], &BTreeMap::new(), 1e-7)
        .unwrap();
    let disjoint = result[0].as_ref().unwrap();
    assert_eq!(disjoint.relation, ExactPairRelation::Separated);
    assert!((disjoint.distance_mm - 1.0).abs() < 1e-7);
    let cancelled = AtomicBool::new(true);
    assert!(matches!(
        worker.query_exact_brep_pairs_with_cancellation(
            &graphs,
            &candidates,
            &BTreeMap::new(),
            1e-7,
            &cancelled
        ),
        Err(WorkerError::Cancelled)
    ));
    assert_eq!(
        results,
        worker
            .query_exact_brep_pairs(&graphs, &candidates, &BTreeMap::new(), 1e-7)
            .unwrap()
    ); // Pre-cancel leaves worker usable.
    // OCCT cannot place a body scaled to nothing. That pair alone is unchecked,
    // with OCCT's reason; the pairs after it in the batch keep their real answers.
    let mut mixed = candidates.clone();
    mixed.insert(2, candidate(0.0, 0.0, 0.0, 0.0));
    let mut answers = worker
        .query_exact_brep_pairs(&graphs, &mixed, &BTreeMap::new(), 1e-7)
        .unwrap();
    let failed = answers.remove(2);
    assert!(
        matches!(&failed, Err(WorkerError::Geometry(cause)) if cause.contains("transform_body")),
        "{failed:?}"
    );
    assert_eq!(answers, results);
    assert_eq!(
        results,
        worker
            .query_exact_brep_pairs(&graphs, &candidates, &BTreeMap::new(), 1e-7)
            .unwrap()
    );
    let oversized =
        vec![candidates[0].clone(); ketchup_scheduler::pair_query::MAX_EXACT_PAIR_CANDIDATES + 1];
    assert!(
        worker
            .query_exact_brep_pairs(&graphs, &oversized, &BTreeMap::new(), 1e-7)
            .is_err()
    );
    assert_eq!(
        results,
        worker
            .query_exact_brep_pairs(&graphs, &candidates, &BTreeMap::new(), 1e-7)
            .unwrap()
    );
}

#[test]
fn worker_evaluates_v9_curved_sweep_with_deterministic_topology_and_mesh() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(89);
    let profile = FeatureId(890);
    let path = FeatureId(891);
    let sweep = FeatureId(892);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "V9 curved sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Section".into(),
                kind: FeatureKind::polygon(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-arc path".into(),
                kind: FeatureKind::Profile {
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
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Curved sweep".into(),
                kind: FeatureKind::sweep(profile, path),
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, sweep).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V9);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert_eq!(package.graph.as_ref(), &graph);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);
    assert_eq!(package.topology_counts[4], 1);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    assert_eq!(
        package.triangles.len(),
        package.triangle_face_ordinals.len()
    );
    assert!(
        package
            .triangle_face_ordinals
            .iter()
            .all(|ordinal| *ordinal < package.topology_counts[2])
    );
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("v9-curved-sweep.step");
    supervisor
        .export_exact_brep_graph_step(&document.current(), &package, &step_path)
        .unwrap();
    let source = std::fs::read(&step_path).unwrap();
    let evidence = supervisor
        .inspect_step_import_with_cancellation(
            &step_path,
            &sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(evidence.solid_count, 1);
    assert!((evidence.volume_mm3 - package.volume_mm3).abs() <= package.volume_mm3 * 1.0e-9);
}

#[test]
fn worker_evaluates_v10_multisegment_sweep_with_step_round_trip() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(90);
    let profile = FeatureId(900);
    let path = FeatureId(901);
    let sweep = FeatureId(902);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "V10 multisegment sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Section".into(),
                kind: FeatureKind::polygon(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-arc-line path".into(),
                kind: FeatureKind::Profile {
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
                id: sweep,
                definition_id: definition,
                name: "Multisegment sweep".into(),
                kind: FeatureKind::sweep(profile, path),
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, sweep).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V10);
    let mut downgraded = graph.clone();
    downgraded.schema = EXACT_BREP_GRAPH_SCHEMA_V9.into();
    assert_eq!(
        downgraded.to_bytes(),
        Err(ExactBRepGraphError::InvalidGraph)
    );

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert_eq!(package.graph.as_ref(), &graph);
    assert_eq!(package.topology_counts[4], 1);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    let expected_volume = 8.0 * (75.0 + 25.0 * std::f64::consts::FRAC_PI_2);
    assert!((package.volume_mm3 - expected_volume).abs() <= expected_volume * 1.0e-9);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("v10-multisegment-sweep.step");
    supervisor
        .export_exact_brep_graph_step(&document.current(), &package, &step_path)
        .unwrap();
    let source = std::fs::read(&step_path).unwrap();
    let evidence = supervisor
        .inspect_step_import_with_cancellation(
            &step_path,
            &sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(evidence.solid_count, 1);
    assert!((evidence.volume_mm3 - package.volume_mm3).abs() <= package.volume_mm3 * 1.0e-9);
}

#[test]
fn worker_binds_multiple_imported_sources_by_digest_for_boolean_and_mesh() {
    let _turn = crate::integration_support::file_turn();
    let (document, definition, base, tool, _) = generated_boolean_document([1.0, 1.0, 1.0], 0);
    let snapshot = document.current();
    let directory = tempfile::tempdir().unwrap();
    let cancelled = AtomicBool::new(false);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut sources = Vec::new();
    let mut evidences = Vec::new();
    let mut source_volumes = Vec::new();
    for producer in [base, tool] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        let path = directory.path().join(format!("source-{}.step", producer.0));
        supervisor
            .export_exact_brep_graph_step(&snapshot, &package, &path)
            .unwrap();
        let source = std::fs::read(&path).unwrap();
        let source_sha256 = sha256_hex(&source);
        let evidence = supervisor
            .inspect_step_import_with_cancellation(&path, &source_sha256, &cancelled)
            .unwrap();
        source_volumes.push(evidence.volume_mm3);
        sources.push(source);
        evidences.push(evidence);
    }

    let mut imported_document = DocumentStore::new();
    imported_document
        .apply_batch(
            &plan_step_import(
                &imported_document.current(),
                &sources[0],
                "target.step",
                &evidences[0],
            )
            .unwrap(),
        )
        .unwrap();
    let target_definition = imported_document
        .current()
        .definitions()
        .next()
        .unwrap()
        .id();
    let target = imported_document.current().features().next().unwrap().id();
    let target_occurrence = imported_document
        .current()
        .occurrences()
        .next()
        .unwrap()
        .id();
    let second_import = plan_step_import(
        &imported_document.current(),
        &sources[1],
        "tool.step",
        &evidences[1],
    )
    .unwrap();
    imported_document.apply_batch(&second_import).unwrap();
    let tool = imported_document
        .current()
        .features()
        .find(|feature| feature.id() != target)
        .unwrap()
        .id();
    let tool_occurrence = imported_document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.id() != target_occurrence)
        .unwrap()
        .id();
    let mut container_data = persistence::ContainerData::default();
    for source in &sources {
        container_data.insert_import_blob(source.clone()).unwrap();
    }
    let imported_baseline =
        persistence::save_container(&imported_document.current(), &container_data).unwrap();
    let result_definition = DefinitionId(3);
    let result_features = [
        FeatureId(3),
        FeatureId(4),
        FeatureId(5),
        FeatureId(6),
        FeatureId(7),
    ];
    let angle = 30.0_f64.to_radians();
    let tool_transform = Transform::from_matrix([
        angle.cos(),
        -angle.sin(),
        0.0,
        4.0,
        angle.sin(),
        angle.cos(),
        0.0,
        -3.0,
        0.0,
        0.0,
        1.0,
        2.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    let before_solid_tool = imported_document.current().canonical_digest();
    imported_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: tool_occurrence,
                transform: tool_transform,
            },
            CanonicalCommand::ApplySolidTool(SolidToolPlan {
                operation: BooleanOperation::Union,
                target_occurrence_id: target_occurrence,
                target_feature_id: target,
                tool_occurrence_id: tool_occurrence,
                tool_feature_id: tool,
                result_definition_id: result_definition,
                result_feature_ids: result_features.to_vec(),
                result_definition_name: "Imported source union".into(),
                result_feature_name: "Rigid imported union".into(),
                keep_tool: true,
            }),
        ]))
        .unwrap();
    let imported_snapshot = imported_document.current();
    assert_eq!(
        imported_snapshot
            .occurrence(target_occurrence)
            .unwrap()
            .definition_id(),
        result_definition
    );
    let producer = result_features[4];
    let graph =
        ExactBRepGraph::from_snapshot(&imported_snapshot, result_definition, producer).unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ketchup_model::exact_brep_graph::ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if *matrix_bits == tool_transform.matrix().map(f64::to_bits)
    )));
    assert_ne!(target_definition, result_definition);
    let pair_sources = sources
        .iter()
        .map(|source| (sha256_hex(source), Arc::new(source.clone())))
        .collect::<BTreeMap<_, _>>();
    let pair = ketchup_scheduler::ExactPairCandidate {
        left_graph: 0,
        right_graph: 0,
        left_transform: ketchup_scheduler::pair_query::EXACT_PAIR_IDENTITY,
        right_transform: ketchup_scheduler::pair_query::EXACT_PAIR_IDENTITY,
    };
    let pair_result = supervisor
        .query_exact_brep_pairs(
            std::slice::from_ref(&graph),
            std::slice::from_ref(&pair),
            &pair_sources,
            1e-7,
        )
        .unwrap();
    assert_eq!(
        pair_result[0].as_ref().unwrap().relation,
        ketchup_scheduler::ExactPairRelation::Penetrating
    );
    let mut corrupt_sources = pair_sources.clone();
    Arc::make_mut(corrupt_sources.values_mut().next().unwrap())[0] ^= 1;
    assert!(
        supervisor
            .query_exact_brep_pairs(
                std::slice::from_ref(&graph),
                std::slice::from_ref(&pair),
                &corrupt_sources,
                1e-7
            )
            .is_err()
    );
    assert!(
        supervisor
            .query_exact_brep_pairs(
                std::slice::from_ref(&graph),
                &[pair],
                &BTreeMap::new(),
                1e-7
            )
            .is_err()
    );
    let reversed_sources = [sources[1].as_slice(), sources[0].as_slice()];
    let package = supervisor
        .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
        .unwrap();
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
            .unwrap(),
        package
    );
    assert_eq!(package.graph.as_ref(), &graph);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    assert!(package.volume_mm3 >= source_volumes[0].max(source_volumes[1]));
    assert!(package.volume_mm3 <= source_volumes.iter().sum::<f64>());
    assert!(package.is_current(&imported_snapshot));

    let mut imported_operation_packages = Vec::new();
    for operation in [BooleanOperation::Cut, BooleanOperation::Intersect] {
        let mut operation_document = persistence::load(&imported_baseline)
            .unwrap()
            .into_editable()
            .ok()
            .unwrap();
        operation_document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: tool_occurrence,
                    transform: tool_transform,
                },
                CanonicalCommand::ApplySolidTool(SolidToolPlan {
                    operation,
                    target_occurrence_id: target_occurrence,
                    target_feature_id: target,
                    tool_occurrence_id: tool_occurrence,
                    tool_feature_id: tool,
                    result_definition_id: result_definition,
                    result_feature_ids: result_features.to_vec(),
                    result_definition_name: format!("Imported source {operation:?}"),
                    result_feature_name: format!("Rigid imported {operation:?}"),
                    keep_tool: true,
                }),
            ]))
            .unwrap();
        let operation_snapshot = operation_document.current();
        let operation_graph = ExactBRepGraph::from_snapshot(
            &operation_snapshot,
            result_definition,
            result_features[4],
        )
        .unwrap();
        assert!(operation_graph.nodes.iter().any(|node| matches!(
            node.operation,
            ketchup_model::exact_brep_graph::ExactBRepOperation::Boolean {
                operation: graph_operation,
                ..
            } if graph_operation == operation.into()
        )));
        let operation_package = supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&operation_graph, &reversed_sources)
            .unwrap();
        assert_eq!(
            supervisor
                .evaluate_exact_brep_graph_with_imported_sources(
                    &operation_graph,
                    &reversed_sources,
                )
                .unwrap(),
            operation_package
        );
        assert!(operation_package.is_current(&operation_snapshot));
        imported_operation_packages.push(operation_package);
    }
    let cut_volume = imported_operation_packages[0].volume_mm3;
    let intersection_volume = imported_operation_packages[1].volume_mm3;
    assert!(cut_volume > 0.0);
    assert!(intersection_volume > 0.0);
    assert!(
        (cut_volume + intersection_volume - source_volumes[0]).abs() <= source_volumes[0] * 1.0e-8
    );
    let exact_body = ExactBodyPackage::Graph(package.clone());
    let result_key = exact_body.result_key();
    let registry = ExactResultRegistry::accept(&imported_snapshot, [Arc::new(exact_body)]).unwrap();
    assert!(registry.get_result(&result_key).is_some());
    let reopened = persistence::load(
        &persistence::save_container(&imported_snapshot, &container_data).unwrap(),
    )
    .unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(
        reopened_snapshot.canonical_digest(),
        imported_snapshot.canonical_digest()
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened_snapshot, result_definition, producer).unwrap(),
        graph
    );
    let graph_step = directory.path().join("imported-union.step");
    supervisor
        .export_exact_brep_graph_step_with_imported_sources(
            &imported_snapshot,
            &package,
            &graph_step,
            &reversed_sources,
        )
        .unwrap();
    assert!(
        std::fs::read(&graph_step)
            .unwrap()
            .windows(9)
            .any(|window| window == b"ISO-10303")
    );
    let source_blobs = BTreeMap::from([
        (sha256_hex(&sources[0]), Arc::new(sources[0].clone())),
        (sha256_hex(&sources[1]), Arc::new(sources[1].clone())),
    ]);
    let model_step = directory.path().join("imported-union-model.step");
    supervisor
        .export_current_model_step_with_imported_sources(
            &imported_snapshot,
            &[(
                ExactBodyPackage::Graph(package.clone()),
                Transform::identity(),
            )],
            &model_step,
            &source_blobs,
        )
        .unwrap();
    assert!(
        std::fs::read(&model_step)
            .unwrap()
            .windows(9)
            .any(|window| window == b"ISO-10303")
    );
    assert!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &[sources[0].as_slice()])
            .is_err()
    );
    assert!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(
                &graph,
                &[sources[0].as_slice(), sources[0].as_slice()],
            )
            .is_err()
    );
    let malformed_source = b"not a STEP payload";
    let malformed_document = DocumentStore::new();
    let malformed_before = malformed_document.current().canonical_digest();
    assert_eq!(
        plan_step_import(
            &malformed_document.current(),
            malformed_source,
            "malformed.step",
            &evidences[0],
        ),
        Err(ketchup_model::import::StepImportPlanError::InvalidWorkerEvidence)
    );
    assert_eq!(
        malformed_document.current().canonical_digest(),
        malformed_before
    );
    assert_eq!(malformed_document.visible_undo_steps(), 0);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
            .unwrap(),
        package
    );
    let undo_snapshot = imported_document.undo().unwrap();
    assert_eq!(undo_snapshot.canonical_digest(), before_solid_tool);
    assert_eq!(
        undo_snapshot
            .occurrence(target_occurrence)
            .unwrap()
            .definition_id(),
        target_definition
    );
    let redo_snapshot = imported_document.redo().unwrap();
    assert_eq!(
        redo_snapshot.canonical_digest(),
        imported_snapshot.canonical_digest()
    );
    assert!(package.is_current(&redo_snapshot));
}

#[test]
fn generated_boolean_graph_properties_cover_all_operations_and_rigid_variants() {
    let _turn = crate::integration_support::file_turn();
    let samples = generated_boolean_scales();
    assert_eq!(samples, generated_boolean_scales());
    assert_eq!(samples.len(), 6);
    for (index, sample) in samples.iter().enumerate() {
        assert!(
            sample
                .iter()
                .all(|value| value.is_finite() && *value >= 0.5 && *value <= 3.0)
        );
        assert!(samples.iter().skip(index + 1).all(|candidate| {
            candidate
                .iter()
                .zip(sample)
                .any(|(left, right)| left.to_bits() != right.to_bits())
        }));
    }

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for (sample_index, scales) in samples.into_iter().enumerate() {
        let mut variant_volumes = [[0.0; 4]; 3];
        for (rigid_variant, volumes) in variant_volumes.iter_mut().enumerate() {
            let (document, definition, base, tool, operations) =
                generated_boolean_document(scales, rigid_variant);
            let snapshot = document.current();
            let before_revision = snapshot.revision_id();
            let before_digest = snapshot.canonical_digest();
            let before_undo = document.visible_undo_steps();
            let base_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, base).unwrap();
            let tool_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap();
            let base_result = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
            let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
            let mut packages = Vec::new();

            for (operation_index, (producer, operation)) in operations.into_iter().enumerate() {
                let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
                let package = supervisor
                    .evaluate_exact_brep_graph(&graph)
                    .unwrap_or_else(|error| {
                        panic!(
                            "sample {sample_index}, rigid variant {rigid_variant}, {operation:?}: {error:?}"
                        )
                    });
                assert_eq!(
                    supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
                    package,
                    "sample {sample_index}, rigid variant {rigid_variant}, {operation:?} is not deterministic"
                );
                assert_eq!(package.graph.as_ref(), &graph);
                assert_eq!(
                    package.identity.canonical_input_digest,
                    graph.canonical_input_digest
                );
                assert_eq!(package.identity.producer_feature_id.0, producer.0);
                assert!(package.is_current(&snapshot));
                volumes[operation_index] = package.volume_mm3;
                packages.push(package);
            }

            let tolerance = base_result.volume_mm3.max(tool_result.volume_mm3) * 1.0e-9;
            assert!(
                (volumes[0] + volumes[2] - base_result.volume_mm3).abs() <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: cut + intersection must equal target"
            );
            assert!(
                (volumes[1] + volumes[2] - base_result.volume_mm3 - tool_result.volume_mm3).abs()
                    <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: union + intersection must equal target + tool"
            );
            assert!(
                (volumes[3] - base_result.volume_mm3).abs() <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: split must preserve target volume"
            );
            assert!(packages[3].topology_counts[4] >= 2);

            let registry = ExactResultRegistry::accept(
                &snapshot,
                packages
                    .iter()
                    .cloned()
                    .map(ExactBodyPackage::Graph)
                    .map(Arc::new),
            )
            .unwrap();
            for package in &packages {
                assert!(
                    registry
                        .get_result(&ExactBodyPackage::Graph(package.clone()).result_key())
                        .is_some()
                );
            }
            assert_eq!(document.current().revision_id(), before_revision);
            assert_eq!(document.current().canonical_digest(), before_digest);
            assert_eq!(document.visible_undo_steps(), before_undo);

            let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
            let reopened_snapshot = reopened.snapshot();
            assert_eq!(reopened_snapshot.canonical_digest(), before_digest);
            for ((producer, _), package) in operations.into_iter().zip(packages) {
                assert_eq!(
                    &ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, producer)
                        .unwrap(),
                    package.graph.as_ref()
                );
                assert!(package.is_current(&reopened_snapshot));
            }
        }

        for operation_index in 0..4 {
            let base = variant_volumes[0][operation_index];
            for (rigid_variant, transformed) in variant_volumes.iter().enumerate().skip(1) {
                assert!(
                    (transformed[operation_index] - base).abs() <= base.max(1.0) * 1.0e-9,
                    "sample {sample_index}, operation {operation_index}: rigid variant {rigid_variant} changed volume"
                );
            }
        }
    }
}

#[test]
fn generated_boolean_graph_property_verifier_confirms_round_trip_and_scaling() {
    let _turn = crate::integration_support::file_turn();
    let samples = generated_boolean_scales();
    assert_eq!(samples, generated_boolean_scales());
    assert_eq!(samples.len(), 6);
    assert_eq!(samples[0], [0.5, 0.75, 0.6]);
    assert_eq!(samples[2], [3.0, 2.5, 1.75]);
    for (index, [scale_x, scale_y, scale_z]) in samples.iter().copied().enumerate() {
        assert!((0.5..=3.0).contains(&scale_x));
        assert!((0.5..=2.5).contains(&scale_y));
        assert!((0.5..=1.75).contains(&scale_z));
        assert!(samples.iter().skip(index + 1).all(|candidate| {
            candidate
                .iter()
                .zip([scale_x, scale_y, scale_z])
                .any(|(left, right)| left.to_bits() != right.to_bits())
        }));
    }

    let verification_samples = [samples[0], samples[2], samples[5]];
    let mut normalized_reference: Option<[f64; 4]> = None;
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for (sample_index, scales) in verification_samples.into_iter().enumerate() {
        let mut rigid_reference: Option<[f64; 4]> = None;
        for rigid_variant in 0..3 {
            let (document, definition, base, tool, operations) =
                generated_boolean_document(scales, rigid_variant);
            let snapshot = document.current();
            let before_revision = snapshot.revision_id();
            let before_digest = snapshot.canonical_digest();
            let before_undo = document.visible_undo_steps();
            let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
            let reopened_snapshot = reopened.snapshot();
            assert_eq!(reopened_snapshot.canonical_digest(), before_digest);

            let base_result = supervisor
                .evaluate_exact_brep_graph(
                    &ExactBRepGraph::from_snapshot(&snapshot, definition, base).unwrap(),
                )
                .unwrap();
            let tool_result = supervisor
                .evaluate_exact_brep_graph(
                    &ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap(),
                )
                .unwrap();
            let mut volumes = [0.0; 4];
            for (operation_index, (producer, operation)) in operations.into_iter().enumerate() {
                let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
                let reopened_graph =
                    ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, producer)
                        .unwrap();
                assert_eq!(reopened_graph, graph);
                let package = supervisor
                    .evaluate_exact_brep_graph(&graph)
                    .unwrap_or_else(|error| {
                        panic!(
                            "verifier sample {sample_index}, rigid variant {rigid_variant}, {operation:?}: {error:?}"
                        )
                    });
                assert_eq!(
                    supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
                    package
                );
                assert_eq!(
                    supervisor
                        .evaluate_exact_brep_graph(&reopened_graph)
                        .unwrap(),
                    package
                );
                assert!(package.is_current(&snapshot));
                assert!(package.is_current(&reopened_snapshot));
                volumes[operation_index] = package.volume_mm3;
            }

            let tolerance = base_result.volume_mm3.max(tool_result.volume_mm3) * 1.0e-9;
            assert!((volumes[0] + volumes[2] - base_result.volume_mm3).abs() <= tolerance);
            assert!(
                (volumes[1] + volumes[2] - base_result.volume_mm3 - tool_result.volume_mm3).abs()
                    <= tolerance
            );
            assert!((volumes[3] - base_result.volume_mm3).abs() <= tolerance);
            if let Some(reference) = rigid_reference {
                for (actual, expected) in volumes.into_iter().zip(reference) {
                    assert!((actual - expected).abs() <= expected.max(1.0) * 1.0e-9);
                }
            } else {
                rigid_reference = Some(volumes);
            }

            assert_eq!(document.current().revision_id(), before_revision);
            assert_eq!(document.current().canonical_digest(), before_digest);
            assert_eq!(document.visible_undo_steps(), before_undo);
        }

        let scale_product = scales.into_iter().product::<f64>();
        let normalized = rigid_reference
            .unwrap()
            .map(|volume| volume / scale_product);
        if let Some(reference) = normalized_reference {
            for (actual, expected) in normalized.into_iter().zip(reference) {
                assert!((actual - expected).abs() <= expected.max(1.0) * 1.0e-9);
            }
        } else {
            normalized_reference = Some(normalized);
        }
    }
}

#[test]
fn generated_boolean_graph_preserves_legacy_export_and_stale_contracts() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, base, _, operations) =
        generated_boolean_document([1.0, 1.0, 1.0], 0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: definition,
                name: "Generated Boolean occurrence".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let split = operations[3].0;
    let snapshot = document.current();
    let split_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, split).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let split_package = supervisor.evaluate_exact_brep_graph(&split_graph).unwrap();
    let split_body = ExactBodyPackage::Graph(split_package.clone());
    let result_key = split_body.result_key();
    let registry = ExactResultRegistry::accept(&snapshot, [Arc::new(split_body)]).unwrap();
    assert!(registry.get_result(&result_key).is_some());

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert!(split_package.is_current(&reopened_snapshot));
    assert_eq!(
        &ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, split).unwrap(),
        split_package.graph.as_ref()
    );
    ExactResultRegistry::accept(
        &reopened_snapshot,
        [Arc::new(ExactBodyPackage::Graph(split_package.clone()))],
    )
    .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("generated-boolean-graph.step");
    supervisor
        .export_exact_brep_graph_step(&snapshot, &split_package, &step_path)
        .unwrap();
    let step = std::fs::read(&step_path).unwrap();
    assert!(step.len() > 256);
    assert!(step.windows(9).any(|window| window == b"ISO-10303"));
    let model_step_path = directory.path().join("generated-boolean-model.step");
    let verified_model_step = supervisor
        .export_current_model_step(
            &snapshot,
            &[(
                ExactBodyPackage::Graph(split_package.clone()),
                Transform::identity(),
            )],
            &model_step_path,
        )
        .unwrap();
    let model_step = std::fs::read(&model_step_path).unwrap();
    assert_eq!(verified_model_step, model_step);
    assert!(model_step.len() > 256);
    assert!(model_step.windows(9).any(|window| window == b"ISO-10303"));
    std::fs::write(&model_step_path, b"replaced after verification").unwrap();
    assert!(
        verified_model_step
            .windows(9)
            .any(|window| window == b"ISO-10303")
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: base,
                dimension: dimension(17.0),
            },
        ]))
        .unwrap();
    let edited_snapshot = document.current();
    assert!(!split_package.is_current(&edited_snapshot));
    assert!(matches!(
        ExactResultRegistry::accept(
            &edited_snapshot,
            [Arc::new(ExactBodyPackage::Graph(split_package.clone()))]
        ),
        Err(ExactProductError::StaleResult)
    ));
    assert!(
        supervisor
            .export_exact_brep_graph_step(&edited_snapshot, &split_package, &step_path)
            .is_err()
    );
    let edited_graph = ExactBRepGraph::from_snapshot(&edited_snapshot, definition, split).unwrap();
    assert_ne!(edited_graph.graph_digest, split_package.graph.graph_digest);
    let edited_package = supervisor.evaluate_exact_brep_graph(&edited_graph).unwrap();
    ExactResultRegistry::accept(
        &edited_snapshot,
        [Arc::new(ExactBodyPackage::Graph(edited_package.clone()))],
    )
    .unwrap();

    let undo_snapshot = document.undo().unwrap();
    assert_eq!(
        undo_snapshot.canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(split_package.is_current(&undo_snapshot));
    assert!(!edited_package.is_current(&undo_snapshot));
    let redo_snapshot = document.redo().unwrap();
    assert_eq!(
        redo_snapshot.canonical_digest(),
        edited_snapshot.canonical_digest()
    );
    assert!(edited_package.is_current(&redo_snapshot));
    assert!(!split_package.is_current(&redo_snapshot));
}

#[test]
fn worker_transforms_a_circle_pad_from_its_arbitrary_workplane_frame() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(2);
    let plane = FeatureId(100);
    let sketch_id = FeatureId(101);
    let pad = FeatureId(102);
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [10.0, 20.0],
            radius_mm: 5.0,
        }],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Oriented Pad graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
                    operation: PadOperation::NewBody,
                }),
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, pad).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let mesh = StepImportMesh {
        vertices_mm: result
            .vertices
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect(),
        triangles: result
            .triangles
            .iter()
            .zip(result.triangle_face_ordinals.iter())
            .map(|(triangle, face_ordinal)| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: *face_ordinal,
            })
            .collect(),
    };
    for invalid_area in [1.0, f64::NAN] {
        assert!(matches!(
            ExactBRepGraphPackage::from_worker_evidence(
                &graph,
                ExactBRepGraphWorkerEvidence {
                    mass: None,
                    exact_input_digest: result.identity.exact_input_digest.clone(),
                    result_fingerprint: result.identity.result_fingerprint.clone(),
                    volume_mm3: result.volume_mm3,
                    area_mm2: invalid_area,
                    topology_counts: result.topology_counts,
                    wire_count: None,
                    bounds_mm: result.bounds_mm,
                    backend: result.identity.backend.clone(),
                    tolerance: result.identity.tolerance.clone(),
                    faces: Vec::new(),
                    edges: Vec::new(),
                },
                &mesh,
            ),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
    }

    assert_bounds_close(result.bounds_mm, [0.0, 5.0, 15.0, 25.0, 15.0, 25.0]);
    assert_eq!(result.identity.producer_feature_id.0, pad.0);
    assert_eq!(result.topology_counts[4], 1);
    assert_eq!(
        result.topological_references.len(),
        result.topology_counts[..3]
            .iter()
            .map(|count| *count as usize)
            .sum::<usize>()
    );
    for (kind, count) in [
        (TopologicalElementKind::Vertex, result.topology_counts[0]),
        (TopologicalElementKind::Edge, result.topology_counts[1]),
        (TopologicalElementKind::Face, result.topology_counts[2]),
    ] {
        assert_eq!(
            result
                .topological_references
                .iter()
                .filter(|reference| reference.kind == kind)
                .count(),
            count as usize
        );
    }
    assert!(result.topological_references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.producer_feature_id == pad
            && reference.stability == TopologicalReferenceStability::Ephemeral
    }));
}

#[test]
fn worker_evaluates_a_bounded_profile_pocket_as_a_body_cut() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(3);
    let base_profile = FeatureId(200);
    let base = FeatureId(201);
    let pocket_profile = FeatureId(202);
    let pocket = FeatureId(203);
    let through_cut = FeatureId(204);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Pocket graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Base boundary".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::extrusion(base_profile, dimension(10.0)),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_profile,
                definition_id: definition,
                name: "Triangular pocket".into(),
                kind: FeatureKind::polygon(&[[6.0, 5.0], [24.0, 7.0], [12.0, 16.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: definition,
                name: "Pocket".into(),
                kind: FeatureKind::pocket(base, pocket_profile, dimension(4.0)),
            },
            CanonicalCommand::CreateFeature {
                id: through_cut,
                definition_id: definition,
                name: "Through cut".into(),
                kind: FeatureKind::through_cut(base, pocket_profile),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pocket).unwrap();
    let through_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, through_cut).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let through_result = supervisor
        .evaluate_exact_brep_graph(&through_graph)
        .unwrap();

    assert!(result.volume_mm3 > 0.0 && result.volume_mm3 < 6_000.0);
    assert_bounds_close(result.bounds_mm, [0.0, 0.0, 0.0, 30.0, 20.0, 10.0]);
    assert_eq!(result.identity.producer_feature_id.0, pocket.0);
    assert!(through_result.volume_mm3 > 0.0 && through_result.volume_mm3 < result.volume_mm3);
    assert_bounds_close(through_result.bounds_mm, [0.0, 0.0, 0.0, 30.0, 20.0, 10.0]);
    assert_ne!(
        through_result.identity.result_fingerprint,
        result.identity.result_fingerprint
    );
}

#[test]
fn worker_booleans_an_unequal_body_transformed_by_its_workplane_frame() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(4);
    let base_profile = FeatureId(300);
    let base = FeatureId(301);
    let plane = FeatureId(302);
    let sketch_id = FeatureId(303);
    let tool = FeatureId(304);
    let intersection = FeatureId(305);
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 5.0],
            radius_mm: 4.0,
        }],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Transformed Boolean graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Rectangular base".into(),
                kind: FeatureKind::polygon(&[
                    [0.0, -10.0],
                    [30.0, -10.0],
                    [30.0, 10.0],
                    [0.0, 10.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::extrusion(base_profile, dimension(10.0)),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Transverse circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: tool,
                definition_id: definition,
                name: "Transverse tool".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
                    operation: PadOperation::NewBody,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: intersection,
                definition_id: definition,
                name: "Transformed intersection".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Intersect,
                    target: base,
                    tool,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let tool_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap();
    let intersection_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, intersection).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
    let intersection_result = supervisor
        .evaluate_exact_brep_graph(&intersection_graph)
        .unwrap();

    assert_eq!(intersection_result.volume_mm3, tool_result.volume_mm3);
    assert_bounds_close(
        intersection_result.bounds_mm,
        [0.0, -4.0, 1.0, 25.0, 4.0, 9.0],
    );
    assert_eq!(
        intersection_result.identity.producer_feature_id.0,
        intersection.0
    );
}

#[test]
fn disjoint_body_booleans_return_exact_results_or_typed_refusals_atomically() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(5);
    let base_profile = FeatureId(400);
    let base = FeatureId(401);
    let tool_profile = FeatureId(402);
    let tool = FeatureId(403);
    let cut = FeatureId(404);
    let union = FeatureId(405);
    let intersect = FeatureId(406);
    let split = FeatureId(407);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Disjoint Boolean graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Base profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::extrusion(base_profile, dimension(5.0)),
            },
            CanonicalCommand::CreateFeature {
                id: tool_profile,
                definition_id: definition,
                name: "Remote tool profile".into(),
                kind: FeatureKind::polygon(&[[30.0, 0.0], [35.0, 0.0], [35.0, 5.0], [30.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: tool,
                definition_id: definition,
                name: "Remote tool".into(),
                kind: FeatureKind::extrusion(tool_profile, dimension(7.0)),
            },
        ]))
        .unwrap();
    for (id, operation) in [
        (cut, BooleanOperation::Cut),
        (union, BooleanOperation::Union),
        (intersect, BooleanOperation::Intersect),
        (split, BooleanOperation::Split),
    ] {
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id: definition,
                name: format!("{operation:?}"),
                kind: FeatureKind::Boolean {
                    operation,
                    target: base,
                    tool,
                },
            }]))
            .unwrap();
    }
    let snapshot = document.current();
    let graph = |producer| ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
    let base_graph = graph(base);
    let tool_graph = graph(tool);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_result = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
    let cut_result = supervisor.evaluate_exact_brep_graph(&graph(cut)).unwrap();
    let union_result = supervisor.evaluate_exact_brep_graph(&graph(union)).unwrap();

    assert!((cut_result.volume_mm3 - base_result.volume_mm3).abs() <= 1.0e-6);
    assert!(
        (union_result.volume_mm3 - base_result.volume_mm3 - tool_result.volume_mm3).abs() <= 1.0e-6
    );
    assert_eq!(union_result.topology_counts[4], 2);

    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&graph(intersect))
            .unwrap_err(),
        "invalid_shape",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_result
    );
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&graph(split))
            .unwrap_err(),
        "no_geometric_change",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_result
    );
}

#[test]
fn oversized_or_unsupported_graph_inputs_fail_closed() {
    let _turn = crate::integration_support::file_turn();
    let valid_graph = simple_extrusion_graph();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let valid_result = supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap();

    let mut oversized = valid_graph.clone();
    let profile = oversized.profiles[0].clone();
    oversized
        .profiles
        .resize(MAX_EXACT_BREP_GRAPH_PROFILES + 1, profile);
    match supervisor
        .evaluate_exact_brep_graph(&oversized)
        .unwrap_err()
    {
        WorkerError::Protocol(detail) => {
            assert!(detail.contains("resource limit") || detail.contains("structure is invalid"));
        }
        other => panic!("expected bounded graph refusal, got {other:?}"),
    }
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap(),
        valid_result
    );

    let definition = DefinitionId(91);
    let profile = FeatureId(910);
    let path = FeatureId(911);
    let sweep = FeatureId(912);
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Unsupported operation graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Spline sweep profile".into(),
                kind: FeatureKind::closed_spline(&[
                    [-3.0, -2.0],
                    [4.0, -2.0],
                    [4.0, 3.0],
                    [-3.0, 3.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Straight path".into(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [20.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Unsupported spline sweep".into(),
                kind: FeatureKind::sweep(profile, path),
            },
        ]))
        .err()
        .expect("spline Sweep must fail before exact worker evaluation");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap(),
        valid_result
    );
}

#[test]
fn worker_evaluates_circle_sketch_revolve_in_its_workplane_frame() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(60);
    let workplane = FeatureId(600);
    let sketch = FeatureId(601);
    let revolve = FeatureId(602);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Framed sketch revolve".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: definition,
                name: "Circle sketch".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [10.0, 0.0],
                        radius_mm: 2.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Framed revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch,
                    axis_start_mm: [0.0, 0.0],
                    axis_end_mm: [0.0, 1.0],
                    angle_degrees: 360.0,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, revolve).unwrap();
    assert_eq!(graph.profiles[0].source_feature_id, sketch.0);
    assert!(graph.profiles[0].region_id.is_some());

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, revolve.0);
    assert!(package.volume_mm3 > 0.0);
    assert_bounds_close(package.bounds_mm, [-12.0, -12.0, -2.0, 12.0, 12.0, 2.0]);
}

#[test]
fn worker_evaluates_compound_mixed_sketch_revolve_with_stable_persistence_and_undo() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(61);
    let workplane = FeatureId(610);
    let sketch_id = FeatureId(611);
    let revolve = FeatureId(612);
    let sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [10.0, -10.0],
                end_mm: [30.0, -10.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [30.0, -10.0],
                end_mm: [30.0, 10.0],
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [30.0, 10.0],
                control_1_mm: [26.0, 14.0],
                control_2_mm: [22.0, 14.0],
                end_mm: [18.0, 10.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(4),
                start_mm: [18.0, 10.0],
                end_mm: [10.0, 2.0],
                center_mm: [18.0, 2.0],
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(5),
                start_mm: [10.0, 2.0],
                end_mm: [10.0, -10.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(6),
                center_mm: [20.0, 0.0],
                radius_mm: 2.0,
            },
        ],
        constraints: Vec::new(),
    };
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].holes.len(), 1);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Compound mixed revolve".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Mixed outer loop and circle hole".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Bounded compound revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch_id,
                    axis_start_mm: [0.0, -20.0],
                    axis_end_mm: [0.0, 20.0],
                    angle_degrees: 270.0,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, revolve).unwrap();
    let ExactBRepPlanarGeometry::Region { outer, holes } = &graph.profiles[0].geometry else {
        panic!("compound sketch must compile to a planar region");
    };
    let ExactBRepPlanarLoop::Boundary { segments } = outer else {
        panic!("mixed outer loop must remain a boundary");
    };
    assert_eq!(segments.len(), 5);
    assert!(
        segments
            .iter()
            .any(|segment| matches!(segment, ExactBRepPlanarSegment::Line { .. }))
    );
    assert!(
        segments
            .iter()
            .any(|segment| matches!(segment, ExactBRepPlanarSegment::CircularArc { .. }))
    );
    assert!(
        segments
            .iter()
            .any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
    );
    assert_eq!(holes.len(), 1);
    assert!(matches!(holes[0], ExactBRepPlanarLoop::Circle { .. }));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, revolve.0);
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3 > 0.0);

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, revolve).unwrap();
    assert_eq!(reopened_graph, graph);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap(),
        package
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: revolve },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Bounded compound revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch_id,
                    axis_start_mm: [0.0, -20.0],
                    axis_end_mm: [0.0, 20.0],
                    angle_degrees: 180.0,
                },
            },
        ]))
        .unwrap();
    let changed = ExactBRepGraph::from_snapshot(&document.current(), definition, revolve).unwrap();
    assert_ne!(changed.graph_digest, graph.graph_digest);
    assert!(!package.is_current(&document.current()));
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.undo().unwrap(), definition, revolve).unwrap(),
        graph
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.redo().unwrap(), definition, revolve).unwrap(),
        changed
    );
}

#[test]
fn worker_evaluates_revolve_non_rectangular_sweep_and_loft_through_one_graph_identity() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(6);
    let revolve_profile = FeatureId(500);
    let revolve = FeatureId(501);
    let sweep_profile = FeatureId(502);
    let sweep_path = FeatureId(503);
    let sweep = FeatureId(504);
    let loft_lower = FeatureId(505);
    let loft_upper = FeatureId(506);
    let loft = FeatureId(507);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Unified exact graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: revolve_profile,
                definition_id: definition,
                name: "Asymmetric revolve boundary".into(),
                kind: FeatureKind::polygon(&[[2.0, -4.0], [7.0, -3.0], [5.0, 6.0], [2.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Partial revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: revolve_profile,
                    axis_start_mm: [0.0, -10.0],
                    axis_end_mm: [0.0, 10.0],
                    angle_degrees: 270.0,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_profile,
                definition_id: definition,
                name: "Curved sweep boundary".into(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [-2.0, -3.0],
                            end_mm: [2.0, -3.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [2.0, -3.0],
                            end_mm: [2.0, 3.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [2.0, 3.0],
                            end_mm: [-2.0, 3.0],
                            center_mm: [0.0, 3.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [-2.0, 3.0],
                            end_mm: [-2.0, -3.0],
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_path,
                definition_id: definition,
                name: "Oblique sweep path".into(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [10.0, -5.0],
                        end_mm: [24.0, 17.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Non-rectangular sweep".into(),
                kind: FeatureKind::sweep(sweep_profile, sweep_path),
            },
            CanonicalCommand::CreateFeature {
                id: loft_lower,
                definition_id: definition,
                name: "Lower spline".into(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -5.0],
                    [9.0, -4.0],
                    [8.0, 6.0],
                    [-7.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: loft_upper,
                definition_id: definition,
                name: "Upper spline".into(),
                kind: FeatureKind::closed_spline(&[
                    [-4.0, -3.0],
                    [6.0, -2.0],
                    [5.0, 4.0],
                    [-3.0, 3.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: definition,
                name: "Bounded loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: loft_lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: loft_upper,
                            elevation_mm: 18.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    for producer in [revolve, sweep, loft] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let first = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        let repeated = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(first, repeated);
        assert_eq!(
            first.identity.canonical_input_digest,
            graph.canonical_input_digest
        );
        assert_eq!(first.graph.graph_digest, graph.graph_digest);
        assert_eq!(first.identity.producer_feature_id.0, producer.0);
        assert!(!first.identity.result_fingerprint.is_empty());
        assert!(first.volume_mm3 > 0.0);
        assert_eq!(first.topology_counts[4], 1);
    }
}

#[test]
fn worker_evaluates_mixed_planar_profile_loft_as_one_exact_solid() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(97);
    let plane = FeatureId(969);
    let lower = FeatureId(970);
    let middle = FeatureId(971);
    let upper = FeatureId(972);
    let loft = FeatureId(973);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Planar Loft definition".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: definition,
                name: "Arc section".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: plane,
                    entities: vec![
                        SketchEntity::Arc {
                            id: SketchEntityId(1),
                            start_mm: [12.0, 0.0],
                            end_mm: [-12.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        SketchEntity::Arc {
                            id: SketchEntityId(2),
                            start_mm: [-12.0, 0.0],
                            end_mm: [12.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: middle,
                definition_id: definition,
                name: "Line section".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: plane,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-14.0, -8.0],
                            end_mm: [14.0, -8.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [14.0, -8.0],
                            end_mm: [14.0, 8.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(3),
                            start_mm: [14.0, 8.0],
                            end_mm: [-14.0, 8.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-14.0, 8.0],
                            end_mm: [-14.0, -8.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Cubic section".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: plane,
                    entities: vec![
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(1),
                            start_mm: [9.0, 0.0],
                            control_1_mm: [9.0, 3.313_708],
                            control_2_mm: [4.970_563, 6.0],
                            end_mm: [0.0, 6.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(2),
                            start_mm: [0.0, 6.0],
                            control_1_mm: [-4.970_563, 6.0],
                            control_2_mm: [-9.0, 3.313_708],
                            end_mm: [-9.0, 0.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [-9.0, 0.0],
                            control_1_mm: [-9.0, -3.313_708],
                            control_2_mm: [-4.970_563, -6.0],
                            end_mm: [0.0, -6.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(4),
                            start_mm: [0.0, -6.0],
                            control_1_mm: [4.970_563, -6.0],
                            control_2_mm: [9.0, -3.313_708],
                            end_mm: [9.0, 0.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: definition,
                name: "Mixed planar profile Loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: middle,
                            elevation_mm: 28.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 55.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, loft).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();

    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);
    assert!(package.bounds_mm[0][2].abs() <= 1.0e-6);
    assert!((package.bounds_mm[1][2] - 55.0).abs() <= 1.0e-6);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
}

#[test]
fn worker_evaluates_loft_with_matching_profile_holes_and_rejects_mismatched_wires() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(2021);
    let plane = FeatureId(20_210);
    let profiles = [FeatureId(20_211), FeatureId(20_212), FeatureId(20_213)];
    let loft = FeatureId(20_214);
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: "Holed Loft definition".into(),
        },
        CanonicalCommand::CreateFeature {
            id: plane,
            definition_id: definition,
            name: "Profile plane".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
        },
    ];
    for (index, profile) in profiles.into_iter().enumerate() {
        commands.push(CanonicalCommand::CreateFeature {
            id: profile,
            definition_id: definition,
            name: format!("Annular section {index}"),
            kind: FeatureKind::Sketch(SketchSpec {
                workplane: plane,
                entities: vec![
                    SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [0.0, 0.0],
                        radius_mm: 4.0 - index as f64 * 0.5,
                    },
                    SketchEntity::Circle {
                        id: SketchEntityId(2),
                        center_mm: [0.0, 0.0],
                        radius_mm: 2.0 - index as f64 * 0.25,
                    },
                ],
                constraints: Vec::new(),
            }),
        });
    }
    commands.push(CanonicalCommand::CreateFeature {
        id: loft,
        definition_id: definition,
        name: "Holed Loft".into(),
        kind: FeatureKind::Loft {
            sections: profiles
                .into_iter()
                .enumerate()
                .map(|(index, profile)| LoftSection {
                    profile,
                    elevation_mm: index as f64 * 10.0,
                })
                .collect(),
            guide: None,
            continuity: LoftContinuity::Position,
        },
    });
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, loft).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V14);
    assert!(graph.profiles.iter().all(|profile| matches!(
        &profile.geometry,
        ExactBRepPlanarGeometry::Region { holes, .. } if holes.len() == 1
    )));
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);

    let mut mismatched = graph.clone();
    mismatched.profiles[1].geometry = match &mismatched.profiles[1].geometry {
        ExactBRepPlanarGeometry::Region {
            outer:
                ExactBRepPlanarLoop::Circle {
                    center_bits,
                    radius_bits,
                },
            ..
        } => ExactBRepPlanarGeometry::Circle {
            center_bits: *center_bits,
            radius_bits: *radius_bits,
        },
        _ => unreachable!(),
    };
    assert_eq!(
        mismatched.validate(),
        Err(ExactBRepGraphError::InvalidGraph)
    );
}

#[test]
fn worker_evaluates_persisted_guided_loft_v15_and_rejects_invalid_guides_atomically() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(202_100);
    let lower = FeatureId(202_101);
    let upper = FeatureId(202_102);
    let guide = FeatureId(202_103);
    let loft = FeatureId(202_104);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Guided Loft definition".into(),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: definition,
                name: "Lower guided section".into(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -5.0],
                    [8.0, -5.0],
                    [8.0, 5.0],
                    [-8.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(202_107),
                definition_id: definition,
                name: "Middle guided section".into(),
                kind: FeatureKind::closed_spline(&[
                    [-10.0, -2.0],
                    [7.0, -6.0],
                    [11.0, 4.0],
                    [-3.0, 8.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Upper guided section".into(),
                kind: FeatureKind::closed_spline(&[
                    [-5.0, -3.0],
                    [5.0, -3.0],
                    [5.0, 3.0],
                    [-5.0, 3.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: guide,
                definition_id: definition,
                name: "Loft spine".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 40.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: definition,
                name: "Guided tangent Loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: FeatureId(202_107),
                            elevation_mm: 20.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 40.0,
                        },
                    ],
                    guide: Some(guide),
                    continuity: LoftContinuity::Tangent,
                },
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, loft).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V15);
    assert!(matches!(
        &graph.nodes[0].operation,
        ExactBRepOperation::Loft {
            guide: Some(path),
            continuity: ketchup_model::exact_brep_graph::ExactBRepLoftContinuity::Tangent,
            ..
        } if path.source_feature_id == guide.0 && path.segments.len() == 1
    ));
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        reopened.snapshot().feature(loft).unwrap().kind(),
        snapshot.feature(loft).unwrap().kind()
    );

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);

    let mut continuity_fingerprints = std::collections::BTreeSet::new();
    for (id, continuity) in [
        (FeatureId(202_108), LoftContinuity::Position),
        (FeatureId(202_109), LoftContinuity::Tangent),
        (FeatureId(202_110), LoftContinuity::Curvature),
    ] {
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id: definition,
                name: format!("Unguided {continuity:?} Loft"),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: FeatureId(202_107),
                            elevation_mm: 20.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 40.0,
                        },
                    ],
                    guide: None,
                    continuity,
                },
            }]))
            .unwrap();
        let continuity_graph =
            ExactBRepGraph::from_snapshot(&document.current(), definition, id).unwrap();
        let continuity_package = supervisor
            .evaluate_exact_brep_graph(&continuity_graph)
            .unwrap();
        assert_eq!(continuity_package.topology_counts[4], 1);
        assert!(continuity_package.volume_mm3.is_finite() && continuity_package.volume_mm3 > 0.0);
        continuity_fingerprints.insert(continuity_package.identity.result_fingerprint);
    }
    assert_eq!(
        continuity_fingerprints.len(),
        3,
        "OCCT C0/C1/C2 Loft continuity must produce distinct exact shapes for asymmetric sections"
    );

    let before_intersection = document.current().canonical_digest();
    let undo_before_intersection = document.visible_undo_steps();
    let intersecting_guide = FeatureId(202_111);
    let rejected = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateFeature {
            id: intersecting_guide,
            definition_id: definition,
            name: "Self-intersecting Loft guide".into(),
            kind: FeatureKind::SpatialPath {
                segments: vec![
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
                ],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(202_112),
            definition_id: definition,
            name: "Rejected intersecting guided Loft".into(),
            kind: FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: lower,
                        elevation_mm: 0.0,
                    },
                    LoftSection {
                        profile: upper,
                        elevation_mm: 40.0,
                    },
                ],
                guide: Some(intersecting_guide),
                continuity: LoftContinuity::Tangent,
            },
        },
    ]));
    assert_eq!(rejected.err(), Some(CanonicalError::InvalidSweep));
    assert_eq!(document.current().canonical_digest(), before_intersection);
    assert_eq!(document.visible_undo_steps(), undo_before_intersection);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&graph)
            .unwrap()
            .identity
            .result_fingerprint,
        package.identity.result_fingerprint
    );

    for (id, guide, continuity, expected) in [
        (
            FeatureId(202_105),
            Some(FeatureId(999_999)),
            LoftContinuity::Tangent,
            CanonicalError::FeatureNotFound(FeatureId(999_999)),
        ),
        (
            FeatureId(202_106),
            Some(guide),
            LoftContinuity::Curvature,
            CanonicalError::InvalidLoft,
        ),
    ] {
        let before = document.current().canonical_digest();
        let undo_steps = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id: definition,
                name: "Rejected guided Loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: FeatureId(202_107),
                            elevation_mm: 20.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 40.0,
                        },
                    ],
                    guide,
                    continuity,
                },
            }]))
            .err()
            .expect("invalid guided Loft must reject atomically");
        assert_eq!(error, expected);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), undo_steps);
    }
}

#[test]
fn worker_evaluates_mixed_spline_and_sketch_loft_in_shifted_rotated_frames() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(1975);
    let lower = FeatureId(19_750);
    let middle_plane = FeatureId(19_751);
    let middle = FeatureId(19_752);
    let upper_plane = FeatureId(19_753);
    let upper = FeatureId(19_754);
    let loft = FeatureId(19_755);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Framed mixed Loft".into(),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: definition,
                name: "Legacy spline section".into(),
                kind: FeatureKind::closed_spline(&[
                    [-10.0, -6.0],
                    [10.0, -6.0],
                    [9.0, 7.0],
                    [-8.0, 8.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: middle_plane,
                definition_id: definition,
                name: "Shifted rotated middle plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [2.0, -1.0, -10.0],
                        [0.866_025_403_784, 0.5, 0.0],
                        [-0.5, 0.866_025_403_784, 0.0],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: middle,
                definition_id: definition,
                name: "Middle sketch circle".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: middle_plane,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [1.0, -1.0],
                        radius_mm: 8.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: upper_plane,
                definition_id: definition,
                name: "Shifted tilted upper plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [4.0, 2.0, -18.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 0.939_692_620_786, 0.342_020_143_326],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Upper sketch circle".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: upper_plane,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [-1.0, 1.0],
                        radius_mm: 6.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: definition,
                name: "Mixed framed Loft solid".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: middle,
                            elevation_mm: 20.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 40.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
        ]))
        .unwrap();

    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, loft).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V14);
    let ExactBRepOperation::Loft { sections, .. } = &graph.nodes[0].operation else {
        panic!("expected terminal Loft operation");
    };
    assert_eq!(
        sections
            .iter()
            .map(|section| graph.profiles[section.profile.0 as usize].source_feature_id)
            .collect::<Vec<_>>(),
        vec![lower.0, middle.0, upper.0]
    );
    assert!(matches!(
        graph.profiles[sections[0].profile.0 as usize].geometry,
        ExactBRepPlanarGeometry::Spline { .. }
    ));
    assert_ne!(
        graph.profiles[sections[1].profile.0 as usize].frame_bits,
        graph.profiles[sections[2].profile.0 as usize].frame_bits
    );

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);
    assert!(package.bounds_mm[1][2] > package.bounds_mm[0][2]);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
}
