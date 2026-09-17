use ketchup_core::cam::{
    CAM_POSTPROCESSOR_SCHEMA_V1, CAM_SIMULATION_SCHEMA_V1, CamCutParameters, CamError,
    CamMotionKind, CamMotionPath, CamOperation, CamOperationKind, CamPath2d, CamPathSegment2d,
    CamPlan, CamPlanHealth, CamPlanId, CamPlannerError, CamPostprocessedMotionKind,
    CamPostprocessorDialect, CamPostprocessorError, CamPostprocessorOutput, CamSetup,
    CamSimulationEvidence, CamStock, CamTool, CamToolKind, CamToolpath, CamWorkOffset,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, SurfaceBodySpec,
};
use ketchup_core::persistence;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(10);
const SOLID: FeatureId = FeatureId(11);
const PLAN: CamPlanId = CamPlanId(1);

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn solid_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Machined plate".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Stock-facing profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: SOLID,
                definition_id: DEFINITION,
                name: "Target solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: dimension(10.0),
                },
            },
        ]))
        .unwrap();
    document
}

fn plan(snapshot: &ketchup_core::document::Snapshot) -> Result<CamPlan, CamError> {
    CamPlan::new(
        snapshot,
        PLAN,
        "Top setup",
        CamStock {
            minimum_mm: [-2.0, -2.0, -1.0],
            maximum_mm: [52.0, 32.0, 12.0],
        },
        CamTool {
            number: 1,
            kind: CamToolKind::FlatEndMill,
            diameter_mm: 6.0,
            flute_length_mm: 18.0,
            overall_length_mm: 50.0,
            holder_diameter_mm: 20.0,
            holder_length_mm: 35.0,
            spindle_rpm: 12_000,
            feed_mm_per_min: 900.0,
            plunge_mm_per_min: 250.0,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 12.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 8.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.45,
            radial_allowance_mm: 0.2,
            axial_allowance_mm: 0.1,
        },
        DEFINITION,
        SOLID,
    )
}

#[test]
fn canonical_cam_plan_is_persisted_deterministic_and_undoable() {
    let mut document = solid_document();
    let before = document.current().canonical_digest();
    let canonical = plan(&document.current()).unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
            canonical.clone(),
        )]))
        .unwrap();
    let committed = document.current();
    assert_eq!(committed.cam_plan(PLAN), Some(&canonical));
    assert_eq!(canonical.health(&committed), CamPlanHealth::Current);

    let bytes = persistence::save(&committed);
    assert_eq!(
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    assert_eq!(reopened.snapshot().cam_plan(PLAN), Some(&canonical));
    assert_eq!(
        canonical.health(&reopened.snapshot()),
        CamPlanHealth::Current
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert!(document.current().cam_plan(PLAN).is_none());
    assert_eq!(document.redo().unwrap().cam_plan(PLAN), Some(&canonical));
}

#[test]
fn target_change_marks_plan_stale_without_blocking_model_edit_and_undo_restores_it() {
    let mut document = solid_document();
    let canonical = plan(&document.current()).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
            canonical,
        )]))
        .unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: SOLID,
                dimension: dimension(11.0),
            },
        ]))
        .unwrap();
    assert_eq!(
        document
            .current()
            .cam_plan(PLAN)
            .unwrap()
            .health(&document.current()),
        CamPlanHealth::StaleTarget
    );
    let reopened_stale = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .snapshot();
    assert_eq!(
        reopened_stale
            .cam_plan(PLAN)
            .unwrap()
            .health(&reopened_stale),
        CamPlanHealth::StaleTarget
    );

    let restored = document.undo().unwrap();
    assert_eq!(
        restored.cam_plan(PLAN).unwrap().health(&restored),
        CamPlanHealth::Current
    );
    let stale_again = document.redo().unwrap();
    assert_eq!(
        stale_again.cam_plan(PLAN).unwrap().health(&stale_again),
        CamPlanHealth::StaleTarget
    );
}

