use ketchup_core::cam::{
    CamCollisionParticipant, CamCollisionTarget, CamCutParameters, CamFixture, CamMotionKind,
    CamOperation, CamPath2d, CamPathSegment2d, CamPlan, CamPlanId, CamPostprocessorDialect,
    CamPostprocessorError, CamPostprocessorOutput, CamSetup, CamStock, CamTool, CamToolKind,
    CamToolpath, CamWorkOffset,
};
use ketchup_core::document::{
    BodyKind, BooleanOperation, CanonicalCommand, CanonicalError, ChamferEdgeSide, ChamferMode,
    CommandBatch, DefinitionId, Dimension, DocumentStore, EdgeFinishKind, FeatureId, FeatureKind,
    FeatureParameterTarget, FilletRadiusStation, InstancePath, LoftContinuity, LoftSection, NodeId,
    OccurrenceId, ParameterValueType, ProfileSegment, ShellDirection, Snapshot, SolidToolPlan,
    SpatialPathSegment, SurfaceBodySpec, Transform,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V8, EXACT_BREP_GRAPH_SCHEMA_V9, EXACT_BREP_GRAPH_SCHEMA_V10,
    EXACT_BREP_GRAPH_SCHEMA_V11, EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V14,
    EXACT_BREP_GRAPH_SCHEMA_V15, EXACT_BREP_GRAPH_SCHEMA_V17, EXACT_BREP_GRAPH_SCHEMA_V18,
    EXACT_BREP_GRAPH_SCHEMA_V20, EXACT_BREP_GRAPH_SCHEMA_V21, EXACT_BREP_GRAPH_SCHEMA_V22,
    EXACT_BREP_GRAPH_SCHEMA_V23, ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation,
    ExactBRepPlanarGeometry, ExactBRepPlanarLoop, ExactBRepPlanarSegment,
    MAX_EXACT_BREP_GRAPH_PROFILES,
};
use ketchup_core::exact_product::{
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest, ExactPlanarOffsetRequest, ExactProductError, ExactResultRegistry,
};
use ketchup_core::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{StepImportMesh, StepMeshTriangle, plan_iges_import, plan_step_import};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, PadSpec, PocketSpec, PrincipalPlane,
    SketchEntity, SketchEntityId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};
use ketchup_exact::{ExactBackend, RectangleExtrudeSpec};
use ketchup_scheduler::{
    DerivedResult, EvaluationScheduler, ExactFeaFaceTraction, ExactFeaSetup, ExactFeaSetupError,
    ExactVolumeMeshWireOptions, ExactWorkerSupervisor, InsertOutcome, WorkerError,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn assert_bounds_close(actual: [[f64; 3]; 2], expected: [f64; 6]) {
    for (actual, expected) in actual.into_iter().flatten().zip(expected) {
        assert!((actual - expected).abs() <= 2.0e-7);
    }
}

fn assert_geometry_error(error: WorkerError, code: &str) {
    match error {
        WorkerError::Geometry(detail) => assert!(detail.starts_with(code), "{detail}"),
        other => panic!("expected {code} geometry refusal, got {other:?}"),
    }
}

fn assert_surface_interchange_roundtrip(
    supervisor: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
    package: &ExactBRepGraphPackage,
    stem: &str,
) {
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join(format!("{stem}.step"));
    let cancelled = AtomicBool::new(false);
    supervisor
        .export_exact_brep_graph_step(snapshot, package, &step_path)
        .unwrap();
    let step = std::fs::read(&step_path).unwrap();
    let step_sha256 = sha256_hex(&step);
    let step_evidence = supervisor
        .inspect_step_import_with_cancellation(&step_path, &step_sha256, &cancelled)
        .unwrap();
    assert_eq!(
        supervisor
            .inspect_step_import_with_cancellation(&step_path, &step_sha256, &cancelled)
            .unwrap(),
        step_evidence
    );
    assert_eq!(step_evidence.body_kind, BodyKind::Surface);
    assert_eq!(step_evidence.solid_count, 0);
    assert_eq!(step_evidence.volume_mm3, 0.0);
    assert_eq!(
        step_evidence.topology_counts[..3],
        package.topology_counts[..3]
    );
    assert!(step_evidence.topology_counts[3] > 0);
    assert_eq!(step_evidence.topology_counts[4], 0);
    assert!(step_evidence.area_mm2.is_finite() && step_evidence.area_mm2 > 0.0);
    assert!(
        (step_evidence.area_mm2 - package.area_mm2).abs()
            <= package.area_mm2.abs().max(1.0) * 1.0e-6
    );
    for (actual, expected) in step_evidence
        .bounds_mm
        .into_iter()
        .flatten()
        .zip(package.bounds_mm.into_iter().flatten())
    {
        assert!((actual - expected).abs() <= 1.0e-6);
    }

    let mut step_document = DocumentStore::new();
    let step_before = step_document.current().canonical_digest();
    let step_batch = plan_step_import(
        &step_document.current(),
        &step,
        &format!("{stem}.step"),
        &step_evidence,
    )
    .unwrap();
    assert_eq!(
        plan_step_import(
            &step_document.current(),
            &step,
            &format!("{stem}.step"),
            &step_evidence,
        )
        .unwrap()
        .digest(),
        step_batch.digest()
    );
    let mut invalid_step_evidence = step_evidence.clone();
    invalid_step_evidence.solid_count = 1;
    assert!(
        plan_step_import(
            &step_document.current(),
            &step,
            &format!("invalid-{stem}.step"),
            &invalid_step_evidence,
        )
        .is_err()
    );
    assert_eq!(step_document.current().canonical_digest(), step_before);
    assert_eq!(step_document.visible_undo_steps(), 0);
    let proposal = step_document.prepare_proposal(step_batch).unwrap();
    step_document.commit_verified_proposal(&proposal).unwrap();
    let step_committed = step_document.current();
    let step_spec = step_committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(step_spec.body_kind, BodyKind::Surface);
    assert_eq!(step_spec.area_mm2, step_evidence.area_mm2);
    assert_eq!(step_spec.volume_mm3, 0.0);
    let mut step_container = persistence::ContainerData::default();
    step_container.insert_import_blob(step).unwrap();
    let encoded = persistence::save_container(&step_committed, &step_container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        step_committed.canonical_digest()
    );
    assert_eq!(
        step_document.undo().unwrap().canonical_digest(),
        step_before
    );
    assert_eq!(step_document.visible_undo_steps(), 0);

    let iges_path = directory.path().join(format!("{stem}.iges"));
    supervisor
        .convert_step_to_iges_with_cancellation(&step_path, &iges_path, &cancelled)
        .unwrap();
    let iges = std::fs::read(&iges_path).unwrap();
    let iges_sha256 = sha256_hex(&iges);
    let iges_evidence = supervisor
        .inspect_iges_import_with_cancellation(&iges_path, &iges_sha256, &cancelled)
        .unwrap();
    assert_eq!(
        supervisor
            .inspect_iges_import_with_cancellation(&iges_path, &iges_sha256, &cancelled)
            .unwrap(),
        iges_evidence
    );
    assert_eq!(iges_evidence.body_kind, BodyKind::Surface);
    assert_eq!(iges_evidence.solid_count, 0);
    assert_eq!(iges_evidence.volume_mm3, 0.0);
    assert_eq!(
        iges_evidence.topology_counts[..3],
        package.topology_counts[..3]
    );
    assert!(iges_evidence.topology_counts[3] > 0);
    assert_eq!(iges_evidence.topology_counts[4], 0);
    assert!(iges_evidence.area_mm2.is_finite() && iges_evidence.area_mm2 > 0.0);
    assert!(
        (iges_evidence.area_mm2 - package.area_mm2).abs()
            <= package.area_mm2.abs().max(1.0) * 1.0e-6
    );
    for (actual, expected) in iges_evidence
        .bounds_mm
        .into_iter()
        .flatten()
        .zip(package.bounds_mm.into_iter().flatten())
    {
        assert!((actual - expected).abs() <= 1.0e-6);
    }

    let mut iges_document = DocumentStore::new();
    let iges_before = iges_document.current().canonical_digest();
    let iges_batch = plan_iges_import(
        &iges_document.current(),
        &iges,
        &format!("{stem}.iges"),
        &iges_evidence,
    )
    .unwrap();
    assert_eq!(
        plan_iges_import(
            &iges_document.current(),
            &iges,
            &format!("{stem}.iges"),
            &iges_evidence,
        )
        .unwrap()
        .digest(),
        iges_batch.digest()
    );
    let mut invalid_iges_evidence = iges_evidence.clone();
    invalid_iges_evidence.volume_mm3 = 1.0;
    assert!(
        plan_iges_import(
            &iges_document.current(),
            &iges,
            &format!("invalid-{stem}.iges"),
            &invalid_iges_evidence,
        )
        .is_err()
    );
    assert_eq!(iges_document.current().canonical_digest(), iges_before);
    assert_eq!(iges_document.visible_undo_steps(), 0);
    let proposal = iges_document.prepare_proposal(iges_batch).unwrap();
    iges_document.commit_verified_proposal(&proposal).unwrap();
    let iges_committed = iges_document.current();
    let iges_spec = iges_committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(iges_spec.body_kind, BodyKind::Surface);
    assert_eq!(iges_spec.area_mm2, iges_evidence.area_mm2);
    assert_eq!(iges_spec.volume_mm3, 0.0);
    let mut iges_container = persistence::ContainerData::default();
    iges_container.insert_import_blob(iges).unwrap();
    let encoded = persistence::save_container(&iges_committed, &iges_container).unwrap();
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        iges_committed.canonical_digest()
    );
    assert_eq!(
        iges_document.undo().unwrap().canonical_digest(),
        iges_before
    );
    assert_eq!(iges_document.visible_undo_steps(), 0);
}

#[test]
fn exact_worker_converts_verified_step_to_iges_and_reinspects_exact_evidence() {
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let iges = directory.path().join("box.iges");
    let cancelled = AtomicBool::new(false);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    supervisor
        .convert_step_to_iges_with_cancellation(&source, &iges, &cancelled)
        .unwrap();
    let bytes = std::fs::read(&iges).unwrap();
    let evidence = supervisor
        .inspect_iges_import_with_cancellation(&iges, &sha256_hex(&bytes), &cancelled)
        .unwrap();

    assert_eq!(
        evidence.source_sha256,
        ketchup_core::graph::sha256_bytes(&bytes)
    );
    assert_eq!(evidence.source_byte_len, bytes.len() as u64);
    assert_eq!(
        evidence.source_unit,
        ketchup_core::import::ImportLengthUnit::Millimetre
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

fn simple_extrusion_document() -> (DocumentStore, DefinitionId, FeatureId, OccurrenceId) {
    let definition = DefinitionId(90);
    let profile = FeatureId(900);
    let extrusion = FeatureId(901);
    let occurrence = OccurrenceId(900);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Safety graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Safety profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 6.0], [0.0, 6.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: definition,
                name: "Safety extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(5.0),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Safety occurrence".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    (document, definition, extrusion, occurrence)
}

fn simple_extrusion_graph() -> ExactBRepGraph {
    let (document, definition, extrusion, _) = simple_extrusion_document();
    ExactBRepGraph::from_snapshot(&document.current(), definition, extrusion).unwrap()
}

#[test]
fn exact_graph_volume_mesh_is_identity_bound_bounded_and_cancellable() {
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

fn cam_simulation_document() -> (DocumentStore, CamPlan) {
    let definition = DefinitionId(95);
    let profile = FeatureId(950);
    let solid = FeatureId(951);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "CAM simulation target".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "20x10 profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "20x10x5 target".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(5.0),
                },
            },
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &document.current(),
        CamPlanId(95),
        "Top face simulation",
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [20.0, 10.0, 7.0],
        },
        CamTool {
            number: 1,
            kind: CamToolKind::FlatEndMill,
            diameter_mm: 2.0,
            flute_length_mm: 5.0,
            overall_length_mm: 10.0,
            holder_diameter_mm: 6.0,
            holder_length_mm: 10.0,
            spindle_rpm: 10_000,
            feed_mm_per_min: 600.0,
            plunge_mm_per_min: 200.0,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 10.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        definition,
        solid,
    )
    .unwrap();
    (document, plan)
}

