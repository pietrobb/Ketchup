use super::*;

// A model's first check built 22 parts of one purchased STEP on every
// worker, and each part's graph opened the source for itself: 214 parses
// of about 4 s. Graphs of parts of one source, each its own request as in
// separate pair batches, now parse it once on a worker.
#[test]
fn graphs_of_the_parts_of_one_source_parse_it_once_on_a_worker() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let bytes = std::fs::read(&corpus).unwrap();
    let step = SourceFile {
        sha256: sha256_hex(&bytes),
        path: corpus,
    };
    let backend = ExactBackend::new();
    let WorkerReply::Xde(evidence) =
        m21_step_xde_inspection_response(&backend, &mut SourceDocument::default(), &step).unwrap()
    else {
        panic!("the STEP source inspects as XDE");
    };
    let template: ExactBRepGraph = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/v9_top_side_contact/left.graph"
    ))
    .unwrap();
    let graphs = evidence
        .parts
        .iter()
        .map(|part| {
            let mut graph = template.clone();
            graph.schema = ketchup_model::exact_brep_graph::EXACT_BREP_GRAPH_SCHEMA_V23.to_owned();
            graph.producer_feature_id = 1;
            graph.profiles.clear();
            graph.nodes = vec![ketchup_model::exact_brep_graph::ExactBRepNode {
                id: ketchup_model::exact_brep_graph::ExactBRepNodeId(0),
                source_feature_id: 1,
                operation: ExactBRepOperation::ImportedExact {
                    source_sha256: ketchup_model::graph::sha256_bytes(&bytes),
                    source_byte_len: bytes.len() as u64,
                    result_fingerprint: part.result_fingerprint.clone(),
                    source_format: ExactBRepImportedFormat::Step,
                    source_part_index: Some(part.index),
                    enclosing_bounds_bits: None,
                },
            }];
            graph.reseal_for_testing().unwrap();
            graph
        })
        .collect::<Vec<_>>();
    let mut pairs = pair_query::PairQuerySession::default();
    let mut read = SourceDocument::default();

    let before = ExactBackend::step_source_parses_on_this_thread();
    for graph in &graphs {
        let reply = handle_request(
            &backend,
            &mut pairs,
            &mut read,
            WorkerRequest::EvaluateGraph(GraphInput {
                graph: graph.clone(),
                sources: vec![step.clone()],
            }),
        );
        assert!(reply.is_ok(), "{reply:?}");
    }
    assert_eq!(
        (
            graphs.len(),
            ExactBackend::step_source_parses_on_this_thread() - before
        ),
        (2, 1),
        "part graphs, then parses of their source"
    );
}