#[test]
fn surface_target_and_undersized_stock_are_refused_without_mutation() {
    let mut document = solid_document();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(20),
            definition_id: DEFINITION,
            name: "Planar surface".into(),
            kind: FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile: PROFILE }),
        }]))
        .unwrap();
    let surface = CamPlan::new(
        &document.current(),
        PLAN,
        "Invalid surface setup",
        CamStock {
            minimum_mm: [-2.0, -2.0, -1.0],
            maximum_mm: [52.0, 32.0, 12.0],
        },
        plan(&document.current()).unwrap().tool().clone(),
        plan(&document.current()).unwrap().setup().clone(),
        plan(&document.current()).unwrap().cut_parameters().clone(),
        DEFINITION,
        FeatureId(20),
    );
    assert_eq!(surface, Err(CamError::WrongBodyKind));

    let undersized = CamPlan::new(
        &document.current(),
        PLAN,
        "Undersized stock",
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [49.0, 30.0, 10.0],
        },
        plan(&document.current()).unwrap().tool().clone(),
        plan(&document.current()).unwrap().setup().clone(),
        plan(&document.current()).unwrap().cut_parameters().clone(),
        DEFINITION,
        SOLID,
    );
    assert_eq!(undersized, Err(CamError::StockDoesNotContainTarget));

    let mut invalid_cut = plan(&document.current()).unwrap().cut_parameters().clone();
    invalid_cut.stepover_ratio = 1.2;
    let invalid_parameters = CamPlan::new(
        &document.current(),
        PLAN,
        "Invalid parameters",
        CamStock {
            minimum_mm: [-2.0, -2.0, -1.0],
            maximum_mm: [52.0, 32.0, 12.0],
        },
        plan(&document.current()).unwrap().tool().clone(),
        plan(&document.current()).unwrap().setup().clone(),
        invalid_cut,
        DEFINITION,
        SOLID,
    );
    assert_eq!(invalid_parameters, Err(CamError::InvalidPlan));
    assert!(document.current().cam_plans().next().is_none());
}

#[test]
fn mixed_valid_and_invalid_cam_batch_is_atomic() {
    let mut document = solid_document();
    let before_revision = document.current().revision_id();
    let before_digest = document.current().canonical_digest();
    let result = document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::UpsertCamPlan(plan(&document.current()).unwrap()),
        CanonicalCommand::DeleteCamPlan { id: CamPlanId(999) },
    ]));
    let Err(error) = result else {
        panic!("mixed CAM batch must fail atomically");
    };
    assert_eq!(error, CanonicalError::CamPlanNotFound(CamPlanId(999)));
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert!(document.current().cam_plan(PLAN).is_none());
}

fn practical_operations() -> Vec<CamOperation> {
    vec![
        CamOperation::Face {
            id: 1,
            minimum_mm: [1.0, 1.0],
            maximum_mm: [49.0, 29.0],
            target_z_mm: -2.0,
        },
        CamOperation::Pocket {
            id: 2,
            minimum_mm: [8.0, 7.0],
            maximum_mm: [32.0, 23.0],
            top_z_mm: -2.0,
            bottom_z_mm: -6.0,
        },
        CamOperation::Contour {
            id: 3,
            center_path: CamPath2d {
                start_mm: [25.0, 10.0],
                segments: vec![
                    CamPathSegment2d::Arc {
                        to_mm: [25.0, 20.0],
                        center_mm: [25.0, 15.0],
                        clockwise: false,
                    },
                    CamPathSegment2d::Arc {
                        to_mm: [25.0, 10.0],
                        center_mm: [25.0, 15.0],
                        clockwise: false,
                    },
                ],
            },
            top_z_mm: -2.0,
            bottom_z_mm: -8.0,
            applied_radial_allowance_mm: 0.2,
        },
        CamOperation::Drill {
            id: 4,
            points_mm: vec![[10.0, 10.0], [40.0, 20.0]],
            top_z_mm: -2.0,
            bottom_z_mm: -9.0,
        },
    ]
}