#[test]
fn cam_simulation_removes_stock_exactly_and_reports_fixture_gouge_and_stale_refusals() {
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
    assert_eq!(
        results.iter().map(|r| r.relation).collect::<Vec<_>>(),
        vec![
            ExactPairRelation::Separated,
            ExactPairRelation::Penetrating,
            ExactPairRelation::Penetrating,
            ExactPairRelation::Touching,
            ExactPairRelation::Touching,
            ExactPairRelation::Touching
        ]
    );
    assert!((results[0].distance_mm - 12.0).abs() < 1e-7);
    assert!((results[1].common_volume_mm3 - 120.0).abs() < 1e-7);
    assert!((results[2].common_volume_mm3 - 3.75).abs() < 1e-7);
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
    assert!((result[0].common_volume_mm3 - 180.0).abs() < 1e-7);
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
    assert_eq!(result[0].relation, ExactPairRelation::Separated);
    assert!((result[0].distance_mm - 1.0).abs() < 1e-7);
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
    assert!(
        worker
            .query_exact_brep_pairs(
                &graphs,
                &[candidate(0.0, 0.0, 0.0, 0.0)],
                &BTreeMap::new(),
                1e-7
            )
            .is_err()
    );
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

fn generated_boolean_scales() -> Vec<[f64; 3]> {
    let mut samples = vec![[0.5, 0.75, 0.6], [1.0, 1.0, 1.0], [3.0, 2.5, 1.75]];
    let mut state = 0x4558_4143_5420_2026_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 11) as f64) / ((1_u64 << 53) as f64)
    };
    samples.extend((0..3).map(|_| [0.5 + next() * 2.5, 0.5 + next() * 2.0, 0.5 + next() * 1.25]));
    samples
}

fn rigid_profile_variant(points: &[[f64; 2]], variant: usize) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|[x, y]| match variant {
            0 => [*x, *y],
            1 => [*x + 37.0, *y - 23.0],
            2 => [-*y + 11.0, *x + 29.0],
            _ => unreachable!("property harness has exactly three rigid variants"),
        })
        .collect()
}

fn generated_boolean_document(
    scales: [f64; 3],
    rigid_variant: usize,
) -> (
    DocumentStore,
    DefinitionId,
    FeatureId,
    FeatureId,
    [(FeatureId, BooleanOperation); 4],
) {
    let definition = DefinitionId(80);
    let base_profile = FeatureId(800);
    let base = FeatureId(801);
    let tool_profile = FeatureId(802);
    let tool = FeatureId(803);
    let operations = [
        (FeatureId(804), BooleanOperation::Cut),
        (FeatureId(805), BooleanOperation::Union),
        (FeatureId(806), BooleanOperation::Intersect),
        (FeatureId(807), BooleanOperation::Split),
    ];
    let [scale_x, scale_y, scale_z] = scales;
    let scale_profile = |points: &[[f64; 2]]| {
        points
            .iter()
            .map(|[x, y]| [x * scale_x, y * scale_y])
            .collect::<Vec<_>>()
    };
    let base_points = rigid_profile_variant(
        &scale_profile(&[
            [-12.0, -8.0],
            [18.0, -6.0],
            [24.0, 9.0],
            [3.0, 20.0],
            [-17.0, 7.0],
        ]),
        rigid_variant,
    );
    let tool_points = rigid_profile_variant(
        &scale_profile(&[[-3.0, -15.0], [27.0, 4.0], [5.0, 24.0]]),
        rigid_variant,
    );
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: "Generated Boolean property graph".into(),
        },
        CanonicalCommand::CreateFeature {
            id: base_profile,
            definition_id: definition,
            name: "Generated target profile".into(),
            kind: FeatureKind::Profile {
                points_mm: base_points,
            },
        },
        CanonicalCommand::CreateFeature {
            id: base,
            definition_id: definition,
            name: "Generated target body".into(),
            kind: FeatureKind::Extrusion {
                profile: base_profile,
                height: dimension(13.0 * scale_z),
            },
        },
        CanonicalCommand::CreateFeature {
            id: tool_profile,
            definition_id: definition,
            name: "Generated tool profile".into(),
            kind: FeatureKind::Profile {
                points_mm: tool_points,
            },
        },
        CanonicalCommand::CreateFeature {
            id: tool,
            definition_id: definition,
            name: "Generated tool body".into(),
            kind: FeatureKind::Extrusion {
                profile: tool_profile,
                height: dimension(19.0 * scale_z),
            },
        },
    ];
    commands.extend(
        operations.map(|(id, operation)| CanonicalCommand::CreateFeature {
            id,
            definition_id: definition,
            name: format!("Generated {operation:?}"),
            kind: FeatureKind::Boolean {
                operation,
                target: base,
                tool,
            },
        }),
    );
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    (document, definition, base, tool, operations)
}

#[test]
fn worker_evaluates_v9_curved_sweep_with_deterministic_topology_and_mesh() {
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-arc path".into(),
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
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Curved sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-arc-line path".into(),
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
                id: sweep,
                definition_id: definition,
                name: "Multisegment sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
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
        ketchup_core::exact_brep_graph::ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if *matrix_bits == tool_transform.matrix().map(f64::to_bits)
    )));
    assert_ne!(target_definition, result_definition);
    let pair_sources = sources
        .iter()
        .map(|source| (sha256_hex(source), source.clone()))
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
        pair_result[0].relation,
        ketchup_scheduler::ExactPairRelation::Penetrating
    );
    let mut corrupt_sources = pair_sources.clone();
    corrupt_sources.values_mut().next().unwrap()[0] ^= 1;
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
            ketchup_core::exact_brep_graph::ExactBRepOperation::Boolean {
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
        (sha256_hex(&sources[0]), sources[0].clone()),
        (sha256_hex(&sources[1]), sources[1].clone()),
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
        Err(ketchup_core::import::StepImportPlanError::InvalidWorkerEvidence)
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
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let intersect = operations[2].0;
    let split = operations[3].0;
    let snapshot = document.current();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_producer(&snapshot, definition, intersect),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );

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
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
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
            .zip(&result.triangle_face_ordinals)
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(10.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: pocket_profile,
                definition_id: definition,
                name: "Triangular pocket".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[6.0, 5.0], [24.0, 7.0], [12.0, 16.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: definition,
                name: "Pocket".into(),
                kind: FeatureKind::Pocket {
                    target: base,
                    profile: pocket_profile,
                    depth: dimension(4.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: through_cut,
                definition_id: definition,
                name: "Through cut".into(),
                kind: FeatureKind::ThroughCut {
                    target: base,
                    profile: pocket_profile,
                },
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, -10.0], [30.0, -10.0], [30.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(10.0),
                },
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
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(5.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: tool_profile,
                definition_id: definition,
                name: "Remote tool profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[30.0, 0.0], [35.0, 0.0], [35.0, 5.0], [30.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: tool,
                definition_id: definition,
                name: "Remote tool".into(),
                kind: FeatureKind::Extrusion {
                    profile: tool_profile,
                    height: dimension(7.0),
                },
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
fn graph_results_are_stale_safe_and_resource_or_unsupported_inputs_fail_closed() {
    let valid_graph = simple_extrusion_graph();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let valid_result = supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap();

    let node = NodeId(77);
    let mut scheduler = EvaluationScheduler::new(1_024);
    scheduler
        .advance_revision(valid_graph.source_revision, [node])
        .unwrap();
    let stale_token = scheduler
        .schedule(node, valid_graph.canonical_input_digest.clone())
        .unwrap();
    scheduler
        .advance_revision(valid_graph.source_revision + 1, [node])
        .unwrap();
    assert_eq!(
        scheduler.accept(DerivedResult {
            token: stale_token,
            result_fingerprint: valid_result.identity.result_fingerprint.clone(),
            charge_bytes: 64,
        }),
        InsertOutcome::Stale
    );
    assert_eq!(scheduler.current_result_fingerprint(node), None);

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
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-3.0, -2.0], [4.0, -2.0], [4.0, 3.0], [-3.0, 3.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Straight path".into(),
                kind: FeatureKind::SegmentProfile {
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
                kind: FeatureKind::Sweep { profile, path },
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, -4.0], [7.0, -3.0], [5.0, 6.0], [2.0, 5.0]],
                },
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
                kind: FeatureKind::SegmentProfile {
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
                kind: FeatureKind::SegmentProfile {
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
                kind: FeatureKind::Sweep {
                    profile: sweep_profile,
                    path: sweep_path,
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_lower,
                definition_id: definition,
                name: "Lower spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -5.0], [9.0, -4.0], [8.0, 6.0], [-7.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_upper,
                definition_id: definition,
                name: "Upper spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -3.0], [6.0, -2.0], [5.0, 4.0], [-3.0, 3.0]],
                },
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
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -5.0], [8.0, -5.0], [8.0, 5.0], [-8.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(202_107),
                definition_id: definition,
                name: "Middle guided section".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-10.0, -2.0], [7.0, -6.0], [11.0, 4.0], [-3.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Upper guided section".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-5.0, -3.0], [5.0, -3.0], [5.0, 3.0], [-5.0, 3.0]],
                },
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
            continuity: ketchup_core::exact_brep_graph::ExactBRepLoftContinuity::Tangent,
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
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-10.0, -6.0], [10.0, -6.0], [9.0, 7.0], [-8.0, 8.0]],
                },
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

#[test]
fn planar_surface_body_round_trips_rebuilds_and_remains_distinct_from_a_solid() {
    let definition = DefinitionId(95);
    let profile = FeatureId(950);
    let surface = FeatureId(951);
    let solid = FeatureId(952);
    let invalid_boolean = FeatureId(953);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Planar surface part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Surface boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 25.0], [0.0, 25.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: surface,
                definition_id: definition,
                name: "Planar surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Comparison solid".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(5.0),
                },
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    assert_eq!(
        snapshot.feature(surface).unwrap().kind().body_kind(),
        Some(BodyKind::Surface)
    );
    assert_eq!(
        snapshot.feature(solid).unwrap().kind().body_kind(),
        Some(BodyKind::Solid)
    );
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, surface).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V20);
    assert_eq!(graph.terminal_body_kind(), BodyKind::Surface);
    assert!(matches!(
        graph.nodes.last().unwrap().operation,
        ExactBRepOperation::PlanarSurface { .. }
    ));

    let loaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert!(loaded.is_editable());
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(matches!(
        loaded.snapshot().feature(surface).unwrap().kind(),
        FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile: source }) if *source == profile
    ));

    let before_invalid = document.current();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: invalid_boolean,
            definition_id: definition,
            name: "Invalid surface boolean".into(),
            kind: FeatureKind::Boolean {
                operation: BooleanOperation::Union,
                target: surface,
                tool: solid,
            },
        }])),
        Err(CanonicalError::InvalidFeatureOwnership(id)) if id == invalid_boolean
    ));
    assert_eq!(
        document.current().revision_id(),
        before_invalid.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid.canonical_digest()
    );

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.volume_mm3, 0.0);
    assert!((package.area_mm2 - 1_000.0).abs() <= 1.0e-7);
    assert_eq!(package.topology_counts[2..], [1, 0, 0]);
    assert!(package.is_current(&snapshot));
    assert_surface_interchange_roundtrip(&mut supervisor, &snapshot, &package, "planar-surface");

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: profile,
                points_mm: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 25.0], [0.0, 25.0]],
            },
        ]))
        .unwrap();
    let edited = document.current();
    assert_ne!(edited.canonical_digest(), snapshot.canonical_digest());
    assert!(!package.is_current(&edited));
    let edited_graph = ExactBRepGraph::from_snapshot(&edited, definition, surface).unwrap();
    let edited_package = supervisor.evaluate_exact_brep_graph(&edited_graph).unwrap();
    assert_eq!(edited_package.volume_mm3, 0.0);
    assert!((edited_package.area_mm2 - 1_250.0).abs() <= 1.0e-7);

    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(package.is_current(&document.current()));
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        edited.canonical_digest()
    );
    assert!(edited_package.is_current(&document.current()));
}

#[test]
fn loft_surface_is_open_exact_geometry_and_planar_surface_rejects_profile_holes() {
    let definition = DefinitionId(2050);
    let lower = FeatureId(20_500);
    let upper = FeatureId(20_501);
    let surface = FeatureId(20_502);
    let workplane = FeatureId(20_503);
    let holed_profile = FeatureId(20_504);
    let rejected_surface = FeatureId(20_505);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Surface loft definition".into(),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: definition,
                name: "Lower surface section".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Upper surface section".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-6.0, -3.0], [6.0, -3.0], [6.0, 3.0], [-6.0, 3.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: surface,
                definition_id: definition,
                name: "Open loft surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 15.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                }),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: holed_profile,
                definition_id: definition,
                name: "Annulus".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![
                        SketchEntity::Circle {
                            id: SketchEntityId(1),
                            center_mm: [0.0, 0.0],
                            radius_mm: 10.0,
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(2),
                            center_mm: [0.0, 0.0],
                            radius_mm: 2.0,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: rejected_surface,
                definition_id: definition,
                name: "Unsupported holed planar surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar {
                    profile: holed_profile,
                }),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, surface).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V20);
    assert_eq!(graph.terminal_body_kind(), BodyKind::Surface);
    assert!(matches!(
        graph.nodes.last().unwrap().operation,
        ExactBRepOperation::LoftSurface { .. }
    ));
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.volume_mm3, 0.0);
    assert!(package.area_mm2.is_finite() && package.area_mm2 > 0.0);
    assert!(package.topology_counts[2] > 0);
    assert_eq!(package.topology_counts[4], 0);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert_surface_interchange_roundtrip(&mut supervisor, &snapshot, &package, "loft-surface");

    assert!(matches!(
        ExactBRepGraph::from_snapshot(&snapshot, definition, rejected_surface),
        Err(ExactBRepGraphError::InvalidGraph)
    ));
}

