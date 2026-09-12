use ketchup_core::document::{
    BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, Transform,
};
use ketchup_core::exact_brep_graph::{
    ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation, MAX_EXACT_BREP_GRAPH_NODES,
};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFINITION: DefinitionId = DefinitionId(1);
const CHILD_NODE_COUNT: &str = "KETCHUP_GRAPH_RESOURCE_CHILD_NODES";

fn transform_chain(node_count: usize) -> DocumentStore {
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Graph resource regression".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DEFINITION,
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DEFINITION,
            name: "Extrusion".into(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: Dimension::new("5", 5.0).unwrap(),
            },
        },
    ];
    for index in 1..node_count {
        commands.push(CanonicalCommand::CreateFeature {
            id: FeatureId(index as u64 + 2),
            definition_id: DEFINITION,
            name: format!("Transform {index}"),
            kind: FeatureKind::RigidTransform {
                target: FeatureId(index as u64 + 1),
                transform: Transform::identity(),
            },
        });
    }
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn deep_graph_compilation_child() {
    let Ok(count) = std::env::var(CHILD_NODE_COUNT) else {
        return;
    };
    let count: usize = count.parse().unwrap();
    let document = transform_chain(count);
    let snapshot = document.current();
    let digest = snapshot.canonical_digest();
    eprintln!("Compiling canonical chain with {count} body nodes");
    let result = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, FeatureId(count as u64 + 1));
    if count > MAX_EXACT_BREP_GRAPH_NODES {
        assert_eq!(result, Err(ExactBRepGraphError::ResourceLimit));
    } else {
        let graph = result.unwrap();
        assert_eq!(graph.nodes.len(), count);
        assert_eq!(graph.profiles.len(), 1);
        for (index, node) in graph.nodes.iter().enumerate().skip(1) {
            assert_eq!(node.source_feature_id, index as u64 + 2);
            assert!(
                matches!(node.operation, ExactBRepOperation::RigidTransform { target, .. }
                if target.0 as usize == index - 1)
            );
        }
        assert_eq!(
            ExactBRepGraph::from_bytes(&graph.to_bytes().unwrap()).unwrap(),
            graph
        );
    }
    assert_eq!(snapshot.canonical_digest(), digest);
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn deep_graph_compilation_is_bounded_and_accepts_the_node_limit() {
    for count in [
        MAX_EXACT_BREP_GRAPH_NODES,
        MAX_EXACT_BREP_GRAPH_NODES + 1,
        MAX_EXACT_BREP_GRAPH_NODES * 4,
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "deep_graph_compilation_child", "--nocapture"])
            .env(CHILD_NODE_COUNT, count.to_string())
            .env("RUST_MIN_STACK", "2097152")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "compilation of {count} nodes failed: {status}"
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("compilation of {count} nodes exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn shared_dependencies_preserve_depth_first_order_and_fingerprint() {
    let mut document = transform_chain(3);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DEFINITION,
                name: "Shared branch".into(),
                kind: FeatureKind::RigidTransform {
                    target: FeatureId(2),
                    transform: Transform::identity(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DEFINITION,
                name: "Join".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: FeatureId(5),
                    tool: FeatureId(4),
                },
            },
        ]))
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, FeatureId(6)).unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .map(|node| node.source_feature_id)
            .collect::<Vec<_>>(),
        [2, 5, 3, 4, 6]
    );
    assert_eq!(
        graph.graph_digest,
        "700dbdc32ed11607f78b1938f829a2d6a7a37170156d60c66adb2992338bbd91"
    );
}
