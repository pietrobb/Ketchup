use ketchup_analysis::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_geometry::sketch::{CutStart, PadOperation, PadProfile};
use ketchup_geometry::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, PadSpec, PrincipalPlane, SketchEntity,
    SketchEntityId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_model::cam::{
    CamCollisionParticipant, CamCollisionTarget, CamCutParameters, CamFixture, CamMotionKind,
    CamOperation, CamPath2d, CamPathSegment2d, CamPlan, CamPlanId, CamPostprocessorDialect,
    CamPostprocessorError, CamPostprocessorOutput, CamSetup, CamStock, CamTool, CamToolKind,
    CamToolpath, CamWorkOffset,
};
use ketchup_model::document::{
    BodyKind, BooleanOperation, CanonicalCommand, CanonicalError, ChamferEdgeSide, ChamferMode,
    CommandBatch, DefinitionId, Dimension, DocumentStore, DraftSpec, EdgeFinishKind, EdgeRef,
    FaceRef, FeatureId, FeatureKind, FeatureParameterTarget, FilletRadiusStation, InstancePath,
    LoftContinuity, LoftSection, OccurrenceId, ParameterValueType, ProfileSegment, ShellDirection,
    Snapshot, SolidToolPlan, SpatialPathSegment, SurfaceBodySpec, Transform,
};
use ketchup_model::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V8, EXACT_BREP_GRAPH_SCHEMA_V9, EXACT_BREP_GRAPH_SCHEMA_V10,
    EXACT_BREP_GRAPH_SCHEMA_V11, EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V14,
    EXACT_BREP_GRAPH_SCHEMA_V15, EXACT_BREP_GRAPH_SCHEMA_V17, EXACT_BREP_GRAPH_SCHEMA_V18,
    EXACT_BREP_GRAPH_SCHEMA_V20, EXACT_BREP_GRAPH_SCHEMA_V21, EXACT_BREP_GRAPH_SCHEMA_V22,
    EXACT_BREP_GRAPH_SCHEMA_V23, ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation,
    ExactBRepPlanarGeometry, ExactBRepPlanarLoop, ExactBRepPlanarSegment,
    MAX_EXACT_BREP_GRAPH_PROFILES,
};
use ketchup_model::exact_product::{
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactProductError, ExactResultRegistry,
};
use ketchup_model::graph::sha256_hex;
use ketchup_model::import::{StepImportMesh, StepMeshTriangle, plan_iges_import, plan_step_import};
use ketchup_model::persistence;
use ketchup_model::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};
use ketchup_scheduler::{
    ExactFeaFaceTraction, ExactFeaSetup, ExactFeaSetupError, ExactVolumeMeshWireOptions,
    ExactWorkerSupervisor, WorkerError,
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [8.0, 0.0], [8.0, 6.0], [0.0, 6.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: definition,
                name: "Safety extrusion".into(),
                kind: FeatureKind::extrusion(profile, dimension(5.0)),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Safety occurrence".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "20x10x5 target".into(),
                kind: FeatureKind::extrusion(profile, dimension(5.0)),
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
            kind: FeatureKind::polygon(&base_points),
        },
        CanonicalCommand::CreateFeature {
            id: base,
            definition_id: definition,
            name: "Generated target body".into(),
            kind: FeatureKind::extrusion(base_profile, dimension(13.0 * scale_z)),
        },
        CanonicalCommand::CreateFeature {
            id: tool_profile,
            definition_id: definition,
            name: "Generated tool profile".into(),
            kind: FeatureKind::polygon(&tool_points),
        },
        CanonicalCommand::CreateFeature {
            id: tool,
            definition_id: definition,
            name: "Generated tool body".into(),
            kind: FeatureKind::extrusion(tool_profile, dimension(19.0 * scale_z)),
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

mod exchange_booleans_and_lofts;
mod sketch_sweeps_and_drafts;
mod surfaces_offsets_and_finishes;