#[test]
fn surface_trim_and_extend_round_trip_through_the_real_worker() {
    let definition = DefinitionId(2051);
    let target_profile = FeatureId(20_510);
    let cutter_profile = FeatureId(20_511);
    let target = FeatureId(20_512);
    let cutter = FeatureId(20_513);
    let trim = FeatureId(20_514);
    let extend = FeatureId(20_515);
    let solid = FeatureId(20_516);
    let invalid_extend = FeatureId(20_517);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Surface trim and extend".into(),
            },
            CanonicalCommand::CreateFeature {
                id: target_profile,
                definition_id: definition,
                name: "Target boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 25.0], [0.0, 25.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: cutter_profile,
                definition_id: definition,
                name: "Cutter boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[10.0, 5.0], [30.0, 5.0], [30.0, 20.0], [10.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: target,
                definition_id: definition,
                name: "Target surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar {
                    profile: target_profile,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: cutter,
                definition_id: definition,
                name: "Cutter surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar {
                    profile: cutter_profile,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: trim,
                definition_id: definition,
                name: "Trimmed surface".into(),
                kind: FeatureKind::SurfaceTrim { target, cutter },
            },
            CanonicalCommand::CreateFeature {
                id: extend,
                definition_id: definition,
                name: "Extended surface".into(),
                kind: FeatureKind::SurfaceExtend {
                    target: trim,
                    distance: dimension(5.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Solid comparison".into(),
                kind: FeatureKind::Extrusion {
                    profile: target_profile,
                    height: dimension(5.0),
                },
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let trim_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, trim).unwrap();
    assert_eq!(trim_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V21);
    assert_eq!(trim_graph.terminal_body_kind(), BodyKind::Surface);
    assert!(matches!(
        trim_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SurfaceTrim { .. }
    ));
    let extend_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, extend).unwrap();
    assert_eq!(extend_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V21);
    assert!(matches!(
        extend_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SurfaceExtend { .. }
    ));

    let loaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(matches!(
        loaded.snapshot().feature(trim).unwrap().kind(),
        FeatureKind::SurfaceTrim { target: loaded_target, cutter: loaded_cutter }
            if *loaded_target == target && *loaded_cutter == cutter
    ));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let trim_package = supervisor.evaluate_exact_brep_graph(&trim_graph).unwrap();
    assert_eq!(trim_package.volume_mm3, 0.0);
    assert_eq!(trim_package.topology_counts[2..], [1, 0, 0]);
    assert!((trim_package.area_mm2 - 300.0).abs() <= 1.0e-7);
    let extend_package = supervisor.evaluate_exact_brep_graph(&extend_graph).unwrap();
    assert_eq!(extend_package.volume_mm3, 0.0);
    assert_eq!(extend_package.topology_counts[2..], [1, 0, 0]);
    assert!((extend_package.area_mm2 - 750.0).abs() <= 1.0e-7);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&extend_graph).unwrap(),
        extend_package
    );

    let before_invalid = document.current();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: invalid_extend,
                definition_id: definition,
                name: "Invalid solid extend".into(),
                kind: FeatureKind::SurfaceExtend {
                    target: solid,
                    distance: dimension(5.0),
                },
            }]))
            .is_err()
    );
    assert_eq!(
        document.current().revision_id(),
        before_invalid.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid.canonical_digest()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: cutter_profile,
                points_mm: vec![[12.0, 6.0], [28.0, 6.0], [28.0, 19.0], [12.0, 19.0]],
            },
        ]))
        .unwrap();
    assert!(!trim_package.is_current(&document.current()));
    assert!(!extend_package.is_current(&document.current()));
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(trim_package.is_current(&document.current()));
}

#[test]
fn surface_knit_round_trips_and_real_worker_requires_connected_watertight_inputs() {
    let definition = DefinitionId(2052);
    let mut commands = vec![CanonicalCommand::CreateDefinition {
        id: definition,
        name: "Surface knit definition".into(),
    }];
    let planes = [
        (
            WorkplaneFrame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            10.0,
            20.0,
        ),
        (
            WorkplaneFrame {
                origin_mm: [0.0, 0.0, 30.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            10.0,
            20.0,
        ),
        (
            WorkplaneFrame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [1.0, 0.0, 0.0],
            },
            20.0,
            30.0,
        ),
        (
            WorkplaneFrame {
                origin_mm: [10.0, 0.0, 0.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [1.0, 0.0, 0.0],
            },
            20.0,
            30.0,
        ),
        (
            WorkplaneFrame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [0.0, -1.0, 0.0],
            },
            10.0,
            30.0,
        ),
        (
            WorkplaneFrame {
                origin_mm: [0.0, 20.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [0.0, -1.0, 0.0],
            },
            10.0,
            30.0,
        ),
    ];
    let mut surfaces = Vec::new();
    for (index, (frame, width, height)) in planes.into_iter().enumerate() {
        let base = 20_520 + index as u64 * 3;
        let workplane = FeatureId(base);
        let sketch = FeatureId(base + 1);
        let surface = FeatureId(base + 2);
        surfaces.push(surface);
        commands.extend([
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: format!("Knit plane {index}"),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: definition,
                name: format!("Knit boundary {index}"),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            end_mm: [width, 0.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [width, 0.0],
                            end_mm: [width, height],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(3),
                            start_mm: [width, height],
                            end_mm: [0.0, height],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [0.0, height],
                            end_mm: [0.0, 0.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: surface,
                definition_id: definition,
                name: format!("Knit surface {index}"),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile: sketch }),
            },
        ]);
    }
    let solid_profile = FeatureId(20_538);
    let solid = FeatureId(20_539);
    let open_knit = FeatureId(20_540);
    let disconnected_knit = FeatureId(20_541);
    let solid_knit = FeatureId(20_542);
    commands.extend([
        CanonicalCommand::CreateFeature {
            id: solid_profile,
            definition_id: definition,
            name: "Solid comparison profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: solid,
            definition_id: definition,
            name: "Solid comparison".into(),
            kind: FeatureKind::Extrusion {
                profile: solid_profile,
                height: dimension(2.0),
            },
        },
        CanonicalCommand::CreateFeature {
            id: open_knit,
            definition_id: definition,
            name: "Open knitted shell".into(),
            kind: FeatureKind::SurfaceKnit {
                surfaces: vec![surfaces[0], surfaces[2]],
                tolerance: dimension(0.001),
                make_solid: false,
            },
        },
        CanonicalCommand::CreateFeature {
            id: disconnected_knit,
            definition_id: definition,
            name: "Disconnected knit".into(),
            kind: FeatureKind::SurfaceKnit {
                surfaces: vec![surfaces[0], surfaces[1]],
                tolerance: dimension(0.001),
                make_solid: false,
            },
        },
        CanonicalCommand::CreateFeature {
            id: solid_knit,
            definition_id: definition,
            name: "Watertight knitted solid".into(),
            kind: FeatureKind::SurfaceKnit {
                surfaces: surfaces.clone(),
                tolerance: dimension(0.001),
                make_solid: true,
            },
        },
    ]);
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let snapshot = document.current();

    let open_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, open_knit).unwrap();
    assert_eq!(open_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V22);
    assert_eq!(open_graph.terminal_body_kind(), BodyKind::Surface);
    let solid_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, solid_knit).unwrap();
    assert_eq!(solid_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V22);
    assert_eq!(solid_graph.terminal_body_kind(), BodyKind::Solid);
    assert!(matches!(
        &solid_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SurfaceKnit {
            surfaces: graph_surfaces,
            make_solid: true,
            ..
        } if graph_surfaces.len() == 6
    ));

    let loaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(matches!(
        loaded.snapshot().feature(solid_knit).unwrap().kind(),
        FeatureKind::SurfaceKnit {
            surfaces: loaded_surfaces,
            make_solid: true,
            ..
        } if loaded_surfaces == &surfaces
    ));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let open_package = supervisor.evaluate_exact_brep_graph(&open_graph).unwrap();
    assert_eq!(open_package.volume_mm3, 0.0);
    assert!((open_package.area_mm2 - 800.0).abs() <= 1.0e-7);
    assert_eq!(open_package.topology_counts[2..], [2, 1, 0]);
    let solid_package = supervisor.evaluate_exact_brep_graph(&solid_graph).unwrap();
    assert!((solid_package.volume_mm3 - 6_000.0).abs() <= 1.0e-7);
    assert_eq!(solid_package.topology_counts[2..], [6, 1, 1]);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&solid_graph).unwrap(),
        solid_package
    );

    let disconnected_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, disconnected_knit).unwrap();
    assert!(
        supervisor
            .evaluate_exact_brep_graph(&disconnected_graph)
            .is_err()
    );

    let before_invalid = document.current();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(20_543),
                definition_id: definition,
                name: "Invalid mixed-kind knit".into(),
                kind: FeatureKind::SurfaceKnit {
                    surfaces: vec![surfaces[0], solid],
                    tolerance: dimension(0.001),
                    make_solid: false,
                },
            }]))
            .is_err()
    );
    assert_eq!(
        document.current().revision_id(),
        before_invalid.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid.canonical_digest()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: FeatureParameterTarget::new(
                    solid_knit,
                    "tolerance",
                    ParameterValueType::Length,
                )
                .unwrap(),
                dimension: dimension(0.002),
            },
        ]))
        .unwrap();
    assert!(!solid_package.is_current(&document.current()));
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(solid_package.is_current(&document.current()));
}

#[test]
fn surface_thicken_round_trips_and_real_worker_preserves_associative_identity() {
    let definition = DefinitionId(2053);
    let profile = FeatureId(20_550);
    let surface = FeatureId(20_551);
    let solid_profile = FeatureId(20_552);
    let solid = FeatureId(20_553);
    let thicken = FeatureId(20_554);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Surface thicken definition".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Surface boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: surface,
                definition_id: definition,
                name: "Planar surface".into(),
                kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }),
            },
            CanonicalCommand::CreateFeature {
                id: solid_profile,
                definition_id: definition,
                name: "Solid comparison profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[30.0, 0.0], [32.0, 0.0], [32.0, 2.0], [30.0, 2.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Solid comparison".into(),
                kind: FeatureKind::Extrusion {
                    profile: solid_profile,
                    height: dimension(2.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: thicken,
                definition_id: definition,
                name: "Thickened surface".into(),
                kind: FeatureKind::SurfaceThicken {
                    target: surface,
                    thickness: dimension(2.0),
                    direction: ShellDirection::Outward,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, thicken).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V23);
    assert_eq!(graph.terminal_body_kind(), BodyKind::Solid);
    assert_eq!(
        graph.producer_bounds_mm().unwrap(),
        Some([[-2.0, -2.0, -2.0], [12.0, 22.0, 2.0]])
    );
    assert!(matches!(
        graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SurfaceThicken {
            thickness_bits,
            direction: ketchup_core::exact_brep_graph::ExactBRepShellDirection::Outward,
            ..
        } if f64::from_bits(thickness_bits) == 2.0
    ));
    let mut downgraded = graph.clone();
    downgraded.schema = EXACT_BREP_GRAPH_SCHEMA_V22.to_owned();
    assert_eq!(
        downgraded.to_bytes(),
        Err(ExactBRepGraphError::InvalidGraph)
    );

    let loaded = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(loaded.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        loaded.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(matches!(
        loaded.snapshot().feature(thicken).unwrap().kind(),
        FeatureKind::SurfaceThicken {
            target,
            thickness,
            direction: ShellDirection::Outward,
        } if *target == surface && thickness.millimetres() == 2.0
    ));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert!((package.volume_mm3 - 400.0).abs() <= 1.0e-7);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );

    let before_invalid = document.current();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(20_555),
                definition_id: definition,
                name: "Invalid solid thicken".into(),
                kind: FeatureKind::SurfaceThicken {
                    target: solid,
                    thickness: dimension(1.0),
                    direction: ShellDirection::Inward,
                },
            }]))
            .is_err()
    );
    assert_eq!(
        document.current().revision_id(),
        before_invalid.revision_id()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid.canonical_digest()
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureParameter {
                target: FeatureParameterTarget::new(
                    thicken,
                    "thickness",
                    ParameterValueType::Length,
                )
                .unwrap(),
                dimension: dimension(3.0),
            },
        ]))
        .unwrap();
    assert!(!package.is_current(&document.current()));
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(package.is_current(&document.current()));
}

