use ketchup_geometry::sketch::{
    FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
    SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};
use ketchup_geometry::thread_profile::{MetricThread, groove_area_per_length};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
};
use ketchup_model::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V23, ExactBRepGraph, ExactBRepGraphError, ExactBRepNode,
    ExactBRepNodeId, ExactBRepOperation, ExactBRepThreadGroove,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::f64::consts::PI;
use std::time::Instant;

const M5: MetricThread = MetricThread {
    nominal_diameter_mm: 5.0,
    pitch_mm: 0.8,
};
const LENGTH_MM: f64 = 20.0;

/// The graph of a plain M5 shank at the 6g crest, `LENGTH_MM` up from the
/// XY plane.
fn shank_graph(crest_radius_mm: f64) -> ExactBRepGraph {
    let definition = DefinitionId(7);
    let [plane, sketch_id, pad] = [FeatureId(70), FeatureId(71), FeatureId(72)];
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: crest_radius_mm,
        }],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "M5 shank".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
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
                name: "Shank".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: sketch_id,
                        region,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(
                        Dimension::new(LENGTH_MM.to_string(), LENGTH_MM).unwrap(),
                    ),
                    operation: PadOperation::NewBody,
                }),
            },
        ]))
        .unwrap();
    ExactBRepGraph::from_snapshot(&document.current(), definition, pad).unwrap()
}

/// `graph` with an M5 thread cut into its last node along +z, running one
/// pitch past both ends of the shank.
fn threaded(
    mut graph: ExactBRepGraph,
    profile: &[[f64; 2]],
) -> Result<ExactBRepGraph, ExactBRepGraphError> {
    let target = graph.nodes.last().unwrap().id;
    let source_feature_id = 9_000;
    graph.nodes.push(ExactBRepNode {
        id: ExactBRepNodeId(target.0 + 1),
        source_feature_id,
        operation: ExactBRepOperation::ThreadGroove(ExactBRepThreadGroove {
            target,
            origin_bits: [0.0_f64; 3].map(f64::to_bits),
            axis_bits: [0.0, 0.0, 1.0].map(f64::to_bits),
            reference_bits: [1.0, 0.0, 0.0].map(f64::to_bits),
            pitch_bits: M5.pitch_mm.to_bits(),
            start_bits: (-M5.pitch_mm).to_bits(),
            end_bits: (LENGTH_MM + M5.pitch_mm).to_bits(),
            left_handed: false,
            profile_bits: profile
                .iter()
                .map(|point| point.map(f64::to_bits))
                .collect(),
        }),
    });
    graph.producer_feature_id = source_feature_id;
    graph.schema = EXACT_BREP_GRAPH_SCHEMA_V23.to_owned();
    graph.reseal_for_testing()?;
    Ok(graph)
}

#[test]
fn the_worker_cuts_a_modelled_thread_into_a_graph_body() {
    let _turn = crate::integration_support::file_turn();
    let crest = M5.external_6g().unwrap().major_mm / 2.0;
    let profile = M5.external_groove(2.0 * crest).unwrap();
    let graph = threaded(shank_graph(crest), &profile).unwrap();
    let round_trip = ExactBRepGraph::from_bytes(&graph.to_bytes().unwrap()).unwrap();
    assert_eq!(round_trip, graph);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let plain = Instant::now();
    supervisor
        .evaluate_exact_brep_graph(&shank_graph(crest))
        .unwrap();
    let plain = plain.elapsed();
    let started = Instant::now();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let elapsed = started.elapsed();
    let removed = groove_area_per_length(&profile, M5.pitch_mm, 0.0, crest);
    let expected = LENGTH_MM * (PI * crest * crest - removed);
    assert!(
        (package.volume_mm3 - expected).abs() <= expected * 1.0e-4,
        "{} mm³ != {expected} mm³",
        package.volume_mm3
    );
    assert_eq!(package.topology_counts[4], 1, "one solid");
    println!(
        "worker, M5x20 bolt with mesh: plain shank {plain:?}, modelled thread {elapsed:?}, {} faces, {} triangles",
        package.face_evidence.len(),
        package.triangles.len()
    );
}

#[test]
fn a_thread_as_wide_as_its_pitch_is_not_a_valid_graph() {
    let crest = M5.external_6g().unwrap().major_mm / 2.0;
    let wide = [[2.0, -0.4], [2.6, -0.4], [2.6, 0.4], [2.0, 0.4]];
    assert_eq!(
        threaded(shank_graph(crest), &wide).unwrap_err(),
        ExactBRepGraphError::InvalidGraph
    );
    let off_axis = [[0.0, -0.1], [2.6, -0.1], [2.6, 0.1]];
    assert_eq!(
        threaded(shank_graph(crest), &off_axis).unwrap_err(),
        ExactBRepGraphError::InvalidGraph
    );
}