#[test]
fn machine_neutral_2_5d_planner_is_deterministic_safe_and_complete() {
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let operations = practical_operations();

    let first = CamToolpath::plan(&snapshot, &canonical, &operations).unwrap();
    let second = CamToolpath::plan(&snapshot, &canonical, &operations).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.plan_digest, canonical.stable_digest());
    assert_eq!(first.toolpath_digest.len(), 64);
    assert_eq!(
        first
            .operations
            .iter()
            .map(|operation| operation.kind)
            .collect::<Vec<_>>(),
        vec![
            CamOperationKind::Face,
            CamOperationKind::Pocket,
            CamOperationKind::Contour,
            CamOperationKind::Drill,
        ]
    );
    assert!(
        first
            .operations
            .iter()
            .all(|operation| operation.motion_count > 0)
    );
    for operation in &first.operations {
        assert!(operation.first_motion + operation.motion_count <= first.motions.len());
    }

    for expected in [
        CamMotionKind::Rapid,
        CamMotionKind::Plunge,
        CamMotionKind::Cut,
        CamMotionKind::Retract,
    ] {
        assert!(first.motions.iter().any(|motion| motion.kind == expected));
    }
    assert!(
        first
            .motions
            .iter()
            .any(|motion| matches!(motion.path, CamMotionPath::Arc { .. }))
    );
    assert!(first.motions.iter().all(|motion| match &motion.path {
        CamMotionPath::Line { start_mm, end_mm }
        | CamMotionPath::Arc {
            start_mm, end_mm, ..
        } => match motion.kind {
            CamMotionKind::Rapid => start_mm[2] == 8.0 && end_mm[2] == 8.0,
            CamMotionKind::Retract => end_mm[2] == 8.0,
            CamMotionKind::Plunge | CamMotionKind::Cut => end_mm[2] < 0.0,
        },
    }));
    assert!(first.motions.iter().any(|motion| match &motion.path {
        CamMotionPath::Line { end_mm, .. } | CamMotionPath::Arc { end_mm, .. } => {
            (end_mm[2] - -1.9).abs() < 1.0e-9
        }
    }));
}

#[test]
fn planner_refuses_stale_or_malformed_geometry_without_document_mutation() {
    let mut document = solid_document();
    let canonical = plan(&document.current()).unwrap();
    let revision = document.current().revision_id();
    let digest = document.current().canonical_digest();

    let duplicate_ids = vec![
        CamOperation::Drill {
            id: 7,
            points_mm: vec![[10.0, 10.0]],
            top_z_mm: -2.0,
            bottom_z_mm: -4.0,
        },
        CamOperation::Drill {
            id: 7,
            points_mm: vec![[20.0, 10.0]],
            top_z_mm: -2.0,
            bottom_z_mm: -4.0,
        },
    ];
    assert_eq!(
        CamToolpath::plan(&document.current(), &canonical, &duplicate_ids),
        Err(CamPlannerError::InvalidOperations)
    );

    let outside_target = [CamOperation::Pocket {
        id: 8,
        minimum_mm: [45.0, 5.0],
        maximum_mm: [55.0, 20.0],
        top_z_mm: -2.0,
        bottom_z_mm: -4.0,
    }];
    assert_eq!(
        CamToolpath::plan(&document.current(), &canonical, &outside_target),
        Err(CamPlannerError::GeometryOutsideTarget)
    );

    let open_contour = [CamOperation::Contour {
        id: 9,
        center_path: CamPath2d {
            start_mm: [10.0, 10.0],
            segments: vec![CamPathSegment2d::Line {
                to_mm: [20.0, 10.0],
            }],
        },
        top_z_mm: -2.0,
        bottom_z_mm: -4.0,
        applied_radial_allowance_mm: 0.2,
    }];
    assert_eq!(
        CamToolpath::plan(&document.current(), &canonical, &open_contour),
        Err(CamPlannerError::InvalidOperations)
    );
    assert_eq!(document.current().revision_id(), revision);
    assert_eq!(document.current().canonical_digest(), digest);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: SOLID,
                dimension: dimension(11.0),
            },
        ]))
        .unwrap();
    assert_eq!(
        CamToolpath::plan(&document.current(), &canonical, &practical_operations()),
        Err(CamPlannerError::Plan(CamError::StaleTarget))
    );
}