#[test]
fn worker_evaluates_planar_offset_face_through_exact_brep_graph() {
    let definition = DefinitionId(96);
    let profile = FeatureId(960);
    let offset = FeatureId(961);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Graph planar offset".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line-arc capsule".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [20.0, 0.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [20.0, 0.0],
                            end_mm: [0.0, 0.0],
                            center_mm: [10.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: offset,
                definition_id: definition,
                name: "Offset face".into(),
                kind: FeatureKind::PlanarOffset {
                    profile,
                    distance: dimension(2.0),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, offset).unwrap();
    assert!(graph.terminal_is_planar_offset());

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let request = ExactPlanarOffsetRequest::from_snapshot(&snapshot, definition).unwrap();
    let dedicated = supervisor.evaluate_planar_offset(&request).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    let mesh = StepImportMesh {
        vertices_mm: package
            .vertices
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect(),
        triangles: package
            .triangles
            .iter()
            .zip(&package.triangle_face_ordinals)
            .map(|(triangle, face_ordinal)| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: *face_ordinal,
            })
            .collect(),
    };
    let evidence = |area_mm2| ExactBRepGraphWorkerEvidence {
        exact_input_digest: package.identity.exact_input_digest.clone(),
        result_fingerprint: package.identity.result_fingerprint.clone(),
        volume_mm3: package.volume_mm3,
        area_mm2,
        topology_counts: package.topology_counts,
        wire_count: None,
        bounds_mm: package.bounds_mm,
        backend: package.identity.backend.clone(),
        tolerance: package.identity.tolerance.clone(),
        faces: Vec::new(),
        edges: Vec::new(),
    };
    assert!(
        ExactBRepGraphPackage::from_worker_evidence(&graph, evidence(package.area_mm2), &mesh,)
            .is_ok()
    );
    for forged_area in [package.area_mm2 * 0.5, f64::NAN] {
        assert!(matches!(
            ExactBRepGraphPackage::from_worker_evidence(&graph, evidence(forged_area), &mesh,),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
    }
    assert!(package.is_current(&snapshot));
    assert_eq!(package.volume_mm3, 0.0);
    assert!(package.topology_counts[0] > 0);
    assert_eq!(package.topology_counts[0], package.topology_counts[1]);
    assert_eq!(package.topology_counts[2..], [1, 0, 0]);
    assert_eq!(package.bounds_mm[0][2], 0.0);
    assert_eq!(package.bounds_mm[1][2], 0.0);
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
            .all(|ordinal| *ordinal == 0)
    );
    assert_eq!(
        package.identity.exact_input_digest,
        dedicated.identity.exact_input_digest
    );
    assert_eq!(
        package.identity.result_fingerprint,
        dedicated.identity.result_fingerprint
    );
    assert_eq!(package.bounds_mm, dedicated.bounds_mm);
}

#[test]
fn worker_evaluates_framed_cubic_sketch_planar_offset() {
    const DEFINITION: DefinitionId = DefinitionId(97);
    const WORKPLANE: FeatureId = FeatureId(970);
    const SKETCH: FeatureId = FeatureId(971);
    const OFFSET: FeatureId = FeatureId(972);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Framed cubic planar offset".into(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "Rotated YZ workplane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [30.0, -20.0, 15.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Cubic enclosure".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, 0.0],
                            control_1_mm: [-20.0, 15.0],
                            control_2_mm: [20.0, 15.0],
                            end_mm: [20.0, 0.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(2),
                            start_mm: [20.0, 0.0],
                            control_1_mm: [20.0, -15.0],
                            control_2_mm: [-20.0, -15.0],
                            end_mm: [-20.0, 0.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Offset face".into(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: dimension(3.0),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, OFFSET).unwrap();
    assert!(graph.terminal_is_planar_offset());
    assert_ne!(graph.profiles[0].frame_bits, [0_u64; 12]);
    let expected_bounds = graph.producer_bounds_mm().unwrap().unwrap();
    assert_eq!(expected_bounds[0][0], 30.0);
    assert_eq!(expected_bounds[1][0], 30.0);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert!(package.is_current(&snapshot));
    assert_eq!(package.volume_mm3, 0.0);
    assert!(package.area_mm2.is_finite() && package.area_mm2 > 0.0);
    assert_eq!(package.topology_counts[2..], [1, 0, 0]);
    assert!((package.bounds_mm[0][0] - 30.0).abs() <= 1.0e-6);
    assert!((package.bounds_mm[1][0] - 30.0).abs() <= 1.0e-6);
}

#[test]
fn worker_evaluates_signed_circle_offset_through_exact_brep_graph_v6() {
    const DEFINITION: DefinitionId = DefinitionId(98);
    const WORKPLANE: FeatureId = FeatureId(980);
    const CIRCLE: FeatureId = FeatureId(981);
    const OFFSET: FeatureId = FeatureId(982);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut result_fingerprints = Vec::new();
    for (distance_mm, output_radius_mm) in [(3.0, 23.0), (-3.0, 17.0), (-19.99, 0.01)] {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Graph circular offset".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: WORKPLANE,
                    definition_id: DEFINITION,
                    name: "XY".into(),
                    kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                },
                CanonicalCommand::CreateFeature {
                    id: CIRCLE,
                    definition_id: DEFINITION,
                    name: "Circle sketch".into(),
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
                    name: "Circular offset face".into(),
                    kind: FeatureKind::PlanarOffset {
                        profile: CIRCLE,
                        distance: dimension(distance_mm),
                    },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, OFFSET).unwrap();
        assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V8);
        assert_eq!(graph.profiles.len(), 1);
        assert!(matches!(
            graph.profiles[0].geometry,
            ExactBRepPlanarGeometry::Circle { .. }
        ));
        assert_bounds_close(
            graph.producer_bounds_mm().unwrap().unwrap(),
            [
                12.0 - output_radius_mm,
                -8.0 - output_radius_mm,
                0.0,
                12.0 + output_radius_mm,
                -8.0 + output_radius_mm,
                0.0,
            ],
        );

        let dedicated_request =
            ExactPlanarOffsetRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let dedicated = supervisor
            .evaluate_planar_offset(&dedicated_request)
            .unwrap();
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.identity.producer_feature_id.0, OFFSET.0);
        assert_eq!(
            package.identity.result_fingerprint,
            dedicated.identity.result_fingerprint
        );
        assert_eq!(package.bounds_mm, dedicated.bounds_mm);
        assert_eq!(package.area_mm2, dedicated.area_mm2);
        assert_eq!(package.topology_counts, [1, 1, 1, 0, 0]);
        assert!(!package.vertices.is_empty());
        assert!(!package.triangles.is_empty());
        assert_eq!(
            package.triangles.len(),
            package.triangle_face_ordinals.len()
        );

        let mesh = StepImportMesh {
            vertices_mm: package
                .vertices
                .iter()
                .map(|vertex| vertex.position_mm)
                .collect(),
            triangles: package
                .triangles
                .iter()
                .zip(&package.triangle_face_ordinals)
                .map(|(triangle, face_ordinal)| StepMeshTriangle {
                    vertex_indices: triangle.vertex_indices,
                    face_ordinal: *face_ordinal,
                })
                .collect(),
        };
        let evidence = ExactBRepGraphWorkerEvidence {
            exact_input_digest: package.identity.exact_input_digest.clone(),
            result_fingerprint: package.identity.result_fingerprint.clone(),
            volume_mm3: package.volume_mm3,
            area_mm2: package.area_mm2,
            topology_counts: package.topology_counts,
            wire_count: None,
            bounds_mm: package.bounds_mm,
            backend: package.identity.backend.clone(),
            tolerance: package.identity.tolerance.clone(),
            faces: Vec::new(),
            edges: Vec::new(),
        };
        assert!(
            ExactBRepGraphPackage::from_worker_evidence(&graph, evidence.clone(), &mesh).is_ok()
        );
        assert!(matches!(
            ExactBRepGraphPackage::from_worker_evidence(
                &graph,
                ExactBRepGraphWorkerEvidence {
                    area_mm2: package.area_mm2 * 0.5,
                    ..evidence
                },
                &mesh,
            ),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
        result_fingerprints.push(package.identity.result_fingerprint);

        let mut collapsed = graph;
        let ExactBRepOperation::PlanarOffset { distance_bits, .. } =
            &mut collapsed.nodes[0].operation
        else {
            panic!("fixture must compile as a planar offset");
        };
        *distance_bits = (-20.0_f64).to_bits();
        assert_eq!(collapsed.validate(), Err(ExactBRepGraphError::InvalidGraph));
        if distance_mm == 3.0 {
            use std::io::{BufRead as _, Write as _};

            let encoded = serde_json::to_vec(&collapsed)
                .unwrap()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let mut worker = std::process::Command::new(env!("CARGO_BIN_EXE_ketchup-exact-worker"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = worker.stdin.take().unwrap();
            writeln!(
                stdin,
                "EVAL_BREP_GRAPH_V6 {} {}",
                collapsed.graph_digest, encoded
            )
            .unwrap();
            drop(stdin);
            let mut response = String::new();
            std::io::BufReader::new(worker.stdout.take().unwrap())
                .read_line(&mut response)
                .unwrap();
            assert_eq!(response.trim(), "ERR invalid_request");
            assert!(worker.wait().unwrap().success());
        }
    }
    assert!(
        result_fingerprints
            .windows(2)
            .all(|pair| pair[0] != pair[1])
    );
}

#[test]
fn worker_preserves_large_bounded_rectangle_offset_through_graph_v6() {
    let definition = DefinitionId(99);
    let profile = FeatureId(990);
    let offset = FeatureId(991);
    let distance_mm = 800_000.0;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Large bounded rectangle offset".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: offset,
                definition_id: definition,
                name: "Large offset".into(),
                kind: FeatureKind::PlanarOffset {
                    profile,
                    distance: dimension(distance_mm),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, offset).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V8);
    let graph_bounds = graph.producer_bounds_mm().unwrap().unwrap();
    assert!(graph_bounds[0][0] <= -distance_mm);
    assert!(graph_bounds[0][1] <= -distance_mm);
    assert!(graph_bounds[1][0] >= 10.0 + distance_mm);
    assert!(graph_bounds[1][1] >= 8.0 + distance_mm);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.volume_mm3, 0.0);
    assert_eq!(package.topology_counts[2..], [1, 0, 0]);
    assert_bounds_close(
        package.bounds_mm,
        [
            -distance_mm,
            -distance_mm,
            0.0,
            10.0 + distance_mm,
            8.0 + distance_mm,
            0.0,
        ],
    );
}

#[test]
fn worker_evaluates_signed_linear_intervals_through_one_graph_protocol() {
    let definition = DefinitionId(7);
    let workplane = FeatureId(600);
    let sketch_id = FeatureId(601);
    let one_sided = FeatureId(602);
    let symmetric = FeatureId(603);
    let bidirectional = FeatureId(604);
    let sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-2.0, -3.0],
                end_mm: [2.0, -3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [2.0, -3.0],
                end_mm: [2.0, 3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [2.0, 3.0],
                end_mm: [-2.0, 3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-2.0, 3.0],
                end_mm: [-2.0, -3.0],
            },
        ],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Signed interval graph".into(),
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
                name: "Shared rectangle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: one_sided,
                definition_id: definition,
                name: "One-sided".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: symmetric,
                definition_id: definition,
                name: "Symmetric oblique".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::Vector([1.0, 0.0, 1.0]),
                    extent: FeatureExtent::Symmetric(dimension(10.0)),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: bidirectional,
                definition_id: definition,
                name: "Unequal bidirectional".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Bidirectional {
                        along: FeatureExtentEnd::Blind(dimension(7.0)),
                        opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                    },
                }),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut packages = Vec::new();
    for producer in [one_sided, symmetric, bidirectional] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(
            supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
            package
        );
        assert_eq!(
            package.identity.canonical_input_digest,
            graph.canonical_input_digest
        );
        assert_eq!(package.graph.graph_digest, graph.graph_digest);
        packages.push(package);
    }

    assert_bounds_close(packages[0].bounds_mm, [-2.0, -3.0, 0.0, 2.0, 3.0, 10.0]);
    let oblique = 5.0 / 2.0_f64.sqrt();
    assert_bounds_close(
        packages[1].bounds_mm,
        [-2.0 - oblique, -3.0, -oblique, 2.0 + oblique, 3.0, oblique],
    );
    assert_bounds_close(packages[2].bounds_mm, [-2.0, -3.0, -3.0, 2.0, 3.0, 7.0]);
    assert_ne!(
        packages[0].identity.result_fingerprint,
        packages[1].identity.result_fingerprint
    );
    assert_ne!(
        packages[0].identity.result_fingerprint,
        packages[2].identity.result_fingerprint
    );
    assert_ne!(
        packages[1].identity.result_fingerprint,
        packages[2].identity.result_fingerprint
    );

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

    let directory = tempfile::tempdir().unwrap();
    for package in &packages {
        let path = directory.path().join(format!(
            "signed-{}.step",
            package.identity.producer_feature_id.0
        ));
        supervisor
            .export_exact_brep_graph_step(&snapshot, package, &path)
            .unwrap();
        let step = std::fs::read(path).unwrap();
        assert!(step.len() > 256);
        assert!(step.windows(9).any(|window| window == b"ISO-10303"));
    }
}

