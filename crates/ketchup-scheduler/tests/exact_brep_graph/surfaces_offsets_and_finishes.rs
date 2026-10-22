use super::*;

#[test]
fn planar_surface_body_round_trips_rebuilds_and_remains_distinct_from_a_solid() {
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 25.0], [0.0, 25.0]]),
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
                kind: FeatureKind::extrusion(profile, dimension(5.0)),
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
    assert_eq!(loaded.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::closed_spline(&[
                    [-10.0, -5.0],
                    [10.0, -5.0],
                    [10.0, 5.0],
                    [-10.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: definition,
                name: "Upper surface section".into(),
                kind: FeatureKind::closed_spline(&[
                    [-6.0, -3.0],
                    [6.0, -3.0],
                    [6.0, 3.0],
                    [-6.0, 3.0],
                ]),
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 25.0], [0.0, 25.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: cutter_profile,
                definition_id: definition,
                name: "Cutter boundary".into(),
                kind: FeatureKind::polygon(&[[10.0, 5.0], [30.0, 5.0], [30.0, 20.0], [10.0, 20.0]]),
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
                kind: FeatureKind::extrusion(target_profile, dimension(5.0)),
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
    assert_eq!(loaded.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
    let _turn = crate::integration_support::file_turn();
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
            kind: FeatureKind::polygon(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]),
        },
        CanonicalCommand::CreateFeature {
            id: solid,
            definition_id: definition,
            name: "Solid comparison".into(),
            kind: FeatureKind::extrusion(solid_profile, dimension(2.0)),
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
    assert_eq!(loaded.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
    assert_eq!(open_package.mass, None);
    let solid_package = supervisor.evaluate_exact_brep_graph(&solid_graph).unwrap();
    assert!((solid_package.volume_mm3 - 6_000.0).abs() <= 1.0e-7);
    assert_eq!(solid_package.topology_counts[2..], [6, 1, 1]);
    // The knitted box carries its centre and central inertia through the worker.
    let mass = solid_package
        .mass
        .as_deref()
        .expect("a knitted solid has mass properties");
    let [low, high] = solid_package.bounds_mm;
    let size: [f64; 3] = std::array::from_fn(|axis| high[axis] - low[axis]);
    let moment = |a: usize, b: usize| 6_000.0 * (size[a] * size[a] + size[b] * size[b]) / 12.0;
    let expected = [moment(1, 2), moment(0, 2), moment(0, 1), 0.0, 0.0, 0.0];
    for axis in 0..3 {
        assert!((mass.centroid_mm[axis] - (low[axis] + high[axis]) / 2.0).abs() <= 1.0e-7);
    }
    for (actual, wanted) in mass.inertia_mm5.into_iter().zip(expected) {
        assert!((actual - wanted).abs() <= expected[0] * 1.0e-9, "{mass:?}");
    }
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]]),
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
                kind: FeatureKind::polygon(&[[30.0, 0.0], [32.0, 0.0], [32.0, 2.0], [30.0, 2.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Solid comparison".into(),
                kind: FeatureKind::extrusion(solid_profile, dimension(2.0)),
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
            direction: ketchup_model::exact_brep_graph::ExactBRepShellDirection::Outward,
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
    assert_eq!(loaded.source_schema(), persistence::OLDEST_WRITTEN_SCHEMA);
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::Profile {
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
        mass: None,
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
    // The capsule (x 0..20, y 0..10) grown by 2 mm on every side.
    for (actual, expected) in package
        .bounds_mm
        .into_iter()
        .flatten()
        .zip([-2.0, -2.0, 0.0, 22.0, 12.0, 0.0])
    {
        assert!(
            (actual - expected).abs() <= 1.0e-6,
            "{:?}",
            package.bounds_mm
        );
    }
}

#[test]
fn worker_evaluates_framed_cubic_sketch_planar_offset() {
    let _turn = crate::integration_support::file_turn();
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
    let _turn = crate::integration_support::file_turn();
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

        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.identity.producer_feature_id.0, OFFSET.0);
        assert_bounds_close(
            package.bounds_mm,
            [
                12.0 - output_radius_mm,
                -8.0 - output_radius_mm,
                0.0,
                12.0 + output_radius_mm,
                -8.0 + output_radius_mm,
                0.0,
            ],
        );
        let disc_area_mm2 = std::f64::consts::PI * output_radius_mm * output_radius_mm;
        assert!((package.area_mm2 - disc_area_mm2).abs() <= disc_area_mm2 * 1.0e-9);
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
            mass: None,
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
                    mass: None,
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
            use ketchup_scheduler::protocol::{
                Frame, GraphInput, MAX_REPLY_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, WorkerFailure,
                WorkerReply, WorkerRequest, read_frame, write_frame,
            };

            // Bypass the client's own validation: the worker must refuse the graph itself.
            let mut worker = std::process::Command::new(env!("CARGO_BIN_EXE_ketchup-exact-worker"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = worker.stdin.take().unwrap();
            write_frame(
                &mut stdin,
                &WorkerRequest::EvaluateGraph(GraphInput {
                    graph: collapsed.clone(),
                    sources: Vec::new(),
                }),
                MAX_REQUEST_FRAME_BYTES,
            )
            .unwrap();
            drop(stdin);
            let reply = read_frame::<WorkerReply>(
                &mut worker.stdout.take().unwrap(),
                MAX_REPLY_FRAME_BYTES,
            );
            assert!(matches!(
                reply,
                Frame::Message(WorkerReply::Failure(failure))
                    if failure.code == WorkerFailure::invalid_request().code
                        && failure.detail.as_ref().is_some_and(|detail| {
                            detail.operation == "validate_exact_brep_graph"
                                && !detail.diagnostic.is_empty()
                        })
            ));
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 8.0], [0.0, 8.0]]),
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
    let _turn = crate::integration_support::file_turn();
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
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                    operation: PadOperation::NewBody,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: symmetric,
                definition_id: definition,
                name: "Symmetric oblique".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::Vector([1.0, 0.0, 1.0]),
                    extent: FeatureExtent::Symmetric(dimension(10.0)),
                    operation: PadOperation::NewBody,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: bidirectional,
                definition_id: definition,
                name: "Unequal bidirectional".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Bidirectional {
                        along: FeatureExtentEnd::Blind(dimension(7.0)),
                        opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                    },
                    operation: PadOperation::NewBody,
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base extrusion".into(),
                kind: FeatureKind::extrusion(base_profile, dimension(19.0)),
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
                kind: FeatureKind::FaceOffset {
                    target: base,
                    face: FaceRef::from(top_face),
                    distance: dimension(5.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: cut_profile,
                definition_id: definition,
                name: "Interior cut profile".into(),
                kind: FeatureKind::polygon(&[[9.0, 7.0], [16.0, 7.0], [16.0, 12.0], [9.0, 12.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: cut,
                definition_id: definition,
                name: "Through all offset body".into(),
                kind: FeatureKind::through_cut(offset, cut_profile),
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[
                    [0.0, -10.0],
                    [10.0, -10.0],
                    [10.0, 10.0],
                    [0.0, 10.0],
                ]),
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
                kind: FeatureKind::polygon(&[[-2.0, -5.0], [2.0, -5.0], [2.0, 5.0], [-2.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: revolve_cut,
                definition_id: revolve_definition,
                name: "Revolve through all".into(),
                kind: FeatureKind::through_cut(revolve, revolve_cut_profile),
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
                kind: FeatureKind::closed_spline(&[
                    [-10.0, -10.0],
                    [10.0, -10.0],
                    [10.0, 10.0],
                    [-10.0, 10.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: loft_definition,
                name: "Upper loft profile".into(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -8.0],
                    [8.0, -8.0],
                    [8.0, 8.0],
                    [-8.0, 8.0],
                ]),
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
                kind: FeatureKind::polygon(&[[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: loft_cut,
                definition_id: loft_definition,
                name: "Loft through all".into(),
                kind: FeatureKind::through_cut(loft, loft_cut_profile),
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
                kind: FeatureKind::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
            },
            CanonicalCommand::CreateFeature {
                id: imported_cut,
                definition_id: imported_definition,
                name: "Imported through all".into(),
                kind: FeatureKind::through_cut(imported, imported_cut_profile),
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
                kind: FeatureKind::polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
            },
            CanonicalCommand::CreateFeature {
                id: stale_cut,
                definition_id: stale_definition,
                name: "Stale imported through all".into(),
                kind: FeatureKind::through_cut(stale_import_id, stale_profile),
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
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(18);
    let profile = FeatureId(1_800);
    let base = FeatureId(1_801);
    let cases = [
        (
            FeatureId(1_802),
            ketchup_model::document::ShellDirection::Inward,
            true,
        ),
        (
            FeatureId(1_803),
            ketchup_model::document::ShellDirection::Outward,
            true,
        ),
        (
            FeatureId(1_804),
            ketchup_model::document::ShellDirection::Symmetric,
            true,
        ),
        (
            FeatureId(1_805),
            ketchup_model::document::ShellDirection::Inward,
            false,
        ),
        (
            FeatureId(1_806),
            ketchup_model::document::ShellDirection::Outward,
            false,
        ),
        (
            FeatureId(1_807),
            ketchup_model::document::ShellDirection::Symmetric,
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::extrusion(profile, dimension(19.0)),
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
            kind: FeatureKind::Shell {
                target: base,
                removed_faces: if *open {
                    vec![FaceRef::from(opening.clone())]
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
                ketchup_model::exact_brep_graph::EXACT_BREP_GRAPH_SCHEMA_V16
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
            FeatureKind::Shell {
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
                kind: FeatureKind::Shell {
                    target: base,
                    removed_faces: Vec::new(),
                    thickness: dimension(0.0),
                    direction: ketchup_model::document::ShellDirection::Outward,
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::extrusion(profile, dimension(19.0)),
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
                kind: FeatureKind::EdgeFinish {
                    target: base,
                    edges: vec![EdgeRef::from(edge.clone())],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.5),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: variable,
                definition_id: definition,
                name: "Variable fillet".into(),
                kind: FeatureKind::EdgeFinish {
                    target: base,
                    edges: vec![EdgeRef::from(edge.clone())],
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
                    chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
    // Both volumes integrate the B-spline fillet faces numerically; the round trip
    // moves them by about 1e-8 to 3e-8 relative depending on the adjacent walls.
    assert!(
        step_volume_error <= variable_package.volume_mm3 * 5.0e-8,
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
            kind: FeatureKind::EdgeFinish {
                target: base,
                edges: vec![EdgeRef::from(edge)],
                kind: EdgeFinishKind::Fillet,
                amount: dimension(0.5),
                fillet_radius_stations: vec![FilletRadiusStation {
                    position: 0.75,
                    radius: dimension(1.0),
                }],
                chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::extrusion(profile, dimension(19.0)),
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let selected_edge = base_package
        .edge_evidence
        .iter()
        .find(|edge| edge.adjacent_face_ordinals.len() == 2)
        .unwrap();
    let first_face = selected_edge.adjacent_face_ordinals[0];
    let second_face = selected_edge.adjacent_face_ordinals[1];
    let invalid_face = (0..base_package.topology_counts[2])
        .find(|ordinal| !selected_edge.adjacent_face_ordinals.contains(ordinal))
        .unwrap();
    // Two opposite (parallel) edges bounding the same face.
    let shared_face = first_face;
    let direction = |bounds: [[f64; 3]; 2]| {
        (0..3)
            .max_by(|left, right| {
                (bounds[1][*left] - bounds[0][*left])
                    .total_cmp(&(bounds[1][*right] - bounds[0][*right]))
            })
            .unwrap()
    };
    let face_edges = base_package
        .edge_evidence
        .iter()
        .filter(|edge| edge.adjacent_face_ordinals.contains(&shared_face))
        .collect::<Vec<_>>();
    assert_eq!(face_edges.len(), 4);
    let opposite = face_edges[1..]
        .iter()
        .find(|edge| direction(edge.bounds_mm) == direction(face_edges[0].bounds_mm))
        .unwrap();
    let shared_edge_ordinals = [face_edges[0].edge_ordinal, opposite.edge_ordinal];

    let reference_for = |kind: TopologicalElementKind, ordinal: u32| {
        base_package
            .topological_references
            .iter()
            .filter(|reference| reference.kind == kind)
            .nth(ordinal as usize)
            .cloned()
            .unwrap()
    };
    let edge = reference_for(TopologicalElementKind::Edge, selected_edge.edge_ordinal);
    let side_a = reference_for(TopologicalElementKind::Face, first_face);
    let side_b = reference_for(TopologicalElementKind::Face, second_face);
    let wrong_side = reference_for(TopologicalElementKind::Face, invalid_face);
    let mut shared_sides = shared_edge_ordinals
        .into_iter()
        .map(|ordinal| ChamferEdgeSide {
            edge: reference_for(TopologicalElementKind::Edge, ordinal),
            side_face: reference_for(TopologicalElementKind::Face, shared_face),
        })
        .collect::<Vec<_>>();
    shared_sides.sort_unstable_by(|left, right| left.edge.cmp(&right.edge));

    let advanced = |edge: &TopologicalElementRef,
                    face: &TopologicalElementRef,
                    mode: ChamferMode| FeatureKind::EdgeFinish {
        target: base,
        edges: vec![EdgeRef::from(edge.clone())],
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
                kind: FeatureKind::EdgeFinish {
                    target: base,
                    edges: shared_sides
                        .iter()
                        .map(|selection| EdgeRef::from(selection.edge.clone()))
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::extrusion(profile, dimension(19.0)),
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
                kind: FeatureKind::Shell {
                    target: base,
                    removed_faces: faces.clone().into_iter().map(FaceRef::from).collect(),
                    thickness: dimension(1.5),
                    direction: ketchup_model::document::ShellDirection::Inward,
                },
            },
            CanonicalCommand::CreateFeature {
                id: fillet,
                definition_id: definition,
                name: "Selected-edge fillet".into(),
                kind: FeatureKind::EdgeFinish {
                    target: base,
                    edges: vec![EdgeRef::from(edges[0].clone())],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.75),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
                    chamfer_edge_sides: Vec::new(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: chamfer,
                definition_id: definition,
                name: "Selected-edge chamfer".into(),
                kind: FeatureKind::EdgeFinish {
                    target: base,
                    edges: edges.clone().into_iter().map(EdgeRef::from).collect(),
                    kind: EdgeFinishKind::Chamfer,
                    amount: dimension(0.75),
                    fillet_radius_stations: Vec::new(),
                    chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
            kind: FeatureKind::Shell {
                target: base,
                removed_faces: reversed_faces.into_iter().map(FaceRef::from).collect(),
                thickness: dimension(1.5),
                direction: ketchup_model::document::ShellDirection::Inward,
            },
        }])),
        Err(CanonicalError::InvalidTopologicalFeatureReference)
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: duplicate_chamfer,
            definition_id: definition,
            name: "Duplicate selected-edge chamfer".into(),
            kind: FeatureKind::EdgeFinish {
                target: base,
                edges: vec![
                    EdgeRef::from(edges[0].clone()),
                    EdgeRef::from(edges[0].clone())
                ],
                kind: EdgeFinishKind::Chamfer,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
            kind: FeatureKind::EdgeFinish {
                target: base,
                edges: vec![
                    EdgeRef::from(faces[0].clone()),
                    EdgeRef::from(edges[0].clone())
                ],
                kind: EdgeFinishKind::Chamfer,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
            kind: FeatureKind::EdgeFinish {
                target: base,
                edges: vec![EdgeRef::from(stale_edge)],
                kind: EdgeFinishKind::Fillet,
                amount: dimension(0.75),
                fillet_radius_stations: Vec::new(),
                chamfer_mode: ketchup_model::document::ChamferMode::Symmetric,
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
