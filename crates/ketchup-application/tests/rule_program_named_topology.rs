use std::collections::BTreeMap;

use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_application::{DocumentSession, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepOperation, ExactBRepProfileFaceReference,
};
use ketchup_scheduler::ExactWorkerSupervisor;

fn worker() -> ExactWorkerSupervisor {
    let executable = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    ExactWorkerSupervisor::spawn(executable).unwrap()
}

fn source(program: &str, overrides: &[(&str, f64)]) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "named-topology.star".to_owned(),
        source: program.to_owned(),
        overrides: overrides
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn graph(session: &DocumentSession) -> ExactBRepGraph {
    let snapshot = session.snapshot();
    let occurrence = snapshot.occurrences().next().unwrap();
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    ExactBRepGraph::from_snapshot(
        &snapshot,
        occurrence.definition_id(),
        *definition.feature_ids().last().unwrap(),
    )
    .unwrap()
}

fn apply_and_evaluate(
    session: &mut DocumentSession,
    worker: &mut ExactWorkerSupervisor,
    source: RuleProgramSource,
) -> u64 {
    let overrides = source.overrides.clone();
    let result = session
        .apply_rule_program(source, false)
        .unwrap_or_else(|error| panic!("apply failed for {overrides:?}: {error}"));
    worker.evaluate_exact_brep_graph(&graph(session)).unwrap();
    result.snapshot.occurrences().next().unwrap().id().0
}

#[test]
fn three_starlark_named_topology_cases_survive_parameter_changes_in_exact_worker() {
    let mut worker = worker();

    let fillet = "W = param(\"width\", 40)\npart = extrude(\"angle\", profile=[[\"bottom\", [0, 0], [W, 0]], [\"outer\", [W, 0], [W, 10]], [\"ledge\", [W, 10], [10, 10]], [\"inner\", [10, 10], [10, 40]], [\"top\", [10, 40], [0, 40]], [\"back\", [0, 40], [0, 0]]], distance=100)\nfillet(part, edges=[[\"bottom\", \"outer\"]], radius=2, name=\"outer round\")";
    let mut fillet_session = DocumentSession::new(SessionSettings::default());
    let fillet_id = apply_and_evaluate(
        &mut fillet_session,
        &mut worker,
        source(fillet, &[("width", 40.0)]),
    );
    assert_eq!(
        apply_and_evaluate(
            &mut fillet_session,
            &mut worker,
            source(fillet, &[("width", 55.0)]),
        ),
        fillet_id
    );
    assert!(graph(&fillet_session).nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::EdgeFinish {
            edges,
            profile_edges,
            ..
        } if edges.is_empty() && !profile_edges.is_empty()
    )));

    let chamfer = "A = param(\"angle\", 90)\npart = revolve(\"ring\", profile=[[\"bottom\", [40, 0], [60, 0]], [\"outer\", [60, 0], [60, 30]], [\"top\", [60, 30], [40, 30]], [\"inner\", [40, 30], [40, 0]]], axis=[[0, 0], [0, 1]], angle=A)\nchamfer(part, edges=[[\"top\", \"outer\"]], distance=2, name=\"outer bevel\")";
    let mut chamfer_session = DocumentSession::new(SessionSettings::default());
    let chamfer_id = apply_and_evaluate(
        &mut chamfer_session,
        &mut worker,
        source(chamfer, &[("angle", 90.0)]),
    );
    assert_eq!(
        apply_and_evaluate(
            &mut chamfer_session,
            &mut worker,
            source(chamfer, &[("angle", 270.0)]),
        ),
        chamfer_id
    );

    let split = "L = param(\"length\", 500)\nG = param(\"groove\", 150)\nP = param(\"pull\", 10)\npart = extrude(\"board\", profile=[[\"front\", [0, 0], [L, 0]], [\"right\", [L, 0], [L, 300]], [\"back\", [L, 300], [0, 300]], [\"left\", [0, 300], [0, 0]]], distance=18)\ncut(part, profile=[[\"entry\", [G, -1], [G + 8, -1]], [\"wall_right\", [G + 8, -1], [G + 8, 301]], [\"exit\", [G + 8, 301], [G, 301]], [\"wall_left\", [G, 301], [G, -1]]], depth=6, name=\"groove\")\npush_pull(part, face=\"end#2\", distance=P, name=\"raise right half\")";
    let mut split_session = DocumentSession::new(SessionSettings::default());
    let split_id = apply_and_evaluate(
        &mut split_session,
        &mut worker,
        source(
            split,
            &[("length", 500.0), ("groove", 150.0), ("pull", 10.0)],
        ),
    );
    for values in [
        [("length", 500.0), ("groove", 320.0), ("pull", 10.0)],
        [("length", 700.0), ("groove", 40.0), ("pull", -4.0)],
    ] {
        assert_eq!(
            apply_and_evaluate(&mut split_session, &mut worker, source(split, &values)),
            split_id
        );
    }
    let split_graph = graph(&split_session);
    assert!(split_graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::ProfileCut {
            tool_name: Some(name),
            ..
        } if name == "groove"
    )));
    assert!(split_graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ExactBRepOperation::FaceOffset {
            face: None,
            profile_face: Some(ExactBRepProfileFaceReference::NamedResult { name }),
            ..
        } if name == "end#2"
    )));
}

#[test]
fn missing_face_and_non_adjacent_edge_are_named_worker_errors() {
    let mut worker = worker();

    let missing_face = "part = extrude(\"block\", profile=[[\"front\", [0, 0], [100, 0]], [\"right\", [100, 0], [100, 60]], [\"back\", [100, 60], [0, 60]], [\"left\", [0, 60], [0, 0]]], distance=18)\npush_pull(part, face=\"end#2\", distance=2, name=\"missing split\")";
    let mut missing_session = DocumentSession::new(SessionSettings::default());
    missing_session
        .apply_rule_program(source(missing_face, &[]), false)
        .unwrap();
    let error = worker
        .evaluate_exact_brep_graph(&graph(&missing_session))
        .unwrap_err()
        .to_string();
    assert!(error.contains("face 'end#2' does not exist"), "{error}");
    assert!(error.contains("end"), "{error}");

    let non_adjacent = "part = extrude(\"block\", profile=[[\"front\", [0, 0], [100, 0]], [\"right\", [100, 0], [100, 60]], [\"back\", [100, 60], [0, 60]], [\"left\", [0, 60], [0, 0]]], distance=18)\nfillet(part, edges=[[\"start\", \"end\"]], radius=1, name=\"impossible\")";
    let mut edge_session = DocumentSession::new(SessionSettings::default());
    edge_session
        .apply_rule_program(source(non_adjacent, &[]), false)
        .unwrap();
    let error = worker
        .evaluate_exact_brep_graph(&graph(&edge_session))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("faces 'start' and 'end' do not share an edge"),
        "{error}"
    );
    assert!(error.contains("borders"), "{error}");
}