#[test]
fn positive_face_offset_bounds_drive_a_complete_through_cut_and_round_trip() {
    let definition = DefinitionId(88);
    let base_profile = FeatureId(880);
    let base = FeatureId(881);
    let offset = FeatureId(882);
    let cut_profile = FeatureId(883);
    let cut = FeatureId(884);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Offset then through cut".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Base profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let top_z = base_package.bounds_mm[1][2];
    let top_face_ordinal = base_package
        .triangles
        .iter()
        .zip(&base_package.triangle_face_ordinals)
        .find_map(|(triangle, face_ordinal)| {
            triangle
                .vertex_indices
                .iter()
                .all(|index| {
                    (base_package.vertices[*index as usize].position_mm[2] - top_z).abs() <= 1.0e-6
                })
                .then_some(*face_ordinal)
        })
        .unwrap();
    let top_face = base_package
        .topological_references
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .nth(top_face_ordinal as usize)
        .unwrap()
        .clone();
    assert_eq!(
        top_face.stability,
        TopologicalReferenceStability::Guaranteed
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: offset,
                definition_id: definition,
                name: "Positive top face offset".into(),
                kind: FeatureKind::TopologyFaceOffset {
                    target: base,
                    face: top_face,
                    distance: dimension(5.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: cut_profile,
                definition_id: definition,
                name: "Interior cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[9.0, 7.0], [16.0, 7.0], [16.0, 12.0], [9.0, 12.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: cut,
                definition_id: definition,
                name: "Through all offset body".into(),
                kind: FeatureKind::ThroughCut {
                    target: offset,
                    profile: cut_profile,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let offset_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, offset).unwrap();
    let cut_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, cut).unwrap();
    assert_eq!(
        offset_graph.producer_bounds_mm().unwrap(),
        Some([[-5.0, -5.0, -5.0], [42.0, 28.0, 24.0]])
    );
    let interval = cut_graph
        .nodes
        .iter()
        .find_map(|node| match &node.operation {
            ExactBRepOperation::ProfileCut { interval, .. } => Some(*interval),
            _ => None,
        })
        .unwrap();
    assert_eq!(interval.start_mm(), -6.0);
    assert_eq!(interval.end_mm(), 25.0);

    let offset_package = supervisor.evaluate_exact_brep_graph(&offset_graph).unwrap();
    let cut_package = supervisor.evaluate_exact_brep_graph(&cut_graph).unwrap();
    assert_bounds_close(offset_package.bounds_mm, [0.0, 0.0, 0.0, 37.0, 23.0, 24.0]);
    assert_bounds_close(cut_package.bounds_mm, [0.0, 0.0, 0.0, 37.0, 23.0, 24.0]);
    assert!((offset_package.volume_mm3 - 20_424.0).abs() <= 1.0e-7);
    assert!(
        (cut_package.volume_mm3 - 19_584.0).abs() <= 19_584.0 * 1.0e-9,
        "unexpected through-cut volume {}",
        cut_package.volume_mm3
    );
    assert!(cut_package.topology_counts[2] >= 10);
    assert_eq!(cut_package.topology_counts[4], 1);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: base,
                dimension: dimension(24.0),
            },
        ]))
        .unwrap();
    let recomputed_snapshot = document.current();
    let recomputed_offset_graph =
        ExactBRepGraph::from_snapshot(&recomputed_snapshot, definition, offset).unwrap();
    let recomputed_offset_package = supervisor
        .evaluate_exact_brep_graph(&recomputed_offset_graph)
        .unwrap();
    assert_bounds_close(
        recomputed_offset_package.bounds_mm,
        [0.0, 0.0, 0.0, 37.0, 23.0, 29.0],
    );
    assert_ne!(
        recomputed_offset_package.identity.result_fingerprint,
        offset_package.identity.result_fingerprint
    );
    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        recomputed_snapshot.canonical_digest()
    );
    let reopened_recomputed = persistence::load(&persistence::save(&recomputed_snapshot)).unwrap();
    let reopened_recomputed_graph =
        ExactBRepGraph::from_snapshot(&reopened_recomputed.snapshot(), definition, offset).unwrap();
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_recomputed_graph)
            .unwrap(),
        recomputed_offset_package
    );

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, cut).unwrap();
    assert_eq!(reopened_graph, cut_graph);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap(),
        cut_package
    );
}

#[test]
fn through_cut_uses_safe_bounds_for_revolve_loft_and_imported_exact_bodies() {
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    let revolve_definition = DefinitionId(188);
    let revolve_profile = FeatureId(1880);
    let revolve = FeatureId(1881);
    let revolve_cut_profile = FeatureId(1882);
    let revolve_cut = FeatureId(1883);
    let mut revolve_document = DocumentStore::new();
    revolve_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: revolve_definition,
                name: "Revolve through cut".into(),
            },
            CanonicalCommand::CreateFeature {
                id: revolve_profile,
                definition_id: revolve_definition,
                name: "Revolve profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, -10.0], [10.0, -10.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: revolve_definition,
                name: "Revolved cylinder".into(),
                kind: FeatureKind::Revolve {
                    profile: revolve_profile,
                    axis_start_mm: [0.0, -20.0],
                    axis_end_mm: [0.0, 20.0],
                    angle_degrees: 360.0,
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve_cut_profile,
                definition_id: revolve_definition,
                name: "Revolve cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -5.0], [2.0, -5.0], [2.0, 5.0], [-2.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve_cut,
                definition_id: revolve_definition,
                name: "Revolve through all".into(),
                kind: FeatureKind::ThroughCut {
                    target: revolve,
                    profile: revolve_cut_profile,
                },
            },
        ]))
        .unwrap();
    let revolve_snapshot = revolve_document.current();
    let revolve_graph =
        ExactBRepGraph::from_snapshot(&revolve_snapshot, revolve_definition, revolve).unwrap();
    let revolve_cut_graph =
        ExactBRepGraph::from_snapshot(&revolve_snapshot, revolve_definition, revolve_cut).unwrap();
    let revolve_package = supervisor
        .evaluate_exact_brep_graph(&revolve_graph)
        .unwrap();
    let revolve_cut_package = supervisor
        .evaluate_exact_brep_graph(&revolve_cut_graph)
        .unwrap();
    let revolve_interval = revolve_cut_graph
        .nodes
        .last()
        .and_then(|node| match node.operation {
            ExactBRepOperation::ProfileCut { interval, .. } => Some(interval),
            _ => None,
        })
        .unwrap();
    assert!(revolve_interval.start_mm() < revolve_package.bounds_mm[0][2]);
    assert!(revolve_interval.end_mm() > revolve_package.bounds_mm[1][2]);
    assert!(revolve_cut_package.volume_mm3 > 0.0);
    assert!(revolve_cut_package.volume_mm3 < revolve_package.volume_mm3);
    assert_eq!(revolve_cut_package.topology_counts[4], 1);

    let loft_definition = DefinitionId(189);
    let lower = FeatureId(1890);
    let upper = FeatureId(1891);
    let loft = FeatureId(1892);
    let loft_cut_profile = FeatureId(1893);
    let loft_cut = FeatureId(1894);
    let mut loft_document = DocumentStore::new();
    loft_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: loft_definition,
                name: "Loft through cut".into(),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: loft_definition,
                name: "Lower loft profile".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![
                        [-10.0, -10.0],
                        [10.0, -10.0],
                        [10.0, 10.0],
                        [-10.0, 10.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: loft_definition,
                name: "Upper loft profile".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -8.0], [8.0, -8.0], [8.0, 8.0], [-8.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: loft_definition,
                name: "Lofted solid".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 20.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_cut_profile,
                definition_id: loft_definition,
                name: "Loft cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_cut,
                definition_id: loft_definition,
                name: "Loft through all".into(),
                kind: FeatureKind::ThroughCut {
                    target: loft,
                    profile: loft_cut_profile,
                },
            },
        ]))
        .unwrap();
    let loft_snapshot = loft_document.current();
    let loft_graph = ExactBRepGraph::from_snapshot(&loft_snapshot, loft_definition, loft).unwrap();
    let loft_cut_graph =
        ExactBRepGraph::from_snapshot(&loft_snapshot, loft_definition, loft_cut).unwrap();
    let loft_package = supervisor.evaluate_exact_brep_graph(&loft_graph).unwrap();
    let loft_cut_package = supervisor
        .evaluate_exact_brep_graph(&loft_cut_graph)
        .unwrap();
    let loft_interval = loft_cut_graph
        .nodes
        .last()
        .and_then(|node| match node.operation {
            ExactBRepOperation::ProfileCut { interval, .. } => Some(interval),
            _ => None,
        })
        .unwrap();
    assert!(loft_interval.start_mm() < loft_package.bounds_mm[0][2]);
    assert!(loft_interval.end_mm() > loft_package.bounds_mm[1][2]);
    assert!(loft_cut_package.volume_mm3 > 0.0);
    assert!(loft_cut_package.volume_mm3 < loft_package.volume_mm3);
    assert_eq!(loft_cut_package.topology_counts[4], 1);

    let source_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let source = std::fs::read(&source_path).unwrap();
    let evidence = supervisor
        .inspect_step_import_with_cancellation(
            &source_path,
            &sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    let mut imported_document = DocumentStore::new();
    imported_document
        .apply_batch(
            &plan_step_import(
                &imported_document.current(),
                &source,
                "through-cut-source.step",
                &evidence,
            )
            .unwrap(),
        )
        .unwrap();
    let imported_definition = imported_document
        .current()
        .definitions()
        .next()
        .unwrap()
        .id();
    let imported = imported_document.current().features().next().unwrap().id();
    let imported_cut_profile = FeatureId(imported.0 + 1);
    let imported_cut = FeatureId(imported.0 + 2);
    let min = evidence.bounds_mm[0];
    let max = evidence.bounds_mm[1];
    let x0 = min[0] + (max[0] - min[0]) * 0.25;
    let x1 = min[0] + (max[0] - min[0]) * 0.5;
    let y0 = min[1] + (max[1] - min[1]) * 0.25;
    let y1 = min[1] + (max[1] - min[1]) * 0.5;
    let imported_before_cut = imported_document.current();
    let FeatureKind::ImportedExactBody(mut narrowed_spec) = imported_before_cut
        .feature(imported)
        .unwrap()
        .kind()
        .clone()
    else {
        panic!("STEP import must produce an exact body");
    };
    narrowed_spec.bounds_mm[0][2] = min[2] + (max[2] - min[2]) * 0.4;
    narrowed_spec.bounds_mm[1][2] = min[2] + (max[2] - min[2]) * 0.6;
    imported_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: imported },
            CanonicalCommand::CreateFeature {
                id: imported,
                definition_id: imported_definition,
                name: "Imported body with stale advisory bounds".into(),
                kind: FeatureKind::ImportedExactBody(narrowed_spec),
            },
        ]))
        .unwrap();
    imported_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: imported_cut_profile,
                definition_id: imported_definition,
                name: "Imported cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: imported_cut,
                definition_id: imported_definition,
                name: "Imported through all".into(),
                kind: FeatureKind::ThroughCut {
                    target: imported,
                    profile: imported_cut_profile,
                },
            },
        ]))
        .unwrap();
    let imported_snapshot = imported_document.current();
    let imported_graph =
        ExactBRepGraph::from_snapshot(&imported_snapshot, imported_definition, imported).unwrap();
    let imported_cut_graph =
        ExactBRepGraph::from_snapshot(&imported_snapshot, imported_definition, imported_cut)
            .unwrap();
    assert!(
        supervisor
            .evaluate_exact_brep_graph(&imported_cut_graph)
            .is_err()
    );
    assert_eq!(
        imported_graph.producer_bounds_mm().unwrap(),
        None,
        "serialized import operations remain source-bound rather than claiming derived bounds"
    );
    let imported_cut_package = supervisor
        .evaluate_exact_brep_graph_with_imported_sources(&imported_cut_graph, &[source.as_slice()])
        .unwrap();
    let imported_interval = imported_cut_graph
        .nodes
        .last()
        .and_then(|node| match node.operation {
            ExactBRepOperation::ProfileCut { interval, .. } => Some(interval),
            _ => None,
        })
        .unwrap();
    assert!(imported_interval.start_mm() > evidence.bounds_mm[0][2]);
    assert!(imported_interval.end_mm() < evidence.bounds_mm[1][2]);
    let expected_imported_cut_volume = evidence.volume_mm3
        - (x1 - x0) * (y1 - y0) * (evidence.bounds_mm[1][2] - evidence.bounds_mm[0][2]);
    assert!(
        (imported_cut_package.volume_mm3 - expected_imported_cut_volume).abs()
            <= expected_imported_cut_volume * 1.0e-9
    );
    assert_eq!(imported_cut_package.topology_counts[4], 1);

    let mut stale_document = DocumentStore::new();
    stale_document
        .apply_batch(
            &plan_step_import(
                &stale_document.current(),
                &source,
                "stale-through-cut-source.step",
                &evidence,
            )
            .unwrap(),
        )
        .unwrap();
    let stale_snapshot = stale_document.current();
    let stale_definition = stale_snapshot.definitions().next().unwrap().id();
    let stale_import = stale_snapshot.features().next().unwrap();
    let stale_import_id = stale_import.id();
    let FeatureKind::ImportedExactBody(mut stale_spec) = stale_import.kind().clone() else {
        panic!("STEP import must produce an exact body");
    };
    stale_spec.result_fingerprint = "stale-result-fingerprint".into();
    let stale_profile = FeatureId(stale_import_id.0 + 1);
    let stale_cut = FeatureId(stale_import_id.0 + 2);
    stale_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature {
                id: stale_import_id,
            },
            CanonicalCommand::CreateFeature {
                id: stale_import_id,
                definition_id: stale_definition,
                name: "Stale imported body".into(),
                kind: FeatureKind::ImportedExactBody(stale_spec),
            },
            CanonicalCommand::CreateFeature {
                id: stale_profile,
                definition_id: stale_definition,
                name: "Stale imported cut profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: stale_cut,
                definition_id: stale_definition,
                name: "Stale imported through all".into(),
                kind: FeatureKind::ThroughCut {
                    target: stale_import_id,
                    profile: stale_profile,
                },
            },
        ]))
        .unwrap();
    let stale_graph =
        ExactBRepGraph::from_snapshot(&stale_document.current(), stale_definition, stale_cut)
            .unwrap();
    assert!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&stale_graph, &[source.as_slice()])
            .is_err()
    );
}

