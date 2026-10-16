use super::*;

#[test]
fn worker_preserves_cubic_sketch_region_hole_volume_and_result_identity() {
    let _turn = crate::integration_support::file_turn();
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
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(12.0)),
                    operation: PadOperation::NewBody,
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
    let _turn = crate::integration_support::file_turn();
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
                    profile: PadProfile::SketchRegion {
                        sketch: base_sketch_id,
                        region: base_region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                    operation: PadOperation::NewBody,
                }),
            },
        ]))
        .unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package =
        ExactBodyPackage::from(supervisor.evaluate_exact_brep_graph(&base_graph).unwrap());
    let top = base_package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in base_package.references().iter().cloned() {
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
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: pocket_sketch_id,
                        region: pocket_region,
                    },
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(dimension(4.0)),
                    operation: PadOperation::Cut {
                        target: base,
                        start: CutStart::Support(Box::new(top)),
                    },
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Line-cubic-line C1 path".into(),
                kind: FeatureKind::Profile {
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
                kind: FeatureKind::sweep(profile, path),
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
    let _turn = crate::integration_support::file_turn();
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
                kind: FeatureKind::polygon(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]]),
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
                kind: FeatureKind::sweep(profile, path),
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
    let _turn = crate::integration_support::file_turn();
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
                    kind: FeatureKind::sweep(profile, path),
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
                kind: FeatureKind::sweep(profile, path),
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

#[test]
fn a_closed_profile_drawn_either_way_round_extrudes_the_same_solid() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(1);
    let square = [[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
    let reversed = |points: &[[f64; 2]]| points.iter().rev().copied().collect::<Vec<_>>();
    let graph = |profile: FeatureKind| {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Loop".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(1),
                    definition_id: definition,
                    name: "Profile".into(),
                    kind: profile,
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: definition,
                    name: "Extrusion".into(),
                    kind: FeatureKind::extrusion(FeatureId(1), dimension(10.0)),
                },
            ]))
            .unwrap();
        ExactBRepGraph::from_snapshot(&document.current(), definition, FeatureId(2))
    };
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let volumes = [square.to_vec(), reversed(&square)].map(|points| {
        let package = supervisor
            .evaluate_exact_brep_graph(&graph(FeatureKind::polygon(&points)).unwrap())
            .unwrap();
        assert_eq!(package.topology_counts[4], 1);
        package.volume_mm3
    });
    assert!((volumes[0] - 4_000.0).abs() <= 1.0e-7);
    assert!((volumes[1] - volumes[0]).abs() <= 1.0e-7);
    // Exact spline extrusion is not supported yet; loop direction must not be what
    // stops a closed spline from compiling into the exact graph.
    for points in [square.to_vec(), reversed(&square)] {
        graph(FeatureKind::closed_spline(&points)).expect("closed spline in either direction");
    }
}

/// A draft is one feature in the chain: it tilts the recorded sides of a box about
/// its bottom into an exact frustum, keeps its angle across save and reopen, and a
/// draft the document cannot take changes nothing.
#[test]
fn draft_feature_tilts_recorded_sides_into_an_exact_frustum_and_survives_reopen() {
    let _turn = crate::integration_support::file_turn();
    let definition = DefinitionId(37);
    let profile = FeatureId(3_700);
    let base = FeatureId(3_701);
    let draft = FeatureId(3_702);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Drafted box".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Square".into(),
                kind: FeatureKind::polygon(&[
                    [0.0, 0.0],
                    [100.0, 0.0],
                    [100.0, 100.0],
                    [0.0, 100.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Box".into(),
                kind: FeatureKind::extrusion(profile, dimension(50.0)),
            },
        ]))
        .unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    // The face references are listed in face ordinal order.
    let face_references = base_package
        .topological_references
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .collect::<Vec<_>>();
    let face_facing = |normal_z: f64| {
        let mut faces = base_package
            .face_evidence
            .iter()
            .filter(|face| (face.unit_normal[2] - normal_z).abs() < 1e-9)
            .map(|face| face_references[face.face_ordinal as usize].clone())
            .map(FaceRef::from)
            .collect::<Vec<_>>();
        faces.sort_unstable_by(|left, right| left.topological().cmp(&right.topological()));
        faces
    };
    let (bottom, sides, top) = (face_facing(-1.0), face_facing(0.0), face_facing(1.0));
    assert_eq!((bottom.len(), sides.len(), top.len()), (1, 4, 1));
    let draft_kind = |faces: Vec<FaceRef>, angle_degrees: f64| {
        FeatureKind::Draft(DraftSpec {
            target: base,
            faces,
            neutral: bottom[0].clone(),
            angle_degrees,
        })
    };

    let before = document.current();
    for (kind, error) in [
        (
            draft_kind(sides.clone(), 45.5),
            "canonical.draft_angle_out_of_range",
        ),
        (
            draft_kind(sides.clone(), 0.05),
            "canonical.draft_angle_out_of_range",
        ),
        (
            draft_kind(bottom.clone(), 5.0),
            "canonical.invalid_topological_feature_reference",
        ),
    ] {
        let Err(refused) =
            document.apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: draft,
                definition_id: definition,
                name: "Refused draft".into(),
                kind,
            }]))
        else {
            panic!("the document took a draft it must refuse");
        };
        assert_eq!(refused.code(), error);
        assert_eq!(
            document.current().canonical_digest(),
            before.canonical_digest()
        );
        assert!(document.current().revision_id() == before.revision_id());
    }

    for angle in [5.0_f64, -5.0] {
        let mut drafted = DocumentStore::from_snapshot(&before).unwrap();
        drafted
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: draft,
                definition_id: definition,
                name: "Draft".into(),
                kind: draft_kind(sides.clone(), angle),
            }]))
            .unwrap();
        let snapshot = drafted.current();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, draft).unwrap();
        assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V23);
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        let top_side = 100.0 - 100.0 * angle.to_radians().tan();
        let (a1, a2) = (100.0 * 100.0, top_side * top_side);
        let frustum = 50.0 / 3.0 * (a1 + a2 + (a1 * a2).sqrt());
        assert!(
            (package.volume_mm3 - frustum).abs() <= frustum * 1e-9,
            "{} != {frustum}",
            package.volume_mm3
        );

        let bytes = persistence::save(&snapshot);
        let reopened = persistence::load(&bytes).unwrap().snapshot();
        assert_eq!(persistence::save(&reopened), bytes);
        assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
        assert!(matches!(
            reopened.feature(draft).unwrap().kind(),
            FeatureKind::Draft(spec) if spec.angle_degrees == angle
        ));
    }

    // The document takes these drafts, but the kernel refuses each by name: the top
    // faces the pull, and 45 degrees on the 50 mm walls closes the top to nothing.
    for (kind, cause) in [
        (draft_kind(top, 5.0), "draft_face_perpendicular_to_pull"),
        (draft_kind(sides, 45.0), "draft_face_degenerates"),
    ] {
        let mut refused = DocumentStore::from_snapshot(&before).unwrap();
        refused
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: draft,
                definition_id: definition,
                name: "Refused draft".into(),
                kind,
            }]))
            .unwrap();
        let graph = ExactBRepGraph::from_snapshot(&refused.current(), definition, draft).unwrap();
        let error = supervisor.evaluate_exact_brep_graph(&graph).unwrap_err();
        assert!(error.to_string().contains(cause), "{error}");
    }
}