#[test]
fn planner_refuses_unsafe_clearance_stock_sweep_flute_and_tool_kind() {
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let face = [CamOperation::Face {
        id: 1,
        minimum_mm: [1.0, 1.0],
        maximum_mm: [49.0, 29.0],
        target_z_mm: -2.0,
    }];

    let mut unsafe_setup = canonical.setup().clone();
    unsafe_setup.origin_mm[2] = 11.0;
    unsafe_setup.safe_height_mm = 0.1;
    let unsafe_plan = CamPlan::new(
        &snapshot,
        CamPlanId(2),
        "Unsafe clearance",
        canonical.stock().clone(),
        canonical.tool().clone(),
        unsafe_setup,
        canonical.cut_parameters().clone(),
        DEFINITION,
        SOLID,
    )
    .unwrap();
    assert_eq!(
        CamToolpath::plan(&snapshot, &unsafe_plan, &face),
        Err(CamPlannerError::UnsafeRapidHeight)
    );

    let outside_stock = [CamOperation::Face {
        id: 2,
        minimum_mm: [0.0, 0.0],
        maximum_mm: [50.0, 30.0],
        target_z_mm: -2.0,
    }];
    assert_eq!(
        CamToolpath::plan(&snapshot, &canonical, &outside_stock),
        Err(CamPlannerError::ToolOutsideStock)
    );

    let mut short_tool = canonical.tool().clone();
    short_tool.flute_length_mm = 1.0;
    let short_plan = CamPlan::new(
        &snapshot,
        CamPlanId(3),
        "Short flute",
        canonical.stock().clone(),
        short_tool,
        canonical.setup().clone(),
        canonical.cut_parameters().clone(),
        DEFINITION,
        SOLID,
    )
    .unwrap();
    assert_eq!(
        CamToolpath::plan(&snapshot, &short_plan, &face),
        Err(CamPlannerError::CuttingDepthExceedsFlute)
    );

    let mut drill = canonical.tool().clone();
    drill.kind = CamToolKind::Drill;
    let drill_plan = CamPlan::new(
        &snapshot,
        CamPlanId(4),
        "Drill-only setup",
        canonical.stock().clone(),
        drill,
        canonical.setup().clone(),
        canonical.cut_parameters().clone(),
        DEFINITION,
        SOLID,
    )
    .unwrap();
    assert_eq!(
        CamToolpath::plan(&snapshot, &drill_plan, &face),
        Err(CamPlannerError::UnsupportedTool)
    );
}

fn successful_simulation(plan: &CamPlan, toolpath: &CamToolpath) -> CamSimulationEvidence {
    let mut evidence = CamSimulationEvidence {
        schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
        plan_digest: plan.stable_digest(),
        toolpath_digest: toolpath.toolpath_digest.clone(),
        target_exact_graph_digest: toolpath.target_exact_graph_digest.clone(),
        motion_count: toolpath.motions.len(),
        cutting_motion_count: toolpath
            .motions
            .iter()
            .filter(|motion| matches!(motion.kind, CamMotionKind::Plunge | CamMotionKind::Cut))
            .count(),
        fixture_count: 0,
        stock_before_mm3: 1_000.0,
        stock_after_mm3: 800.0,
        removed_stock_mm3: 200.0,
        residual_stock_mm3: 0.0,
        gouge_mm3: 0.0,
        collisions: Vec::new(),
        backend: "OCCT exact CAM regression".to_owned(),
        tolerance: "1e-9 mm".to_owned(),
        result_fingerprint: String::new(),
    };
    evidence.result_fingerprint = evidence.stable_fingerprint();
    evidence
}

fn bidirectional_arc_operations() -> Vec<CamOperation> {
    let mut operations = practical_operations();
    let CamOperation::Contour { center_path, .. } = &mut operations[2] else {
        panic!("third practical operation must be the contour");
    };
    let CamPathSegment2d::Arc { clockwise, .. } = &mut center_path.segments[1] else {
        panic!("contour must end with an arc");
    };
    *clockwise = true;
    operations
}