#[test]
fn worker_evaluates_open_and_closed_shell_directions_atomically() {
    let definition = DefinitionId(18);
    let profile = FeatureId(1_800);
    let base = FeatureId(1_801);
    let cases = [
        (
            FeatureId(1_802),
            ketchup_core::document::ShellDirection::Inward,
            true,
        ),
        (
            FeatureId(1_803),
            ketchup_core::document::ShellDirection::Outward,
            true,
        ),
        (
            FeatureId(1_804),
            ketchup_core::document::ShellDirection::Symmetric,
            true,
        ),
        (
            FeatureId(1_805),
            ketchup_core::document::ShellDirection::Inward,
            false,
        ),
        (
            FeatureId(1_806),
            ketchup_core::document::ShellDirection::Outward,
            false,
        ),
        (
            FeatureId(1_807),
            ketchup_core::document::ShellDirection::Symmetric,
            false,
        ),
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Directional shells".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Asymmetric rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let opening = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference.kind == TopologicalElementKind::Face
                && reference.stability == TopologicalReferenceStability::Guaranteed
                && !reference.source_element_id.contains("generated-source")
                && !reference.producer_element_id.contains("generated-result")
        })
        .cloned()
        .unwrap();

    let commands = cases
        .iter()
        .map(|(id, direction, open)| CanonicalCommand::CreateFeature {
            id: *id,
            definition_id: definition,
            name: format!(
                "{direction:?} {} shell",
                if *open { "open" } else { "closed" }
            ),
            kind: FeatureKind::TopologyShell {
                target: base,
                removed_faces: if *open {
                    vec![opening.clone()]
                } else {
                    Vec::new()
                },
                thickness: dimension(1.5),
                direction: *direction,
            },
        })
        .collect();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let snapshot = document.current();
    let mut packages = Vec::new();
    for (index, (id, _, _)) in cases.iter().enumerate() {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, *id)
            .unwrap_or_else(|error| panic!("shell case {index} ({id:?}) failed: {error:?}"));
        if index > 0 {
            assert_eq!(
                graph.schema,
                ketchup_core::exact_brep_graph::EXACT_BREP_GRAPH_SCHEMA_V16
            );
        }
        let package = supervisor
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("shell case {index} ({id:?}) worker failed: {error:?}"));
        assert!(package.volume_mm3 > 0.0);
        assert_eq!(package.topology_counts[4], 1);
        packages.push(package);
    }
    for left in 0..packages.len() {
        for right in left + 1..packages.len() {
            assert_ne!(
                packages[left].identity.result_fingerprint,
                packages[right].identity.result_fingerprint
            );
        }
    }
    for group in [0..3, 3..6] {
        assert!(
            (packages[group.start].volume_mm3 - packages[group.start + 1].volume_mm3).abs()
                > 1.0e-6
        );
        assert!(
            (packages[group.start].volume_mm3 - packages[group.start + 2].volume_mm3).abs()
                > 1.0e-6
        );
        assert!(
            (packages[group.start + 1].volume_mm3 - packages[group.start + 2].volume_mm3).abs()
                > 1.0e-6
        );
    }

    let saved = persistence::save(&snapshot);
    let reopened = persistence::load(&saved).unwrap().snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    for (id, direction, open) in cases {
        assert!(matches!(
            reopened.feature(id).unwrap().kind(),
            FeatureKind::TopologyShell {
                removed_faces,
                direction: actual,
                ..
            } if removed_faces.is_empty() == !open && *actual == direction
        ));
    }

    let before_revision = document.current().revision_id();
    let before_digest = document.current().canonical_digest();
    let before_undo = document.visible_undo_steps();
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(1_808),
                definition_id: definition,
                name: "Invalid zero-thickness shell".into(),
                kind: FeatureKind::TopologyShell {
                    target: base,
                    removed_faces: Vec::new(),
                    thickness: dimension(0.0),
                    direction: ketchup_core::document::ShellDirection::Outward,
                },
            }]))
            .is_err()
    );
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);
}

#[test]
fn worker_evaluates_variable_radius_fillet_v17_and_rejects_invalid_profiles_atomically() {
    let definition = DefinitionId(19);
    let profile = FeatureId(1_900);
    let base = FeatureId(1_901);
    let constant = FeatureId(1_902);
    let variable = FeatureId(1_903);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Variable radius fillet".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Asymmetric rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let edge = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference.kind == TopologicalElementKind::Edge
                && reference.stability == TopologicalReferenceStability::Guaranteed
                && !reference.source_element_id.contains("generated-source")
                && !reference.producer_element_id.contains("generated-result")
        })
        .cloned()
        .unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: constant,
                definition_id: definition,
                name: "Constant fillet".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: vec![edge.clone()],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.5),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: variable,
                definition_id: definition,
                name: "Variable fillet".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: vec![edge.clone()],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.5),
                    fillet_radius_stations: vec![
                        FilletRadiusStation {
                            position: 0.5,
                            radius: dimension(0.8),
                        },
                        FilletRadiusStation {
                            position: 1.0,
                            radius: dimension(1.2),
                        },
                    ],
                    chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let constant_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, constant).unwrap();
    let variable_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, variable).unwrap();
    assert_ne!(constant_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V17);
    assert_eq!(variable_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V17);
    assert!(variable_graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::EdgeFinish {
            amount_bits,
            fillet_radius_stations,
            ..
        } if *amount_bits == 0.5_f64.to_bits()
            && fillet_radius_stations.len() == 2
            && fillet_radius_stations[0].position_bits == 0.5_f64.to_bits()
            && fillet_radius_stations[0].radius_bits == 0.8_f64.to_bits()
            && fillet_radius_stations[1].position_bits == 1.0_f64.to_bits()
            && fillet_radius_stations[1].radius_bits == 1.2_f64.to_bits()
    )));

    let constant_package = supervisor
        .evaluate_exact_brep_graph(&constant_graph)
        .unwrap();
    let variable_package = supervisor
        .evaluate_exact_brep_graph(&variable_graph)
        .unwrap();
    assert_eq!(variable_package.topology_counts[4], 1);
    assert!(!variable_package.vertices.is_empty());
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&variable_graph)
            .unwrap(),
        variable_package
    );
    assert_ne!(
        variable_package.identity.result_fingerprint,
        constant_package.identity.result_fingerprint
    );

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("variable-radius-fillet.step");
    supervisor
        .export_exact_brep_graph_step(&snapshot, &variable_package, &step_path)
        .unwrap();
    let step = std::fs::read(&step_path).unwrap();
    assert!(step.len() > 256);
    assert!(step.windows(9).any(|window| window == b"ISO-10303"));
    let imported = supervisor
        .inspect_step_import_with_cancellation(
            &step_path,
            &sha256_hex(&step),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(imported.solid_count, 1);
    let step_volume_error = (imported.volume_mm3 - variable_package.volume_mm3).abs();
    assert!(
        step_volume_error <= variable_package.volume_mm3 * 2.0e-8,
        "variable fillet STEP relative volume error {}; imported={}, exact={}",
        step_volume_error / variable_package.volume_mm3,
        imported.volume_mm3,
        variable_package.volume_mm3
    );

    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(1_904),
            definition_id: definition,
            name: "Invalid variable fillet".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: base,
                edges: vec![edge],
                kind: EdgeFinishKind::Fillet,
                amount: dimension(0.5),
                fillet_radius_stations: vec![FilletRadiusStation {
                    position: 0.75,
                    radius: dimension(1.0),
                }],
                chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        }])),
        Err(CanonicalError::DimensionOutsideEnvelope)
    ));
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);
}

#[test]
fn worker_evaluates_oriented_advanced_chamfers_and_rejects_non_adjacent_faces() {
    let definition = DefinitionId(17);
    let profile = FeatureId(1_700);
    let base = FeatureId(1_701);
    let two_distance = FeatureId(1_702);
    let reversed_side = FeatureId(1_703);
    let distance_angle = FeatureId(1_704);
    let shared_face_edges = FeatureId(1_705);
    let non_adjacent = FeatureId(1_706);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Advanced chamfer".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Unequal rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let native_base = ExactBackend::new()
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 37.0,
            depth_mm: 23.0,
            height_mm: 19.0,
        })
        .unwrap();
    let selected_edge = native_base
        .body
        .topology
        .edges
        .iter()
        .find(|edge| edge.adjacent_face_ordinals.len() == 2)
        .unwrap();
    let first_face = selected_edge.adjacent_face_ordinals[0];
    let second_face = selected_edge.adjacent_face_ordinals[1];
    let invalid_face = native_base
        .body
        .topology
        .faces
        .iter()
        .map(|face| face.ordinal)
        .find(|ordinal| !selected_edge.adjacent_face_ordinals.contains(ordinal))
        .unwrap();
    let shared_face = native_base
        .body
        .topology
        .faces
        .iter()
        .find(|face| face.edge_ordinals.len() >= 4)
        .unwrap();
    let shared_edge_ordinals = [shared_face.edge_ordinals[0], shared_face.edge_ordinals[2]];

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let reference_for = |kind: TopologicalElementKind, ordinal: u32| {
        base_package
            .topological_references
            .iter()
            .filter(|reference| reference.kind == kind)
            .nth(ordinal as usize)
            .cloned()
            .unwrap()
    };
    let edge = reference_for(TopologicalElementKind::Edge, selected_edge.ordinal);
    let side_a = reference_for(TopologicalElementKind::Face, first_face);
    let side_b = reference_for(TopologicalElementKind::Face, second_face);
    let wrong_side = reference_for(TopologicalElementKind::Face, invalid_face);
    let mut shared_sides = shared_edge_ordinals
        .into_iter()
        .map(|ordinal| ChamferEdgeSide {
            edge: reference_for(TopologicalElementKind::Edge, ordinal),
            side_face: reference_for(TopologicalElementKind::Face, shared_face.ordinal),
        })
        .collect::<Vec<_>>();
    shared_sides.sort_unstable_by(|left, right| left.edge.cmp(&right.edge));

    let advanced = |edge: &TopologicalElementRef,
                    face: &TopologicalElementRef,
                    mode: ChamferMode| FeatureKind::TopologyEdgeFinish {
        target: base,
        edges: vec![edge.clone()],
        kind: EdgeFinishKind::Chamfer,
        amount: dimension(0.75),
        fillet_radius_stations: Vec::new(),
        chamfer_mode: mode,
        chamfer_edge_sides: vec![ChamferEdgeSide {
            edge: edge.clone(),
            side_face: face.clone(),
        }],
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: two_distance,
                definition_id: definition,
                name: "Two-distance chamfer".into(),
                kind: advanced(
                    &edge,
                    &side_a,
                    ChamferMode::TwoDistance {
                        second_distance: dimension(1.5),
                    },
                ),
            },
            CanonicalCommand::CreateFeature {
                id: reversed_side,
                definition_id: definition,
                name: "Reversed-side chamfer".into(),
                kind: advanced(
                    &edge,
                    &side_b,
                    ChamferMode::TwoDistance {
                        second_distance: dimension(1.5),
                    },
                ),
            },
            CanonicalCommand::CreateFeature {
                id: distance_angle,
                definition_id: definition,
                name: "Distance-angle chamfer".into(),
                kind: advanced(
                    &edge,
                    &side_a,
                    ChamferMode::DistanceAngle {
                        angle_degrees: 30.0,
                    },
                ),
            },
            CanonicalCommand::CreateFeature {
                id: shared_face_edges,
                definition_id: definition,
                name: "Shared-side-face chamfer".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: shared_sides
                        .iter()
                        .map(|selection| selection.edge.clone())
                        .collect(),
                    kind: EdgeFinishKind::Chamfer,
                    amount: dimension(0.25),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ChamferMode::TwoDistance {
                        second_distance: dimension(0.5),
                    },
                    chamfer_edge_sides: shared_sides,
                },
            },
            CanonicalCommand::CreateFeature {
                id: non_adjacent,
                definition_id: definition,
                name: "Non-adjacent side face".into(),
                kind: advanced(
                    &edge,
                    &wrong_side,
                    ChamferMode::TwoDistance {
                        second_distance: dimension(1.5),
                    },
                ),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let two_distance_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, two_distance).unwrap();
    let reversed_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, reversed_side).unwrap();
    let angle_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, distance_angle).unwrap();
    let shared_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, shared_face_edges).unwrap();
    let invalid_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, non_adjacent).unwrap();
    assert_eq!(two_distance_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    assert_eq!(reversed_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    assert_eq!(angle_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);
    assert_eq!(shared_graph.schema, EXACT_BREP_GRAPH_SCHEMA_V18);

    let two_distance_package = supervisor
        .evaluate_exact_brep_graph(&two_distance_graph)
        .unwrap();
    let reversed_package = supervisor
        .evaluate_exact_brep_graph(&reversed_graph)
        .unwrap();
    let angle_package = supervisor.evaluate_exact_brep_graph(&angle_graph).unwrap();
    let shared_package = supervisor.evaluate_exact_brep_graph(&shared_graph).unwrap();
    for package in [
        &two_distance_package,
        &reversed_package,
        &angle_package,
        &shared_package,
    ] {
        assert_eq!(package.topology_counts[4], 1);
        assert!(package.volume_mm3 < base_package.volume_mm3);
    }
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&two_distance_graph)
            .unwrap(),
        two_distance_package
    );
    assert_ne!(
        two_distance_package.identity.result_fingerprint,
        reversed_package.identity.result_fingerprint
    );
    assert_ne!(
        two_distance_package.identity.result_fingerprint,
        angle_package.identity.result_fingerprint
    );
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&invalid_graph)
            .unwrap_err(),
        "invalid_parameter",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_package
    );
}

#[test]
fn worker_rebinds_topology_selected_finishes_and_rejects_lost_provenance() {
    let definition = DefinitionId(8);
    let profile = FeatureId(700);
    let base = FeatureId(701);
    let shell = FeatureId(702);
    let fillet = FeatureId(703);
    let chamfer = FeatureId(704);
    let stale_finish = FeatureId(705);
    let noncanonical_shell = FeatureId(706);
    let duplicate_chamfer = FeatureId(707);
    let mixed_chamfer = FeatureId(708);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Topology-selected finishes".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Unequal rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let mut faces = base_package
        .topological_references
        .iter()
        .filter(|reference| {
            reference.kind == TopologicalElementKind::Face
                && reference.stability == TopologicalReferenceStability::Guaranteed
        })
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    faces.sort_unstable();
    let mut edges = base_package
        .topological_references
        .iter()
        .filter(|reference| {
            reference.kind == TopologicalElementKind::Edge
                && reference.stability == TopologicalReferenceStability::Guaranteed
        })
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    edges.sort_unstable();
    assert_eq!(faces.len(), 2);
    assert_eq!(edges.len(), 2);
    assert!(
        faces.iter().chain(&edges).all(|reference| {
            reference.stability == TopologicalReferenceStability::Guaranteed
                && !reference.source_element_id.contains("generated-source")
                && !reference.producer_element_id.contains("generated-result")
        }),
        "faces={faces:#?} edges={edges:#?}"
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: shell,
                definition_id: definition,
                name: "Selected-face shell".into(),
                kind: FeatureKind::TopologyShell {
                    target: base,
                    removed_faces: faces.clone(),
                    thickness: dimension(1.5),
                    direction: ketchup_core::document::ShellDirection::Inward,
                },
            },
            CanonicalCommand::CreateFeature {
                id: fillet,
                definition_id: definition,
                name: "Selected-edge fillet".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: vec![edges[0].clone()],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.75),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: chamfer,
                definition_id: definition,
                name: "Selected-edge chamfer".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: edges.clone(),
                    kind: EdgeFinishKind::Chamfer,
                    amount: dimension(0.75),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let shell_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, shell).unwrap();
    let fillet_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, fillet).unwrap();
    let chamfer_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, chamfer).unwrap();
    let shell_package = supervisor.evaluate_exact_brep_graph(&shell_graph).unwrap();
    let fillet_package = supervisor.evaluate_exact_brep_graph(&fillet_graph).unwrap();
    let chamfer_package = supervisor
        .evaluate_exact_brep_graph(&chamfer_graph)
        .unwrap();

    assert!(shell_package.volume_mm3 < base_package.volume_mm3);
    assert_eq!(shell_package.topology_counts[4], 1);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&shell_graph).unwrap(),
        shell_package
    );
    assert_eq!(fillet_package.topology_counts[4], 1);
    assert_eq!(chamfer_package.topology_counts[4], 1);
    assert_ne!(
        fillet_package.identity.result_fingerprint,
        chamfer_package.identity.result_fingerprint
    );
    let (shell_selectors, shell_thickness_bits) = shell_graph
        .nodes
        .iter()
        .find_map(|node| match &node.operation {
            ExactBRepOperation::Shell {
                removed_faces,
                thickness_bits,
                ..
            } => Some((removed_faces.clone(), *thickness_bits)),
            _ => None,
        })
        .unwrap();
    let (chamfer_selectors, chamfer_kind, chamfer_amount_bits) = chamfer_graph
        .nodes
        .iter()
        .find_map(|node| match &node.operation {
            ExactBRepOperation::EdgeFinish {
                edges,
                kind,
                amount_bits,
                ..
            } => Some((edges.clone(), *kind, *amount_bits)),
            _ => None,
        })
        .unwrap();
    assert_eq!(shell_selectors.len(), 2);
    assert_eq!(shell_thickness_bits, 1.5_f64.to_bits());
    assert_eq!(chamfer_selectors.len(), 2);
    assert_eq!(chamfer_amount_bits, 0.75_f64.to_bits());

    let registry = ExactResultRegistry::accept(
        &snapshot,
        [Arc::new(ExactBodyPackage::Graph(shell_package.clone()))],
    )
    .unwrap();
    let shell_result_key = ExactBodyPackage::Graph(shell_package.clone()).result_key();
    assert!(registry.get_result(&shell_result_key).is_some());

    let accepted_revision = snapshot.revision_id();
    let accepted_digest = snapshot.canonical_digest();
    let mut reversed_faces = faces.clone();
    reversed_faces.reverse();
    assert_ne!(reversed_faces, faces);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: noncanonical_shell,
            definition_id: definition,
            name: "Noncanonical selected-face shell".into(),
            kind: FeatureKind::TopologyShell {
                target: base,
                removed_faces: reversed_faces,
                thickness: dimension(1.5),
                direction: ketchup_core::document::ShellDirection::Inward,
            },
        }])),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: duplicate_chamfer,
            definition_id: definition,
            name: "Duplicate selected-edge chamfer".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: base,
                edges: vec![edges[0].clone(), edges[0].clone()],
                kind: EdgeFinishKind::Chamfer,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        }])),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: mixed_chamfer,
            definition_id: definition,
            name: "Mixed selected-edge chamfer".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: base,
                edges: vec![faces[0].clone(), edges[0].clone()],
                kind: EdgeFinishKind::Chamfer,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        }])),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    ));
    let refused_snapshot = document.current();
    assert_eq!(refused_snapshot.revision_id(), accepted_revision);
    assert_eq!(refused_snapshot.canonical_digest(), accepted_digest);
    let retained_registry = ExactResultRegistry::carried_forward(&refused_snapshot, &registry);
    assert!(retained_registry.get_result(&shell_result_key).is_some());
    assert_eq!(retained_registry.len(), 1);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: base,
                dimension: dimension(27.0),
            },
            CanonicalCommand::SetFeatureDimension {
                id: shell,
                dimension: dimension(1.25),
            },
            CanonicalCommand::SetFeatureDimension {
                id: chamfer,
                dimension: dimension(0.5),
            },
        ]))
        .unwrap();
    let recomputed_snapshot = document.current();
    assert!(!shell_package.is_current(&recomputed_snapshot));
    assert!(!chamfer_package.is_current(&recomputed_snapshot));
    let recomputed_shell_graph =
        ExactBRepGraph::from_snapshot(&recomputed_snapshot, definition, shell).unwrap();
    let recomputed_fillet_graph =
        ExactBRepGraph::from_snapshot(&recomputed_snapshot, definition, fillet).unwrap();
    let recomputed_chamfer_graph =
        ExactBRepGraph::from_snapshot(&recomputed_snapshot, definition, chamfer).unwrap();
    assert!(recomputed_shell_graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::Shell { removed_faces, thickness_bits, .. }
            if removed_faces == &shell_selectors && *thickness_bits == 1.25_f64.to_bits()
    )));
    assert!(recomputed_chamfer_graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::EdgeFinish { edges, kind, amount_bits, .. }
            if edges == &chamfer_selectors
                && *kind == chamfer_kind
                && *amount_bits == 0.5_f64.to_bits()
    )));
    assert_ne!(
        recomputed_shell_graph.graph_digest,
        shell_graph.graph_digest
    );
    assert_ne!(
        recomputed_chamfer_graph.graph_digest,
        chamfer_graph.graph_digest
    );
    let recomputed_shell_package = supervisor
        .evaluate_exact_brep_graph(&recomputed_shell_graph)
        .unwrap();
    let recomputed_fillet_package = supervisor
        .evaluate_exact_brep_graph(&recomputed_fillet_graph)
        .unwrap();
    let recomputed_chamfer_package = supervisor
        .evaluate_exact_brep_graph(&recomputed_chamfer_graph)
        .unwrap();
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&recomputed_shell_graph)
            .unwrap(),
        recomputed_shell_package
    );
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&recomputed_chamfer_graph)
            .unwrap(),
        recomputed_chamfer_package
    );
    assert_ne!(
        recomputed_shell_package.identity.result_fingerprint,
        shell_package.identity.result_fingerprint
    );
    assert_ne!(
        recomputed_fillet_package.identity.result_fingerprint,
        fillet_package.identity.result_fingerprint
    );
    assert_ne!(
        recomputed_chamfer_package.identity.result_fingerprint,
        chamfer_package.identity.result_fingerprint
    );
    ExactResultRegistry::accept(
        &recomputed_snapshot,
        [
            recomputed_shell_package,
            recomputed_fillet_package,
            recomputed_chamfer_package,
        ]
        .map(ExactBodyPackage::Graph)
        .map(Arc::new),
    )
    .unwrap();

    let edge = &edges[0];
    let stale_edge = TopologicalElementRef::new(
        edge.document_id,
        edge.definition_id,
        edge.source_feature_id,
        edge.producer_feature_id,
        edge.kind,
        "lost-source-provenance",
        edge.producer_element_id.clone(),
        edge.stability,
        edge.evaluator.clone(),
        edge.backend.clone(),
        edge.tolerance.clone(),
        "stale-result-fingerprint",
        edge.corroborating_geometry_fingerprint.clone(),
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: stale_finish,
            definition_id: definition,
            name: "Stale selected edge".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: base,
                edges: vec![stale_edge],
                kind: EdgeFinishKind::Fillet,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_core::document::ChamferMode::Symmetric,
                chamfer_edge_sides: Vec::new(),
            },
        }]))
        .unwrap();
    let stale_graph =
        ExactBRepGraph::from_snapshot(&document.current(), definition, stale_finish).unwrap();
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&stale_graph)
            .unwrap_err(),
        "invalid_parameter",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_package
    );
}