#[test]
fn two_offline_postprocessors_are_deterministic_and_semantically_roundtrip() {
    let document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let toolpath =
        CamToolpath::plan(&snapshot, &canonical, &bidirectional_arc_operations()).unwrap();
    let simulation = successful_simulation(&canonical, &toolpath);

    let iso = CamPostprocessorOutput::generate(
        &snapshot,
        &canonical,
        &toolpath,
        &simulation,
        CamPostprocessorDialect::IsoMetricGCode,
    )
    .unwrap();
    let iso_again = CamPostprocessorOutput::generate(
        &snapshot,
        &canonical,
        &toolpath,
        &simulation,
        CamPostprocessorDialect::IsoMetricGCode,
    )
    .unwrap();
    let neutral = CamPostprocessorOutput::generate(
        &snapshot,
        &canonical,
        &toolpath,
        &simulation,
        CamPostprocessorDialect::ControllerNeutralJson,
    )
    .unwrap();

    assert_eq!(iso, iso_again);
    assert_ne!(iso.content, neutral.content);
    assert_eq!(iso.schema, CAM_POSTPROCESSOR_SCHEMA_V1);
    assert_eq!(iso.content_digest.len(), 64);
    assert_eq!(iso.parse().unwrap(), iso.program);
    assert_eq!(neutral.parse().unwrap(), iso.program);
    iso.verify(&snapshot, &canonical, &toolpath, &simulation)
        .unwrap();
    neutral
        .verify(&snapshot, &canonical, &toolpath, &simulation)
        .unwrap();

    let gcode = std::str::from_utf8(&iso.content).unwrap();
    for required in [
        "G21",
        "G54",
        "T1 M6",
        "S12000 M3",
        "G0 ",
        "G1 ",
        "G2 ",
        "G3 ",
        ";PLUNGE",
        ";RETRACT",
        "M5",
        "M30",
    ] {
        assert!(gcode.contains(required), "missing {required} in {gcode}");
    }
    for expected in [
        CamPostprocessedMotionKind::Rapid,
        CamPostprocessedMotionKind::Plunge,
        CamPostprocessedMotionKind::Cut,
        CamPostprocessedMotionKind::Retract,
        CamPostprocessedMotionKind::ArcClockwise,
        CamPostprocessedMotionKind::ArcCounterclockwise,
    ] {
        assert!(
            iso.program
                .motions
                .iter()
                .any(|motion| motion.kind == expected)
        );
    }
}

#[test]
fn postprocessing_refuses_unsafe_stale_and_tampered_inputs() {
    let mut document = solid_document();
    let snapshot = document.current();
    let canonical = plan(&snapshot).unwrap();
    let toolpath = CamToolpath::plan(&snapshot, &canonical, &practical_operations()).unwrap();
    let simulation = successful_simulation(&canonical, &toolpath);
    let output = CamPostprocessorOutput::generate(
        &snapshot,
        &canonical,
        &toolpath,
        &simulation,
        CamPostprocessorDialect::ControllerNeutralJson,
    )
    .unwrap();

    let mut gouged = simulation.clone();
    gouged.gouge_mm3 = 1.0;
    assert_eq!(
        CamPostprocessorOutput::generate(
            &snapshot,
            &canonical,
            &toolpath,
            &gouged,
            CamPostprocessorDialect::IsoMetricGCode,
        ),
        Err(CamPostprocessorError::UnsafeSimulation)
    );

    let mut forged_metrics = simulation.clone();
    forged_metrics.residual_stock_mm3 = 1.0;
    assert_eq!(
        CamPostprocessorOutput::generate(
            &snapshot,
            &canonical,
            &toolpath,
            &forged_metrics,
            CamPostprocessorDialect::IsoMetricGCode,
        ),
        Err(CamPostprocessorError::UnsafeSimulation)
    );

    let mut incomplete = simulation.clone();
    incomplete.motion_count -= 1;
    assert_eq!(
        CamPostprocessorOutput::generate(
            &snapshot,
            &canonical,
            &toolpath,
            &incomplete,
            CamPostprocessorDialect::IsoMetricGCode,
        ),
        Err(CamPostprocessorError::UnsafeSimulation)
    );

    let mut changed_content = output.clone();
    changed_content.content.push(b' ');
    assert_eq!(
        changed_content.parse(),
        Err(CamPostprocessorError::DigestMismatch)
    );

    let mut changed_receipt = output.clone();
    changed_receipt.program.tool_number += 1;
    assert_eq!(
        changed_receipt.verify(&snapshot, &canonical, &toolpath, &simulation),
        Err(CamPostprocessorError::RoundtripMismatch)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: SOLID,
                dimension: dimension(11.0),
            },
        ]))
        .unwrap();
    assert_eq!(
        output.verify(&document.current(), &canonical, &toolpath, &simulation),
        Err(CamPostprocessorError::Plan(CamPlannerError::Plan(
            CamError::StaleTarget
        )))
    );
}