#[test]
fn worker_preserves_cubic_sketch_region_hole_volume_and_result_identity() {
    let definition = DefinitionId(92);
    let workplane = FeatureId(920);
    let sketch_id = FeatureId(921);
    let pad = FeatureId(922);
    let sketch = SketchSpec {
        workplane,
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
    };
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].holes.len(), 1);
    let region = regions[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Worker compound region".into(),
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
                name: "Line-cubic profile with centered hole".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Compound Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(12.0)),
                }),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pad).unwrap();
    let ExactBRepPlanarGeometry::Region { outer, holes } = &graph.profiles[0].geometry else {
        panic!("compound sketch must reach the worker as a planar region");
    };
    assert!(matches!(
        outer,
        ExactBRepPlanarLoop::Boundary { segments } if segments.len() == 4
    ));
    assert_eq!(holes.len(), 1);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, pad.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_eq!(
        package.edge_evidence.len(),
        package.topology_counts[1] as usize
    );
    let circular_rims = package
        .edge_evidence
        .iter()
        .filter(|edge| {
            edge.curve_kind == "circle"
                && edge.circle_radius_mm == Some(5.0)
                && edge
                    .axis_origin_mm
                    .is_some_and(|origin| origin[0] == 0.0 && origin[1] == 0.0)
                && edge.unit_axis_direction == Some([0.0, 0.0, 1.0])
        })
        .collect::<Vec<_>>();
    assert_eq!(circular_rims.len(), 2);
    for expected_z in [0.0, 12.0] {
        assert!(
            circular_rims
                .iter()
                .any(|edge| (edge.centroid_mm[2] - expected_z).abs() <= 1.0e-9)
        );
    }
    for edge in circular_rims {
        assert!(edge.closed);
        assert_eq!(edge.adjacent_face_ordinals.len(), 2);
        let reference = package
            .topological_references
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Edge)
            .nth(edge.edge_ordinal as usize)
            .expect("every edge evidence ordinal has a stable reference");
        assert!(reference.has_valid_lineage());
        assert_eq!(reference.producer_feature_id, pad);
    }
    assert_bounds_close(package.bounds_mm, [-20.0, -15.0, 0.0, 20.0, 22.5, 12.0]);
    let expected_volume = (1_410.0 - std::f64::consts::PI * 5.0 * 5.0) * 12.0;
    assert!(package.volume_mm3 < 1_410.0 * 12.0);
    assert!(
        (package.volume_mm3 - expected_volume).abs() <= 2.0e-7,
        "worker volume {} differed from analytic compound-region volume {expected_volume}",
        package.volume_mm3
    );

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, pad).unwrap();
    assert_eq!(reopened_graph, graph);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap(),
        package
    );
    assert!(package.is_current(&reopened_snapshot));
}

#[test]
fn worker_cuts_a_compound_sketch_pocket_while_preserving_its_inner_island() {
    let definition = DefinitionId(93);
    let base_plane = FeatureId(929);
    let base_sketch_id = FeatureId(930);
    let base = FeatureId(931);
    let face_plane = FeatureId(932);
    let pocket_sketch_id = FeatureId(933);
    let pocket = FeatureId(934);
    let base_sketch = SketchSpec {
        workplane: base_plane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [50.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [50.0, 0.0],
                end_mm: [50.0, 40.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [50.0, 40.0],
                end_mm: [0.0, 40.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [0.0, 40.0],
                end_mm: [0.0, 0.0],
            },
        ],
        constraints: Vec::new(),
    };
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Compound pocket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_plane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: base_sketch_id,
                definition_id: definition,
                name: "Base sketch".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: base_sketch_id,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                }),
            },
        ]))
        .unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&document.current(), definition, base)
            .unwrap();
    let base_package = supervisor.evaluate_rectangle(&base_request).unwrap();
    let top = base_package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in base_package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    let pocket_sketch = SketchSpec {
        workplane: face_plane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [5.0, 5.0],
                end_mm: [45.0, 5.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [45.0, 5.0],
                end_mm: [45.0, 35.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [45.0, 35.0],
                end_mm: [5.0, 35.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [5.0, 35.0],
                end_mm: [5.0, 5.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(5),
                center_mm: [25.0, 20.0],
                radius_mm: 5.0,
            },
        ],
        constraints: Vec::new(),
    };
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: face_plane,
                definition_id: definition,
                name: "Base top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [0.0, 0.0, 10.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_sketch_id,
                definition_id: definition,
                name: "Pocket region".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: definition,
                name: "Compound Pocket".into(),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target: base,
                    sketch: pocket_sketch_id,
                    region: pocket_region,
                    support: Box::new(top),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(dimension(4.0)),
                }),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pocket).unwrap();
    assert!(matches!(
        graph.profiles.last().map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Region { holes, .. }) if holes.len() == 1
    ));
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let removed_volume = (40.0 * 30.0 - std::f64::consts::PI * 5.0 * 5.0) * 4.0;
    let expected_volume = 50.0 * 40.0 * 10.0 - removed_volume;
    assert!(
        (result.volume_mm3 - expected_volume).abs() <= 2.0e-7,
        "compound pocket volume {} differed from {expected_volume}",
        result.volume_mm3
    );
    assert_bounds_close(result.bounds_mm, [0.0, 0.0, 0.0, 50.0, 40.0, 10.0]);
    assert_eq!(result.topology_counts[4], 1);
    assert_eq!(result.identity.producer_feature_id.0, pocket.0);
}

#[test]
fn worker_evaluates_v11_cubic_sweep_with_mesh_and_step_round_trip() {
    let definition = DefinitionId(800);
    let profile = FeatureId(801);
    let path = FeatureId(802);
    let sweep = FeatureId(803);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "V11 cubic sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Small rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-cubic-line C1 path".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
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
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "V11 sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, sweep).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V11);
    let mut downgraded = graph.clone();
    downgraded.schema = EXACT_BREP_GRAPH_SCHEMA_V10.to_owned();
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
    assert!(package.topology_counts.iter().all(|count| *count > 0));
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    assert_eq!(
        package.triangles.len(),
        package.triangle_face_ordinals.len()
    );

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("v11-cubic-sweep.step");
    supervisor
        .export_exact_brep_graph_step(&snapshot, &package, &step_path)
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
fn worker_evaluates_closed_non_planar_v12_sweep_with_mesh_and_step_round_trip() {
    let definition = DefinitionId(810);
    let profile = FeatureId(811);
    let path = FeatureId(812);
    let sweep = FeatureId(813);
    let spatial_segments = vec![
        SpatialPathSegment::CubicBezier {
            start_mm: [30.0, 0.0, 0.0],
            control_1_mm: [30.0, 15.0, 10.0],
            control_2_mm: [15.0, 30.0, 10.0],
            end_mm: [0.0, 30.0, 0.0],
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [0.0, 30.0, 0.0],
            control_1_mm: [-15.0, 30.0, -10.0],
            control_2_mm: [-30.0, 15.0, -10.0],
            end_mm: [-30.0, 0.0, 0.0],
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [-30.0, 0.0, 0.0],
            control_1_mm: [-30.0, -15.0, 10.0],
            control_2_mm: [-15.0, -30.0, 10.0],
            end_mm: [0.0, -30.0, 0.0],
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [0.0, -30.0, 0.0],
            control_1_mm: [15.0, -30.0, -10.0],
            control_2_mm: [30.0, -15.0, -10.0],
            end_mm: [30.0, 0.0, 0.0],
        },
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "V12 spatial sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Spatial section".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Closed non-planar path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: spatial_segments.clone(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Spatial sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, sweep).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V12);
    assert!(matches!(
        &graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == spatial_segments.len()
    ));
    let mut downgraded = graph.clone();
    downgraded.schema = EXACT_BREP_GRAPH_SCHEMA_V11.to_owned();
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
    assert!(package.bounds_mm[1][2] > package.bounds_mm[0][2]);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("v12-closed-spatial-sweep.step");
    supervisor
        .export_exact_brep_graph_step(&snapshot, &package, &step_path)
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
fn worker_evaluates_general_sketch_profiles_on_a_mixed_spatial_path() {
    let spatial_segments = vec![
        SpatialPathSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [0.0, 0.0, 20.0],
        },
        SpatialPathSegment::CircularArc {
            start_mm: [0.0, 0.0, 20.0],
            end_mm: [10.0, 0.0, 30.0],
            center_mm: [10.0, 0.0, 20.0],
            normal: [0.0, 1.0, 0.0],
            clockwise: false,
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [10.0, 0.0, 30.0],
            control_1_mm: [15.0, 0.0, 30.0],
            control_2_mm: [20.0, 5.0, 35.0],
            end_mm: [25.0, 10.0, 40.0],
        },
    ];
    let profiles = [
        vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-2.0, -2.0],
                end_mm: [2.0, -2.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [2.0, -2.0],
                end_mm: [2.0, 2.0],
                center_mm: [2.0, 0.0],
                clockwise: false,
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [2.0, 2.0],
                control_1_mm: [0.5, 3.0],
                control_2_mm: [-0.5, 3.0],
                end_mm: [-2.0, 2.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-2.0, 2.0],
                end_mm: [-2.0, -2.0],
            },
        ],
        vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: 2.0,
        }],
        vec![
            SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [0.0, 0.0],
                radius_mm: 2.0,
            },
            SketchEntity::Circle {
                id: SketchEntityId(2),
                center_mm: [0.0, 0.0],
                radius_mm: 1.0,
            },
        ],
    ];
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut volumes = Vec::new();
    for (index, entities) in profiles.into_iter().enumerate() {
        let definition = DefinitionId(900 + index as u64);
        let workplane = FeatureId(9000 + index as u64 * 10);
        let profile = FeatureId(workplane.0 + 1);
        let path = FeatureId(workplane.0 + 2);
        let sweep = FeatureId(workplane.0 + 3);
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: format!("General sketch sweep {index}"),
                },
                CanonicalCommand::CreateFeature {
                    id: workplane,
                    definition_id: definition,
                    name: "Profile plane".into(),
                    kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "General sketch profile".into(),
                    kind: FeatureKind::Sketch(SketchSpec {
                        workplane,
                        entities,
                        constraints: Vec::new(),
                    }),
                },
                CanonicalCommand::CreateFeature {
                    id: path,
                    definition_id: definition,
                    name: "Mixed spatial path".into(),
                    kind: FeatureKind::SpatialPath {
                        segments: spatial_segments.clone(),
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: sweep,
                    definition_id: definition,
                    name: "General sketch sweep".into(),
                    kind: FeatureKind::Sweep { profile, path },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, sweep).unwrap();
        assert!(matches!(
            &graph.nodes.last().unwrap().operation,
            ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
        ));
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(package.topology_counts[4], 1);
        assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);
        volumes.push(package.volume_mm3);
        assert!(!package.vertices.is_empty());
        assert!(!package.triangles.is_empty());
        if index == 0 {
            let directory = tempfile::tempdir().unwrap();
            let step_path = directory.path().join("general-sketch-spatial-sweep.step");
            supervisor
                .export_exact_brep_graph_step(&snapshot, &package, &step_path)
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
            assert!(
                (evidence.volume_mm3 - package.volume_mm3).abs() <= package.volume_mm3 * 1.0e-9
            );
        }
    }
    assert!(volumes[2] < volumes[1] * 0.76);
    assert!(volumes[2] > volumes[1] * 0.74);

    let definition = DefinitionId(950);
    let profile_plane = FeatureId(9500);
    let profile = FeatureId(9501);
    let path_plane = FeatureId(9502);
    let path = FeatureId(9503);
    let sweep = FeatureId(9504);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Sketch path worker proof".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_plane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Circle profile".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: profile_plane,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [0.0, 0.0],
                        radius_mm: 2.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: path_plane,
                definition_id: definition,
                name: "Path plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xz)),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Mixed Sketch path".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: path_plane,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            end_mm: [0.0, 20.0],
                        },
                        SketchEntity::Arc {
                            id: SketchEntityId(2),
                            start_mm: [0.0, 20.0],
                            end_mm: [10.0, 30.0],
                            center_mm: [10.0, 20.0],
                            clockwise: true,
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [10.0, 30.0],
                            control_1_mm: [15.0, 30.0],
                            control_2_mm: [20.0, 35.0],
                            end_mm: [25.0, 40.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Sketch path Sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, sweep).unwrap();
    assert!(matches!(
        &graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    ));
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3.is_finite() && package.volume_mm3 > 0.0);
}
