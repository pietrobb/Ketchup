use ketchup_core::document::{
    CanonicalCommand, CanonicalOverride, CollectionId, CommandBatch, DefinitionId, DerivedIdentity,
    Dimension, DocumentStore, EvaluationIdentity, FeatureId, FeatureKind, GroupId, NodeId,
    OccurrenceId, OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution,
    SlotSegment, TagId, Transform,
};
use ketchup_core::persistence;
use ketchup_core::state_view::{
    AGENT_STATE_VIEW, COMPLETE_STATE_VIEW, encode_semantic_state,
    encode_semantic_state_with_evaluation,
};
use std::path::{Path, PathBuf};

fn fixture_document(reverse_nodes: bool) -> DocumentStore {
    let nodes = if reverse_nodes {
        vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(2),
                name: "Dependent".to_owned(),
                dimension: Dimension::new("2.500", 2.5).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Width \"quoted\"".to_owned(),
                dimension: Dimension::new("100.00", 100.0).unwrap(),
                dependencies: vec![],
            },
        ]
    } else {
        vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Width \"quoted\"".to_owned(),
                dimension: Dimension::new("100.00", 100.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(2),
                name: "Dependent".to_owned(),
                dimension: Dimension::new("2.500", 2.5).unwrap(),
                dependencies: vec![],
            },
        ]
    };
    let mut commands = nodes;
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(10),
            name: "Cabinet\\nA".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(11),
            definition_id: DefinitionId(10),
            name: "Rectangle".to_owned(),
            kind: FeatureKind::polygon(&[[0.0, 0.0], [600.0, 0.0], [600.0, 580.0], [0.0, 580.0]]),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(12),
            definition_id: DefinitionId(10),
            name: "Extrusion".to_owned(),
            kind: FeatureKind::extrusion(FeatureId(11), Dimension::new("720.000", 720.0).unwrap()),
        },
        CanonicalCommand::CreateTag {
            id: TagId(7),
            name: "Casework".to_owned(),
            visible: true,
        },
        CanonicalCommand::CreateGroup {
            id: GroupId(20),
            name: "Kitchen run".to_owned(),
            transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
            parent: None,
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(30),
            definition_id: DefinitionId(10),
            name: "Cabinet #1".to_owned(),
            transform: Transform::identity(),
            parent: Some(GroupId(20)),
            tag: Some(TagId(7)),
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(31),
            definition_id: DefinitionId(10),
            name: "Cabinet #2".to_owned(),
            transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
            parent: None,
            tag: None,
            visible: false,
        },
        CanonicalCommand::CreateCollection {
            id: CollectionId(8),
            name: "Upper run".to_owned(),
        },
        CanonicalCommand::SetCollectionOccurrences {
            id: CollectionId(8),
            occurrence_ids: vec![OccurrenceId(30), OccurrenceId(31)],
        },
    ]);
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("state-view")
        .join(name)
}

fn mask_durable_values(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            if let Some((key, _)) = line.split_once('=')
                && (key.ends_with("document_id") || key.ends_with("canonical_digest"))
            {
                format!("{key}=<durable>")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn assert_golden(name: &str, actual: &str) {
    let path = fixture_path(name);
    let masked = mask_durable_values(actual);
    if std::env::var_os("UPDATE_STATE_VIEW_FIXTURES").is_some() {
        std::fs::write(&path, &masked).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    assert_eq!(masked, expected, "StateView drift in {}", path.display());
}

#[test]
fn complete_and_agent_views_match_golden_fixtures() {
    assert_ne!(COMPLETE_STATE_VIEW, AGENT_STATE_VIEW);
    let state = encode_semantic_state(&fixture_document(false).current());
    let complete = state.complete();
    let agent = state.agent();

    assert_golden("complete.txt", &complete);
    assert!(!complete.contains("<durable>"));
    assert!(
        !complete.contains(&format!(
            "document_id={}",
            fixture_document(false).current().document_id().0
        )) || complete.contains("document_id=")
    );
    assert_golden("agent.txt", &agent);
    assert!(complete.contains("features.12.kind.Pad.extent.Blind.millimetres=720.0"));
    assert!(complete.contains("occurrences.30.transform.matrix=[1.0,"));
    assert!(agent.contains("occurrences.30={id:30,"));
    assert!(!agent.contains("occurrences.30.transform"));
    assert!(agent.contains("evaluation=not_supplied"));
}

#[test]
fn one_encoder_is_deterministic_and_complete_output_detects_semantic_drift() {
    let first = fixture_document(false);
    let reordered = fixture_document(true);
    let normalize = |value: String| {
        value
            .lines()
            .filter(|line| {
                !line.starts_with("document_id=") && !line.starts_with("source.canonical_digest=")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        normalize(encode_semantic_state(&first.current()).complete()),
        normalize(encode_semantic_state(&reordered.current()).complete())
    );

    let before = encode_semantic_state(&first.current()).complete();
    let mut changed = first;
    changed
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(31),
                visible: true,
            },
        ]))
        .unwrap();
    let after = encode_semantic_state(&changed.current()).complete();
    assert_ne!(before, after);
    assert!(after.contains("occurrences.31.visible=true"));
}

#[test]
fn final_m2_state_view_covers_graph_overrides_and_supplied_evaluation_without_mutation() {
    let outer = SlotSegment::new(NodeId(3), "items", "cabinet").unwrap();
    let inner = SlotSegment::new(NodeId(3), "items", "drawer").unwrap();
    let nested_path = SlotPath::new(vec![outer.clone(), inner.clone()]).unwrap();
    let target = DerivedIdentity::new(NodeId(3), nested_path).unwrap();
    let outputs =
        vec![RuleOutput::new(outer, vec![RuleOutput::new(inner, vec![]).unwrap()]).unwrap()];

    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "base".into(),
                dimension: Dimension::new("4.5", 4.5).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: NodeId(2),
                name: "double".into(),
                expression: "$1 * 2".into(),
            },
            CanonicalCommand::CreateRuleNode {
                id: NodeId(3),
                name: "layout".into(),
                expression: "$2 + 1".into(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("items").unwrap()],
                outputs,
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::UpsertOverride(
                CanonicalOverride::new(
                    7,
                    target,
                    "offset",
                    6.25,
                    SlotResolution::Lost { segment_index: 0 },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();

    let snapshot = store.current();
    assert_eq!(
        snapshot.override_by_id(7).unwrap().health,
        SlotResolution::Resolved
    );
    let canonical_digest = snapshot.canonical_digest();
    let revision = snapshot.revision_id();
    let identity = EvaluationIdentity {
        evaluator: "state-view-test-evaluator".into(),
        schema: "state-view-test-schema".into(),
        tolerance: "state-view-test-tolerance".into(),
        backend: Some("state-view-test-backend".into()),
    };
    let report = snapshot.evaluate(&identity).unwrap();
    let state = encode_semantic_state_with_evaluation(&snapshot, Some(&report));
    let complete = state.complete();
    let agent = state.agent();

    for expected in [
        "evaluator_nodes.1.kind.Parameter.value.source_token=\"4.5\"",
        "evaluator_nodes.1.dependencies=[]",
        "evaluator_nodes.1.output_ports.0.name=\"value\"",
        "evaluator_nodes.1.output_ports.0.value_type=\"Number\"",
        "evaluator_nodes.2.kind.Expression.source=\"$1 * 2\"",
        "evaluator_nodes.2.dependencies=[1]",
        "evaluator_nodes.2.input_ports.0.name=\"node_1\"",
        "evaluator_nodes.2.input_ports.0.value_type=\"Number\"",
        "evaluator_nodes.2.output_ports.0.name=\"value\"",
        "evaluator_nodes.3.kind.Rule.source=\"$2 + 1\"",
        "evaluator_nodes.3.dependencies=[2]",
        "evaluator_nodes.3.input_ports.0.name=\"source\"",
        "evaluator_nodes.3.output_ports.0.name=\"items\"",
        "evaluator_nodes.3.kind.Rule.outputs.0.children.0.segment.semantic_key=\"drawer\"",
        "evaluator_nodes.3.kind.Rule.allowed_parameters.0.name=\"offset\"",
        "evaluator_nodes.3.kind.Rule.allowed_parameters.0.merge_policy=\"Replace\"",
        "overrides.7.target.root_rule_node_id=3",
        "overrides.7.target.slot_path.0.semantic_key=\"cabinet\"",
        "overrides.7.target.slot_path.1.semantic_key=\"drawer\"",
        "overrides.7.parameter=\"offset\"",
        "overrides.7.health=\"Resolved\"",
        "evaluation.identity.evaluator=\"state-view-test-evaluator\"",
        "evaluation.identity.schema=\"state-view-test-schema\"",
        "evaluation.identity.tolerance=\"state-view-test-tolerance\"",
        "evaluation.identity.backend=\"state-view-test-backend\"",
        "evaluation.current=true",
        "evaluation.recomputed_nodes=[1,2,3]",
        "evaluation.outputs.3:3:items:cabinet:3:items:drawer.value=10.0",
    ] {
        assert!(
            complete.contains(expected),
            "missing StateView line: {expected}"
        );
    }
    assert!(complete.contains(&format!("source.canonical_digest={canonical_digest}")));
    assert!(complete.contains(&format!("source.revision={revision}")));
    assert!(complete.contains(&format!(
        "evaluation.document_id={}",
        snapshot.document_id().0
    )));
    assert!(complete.contains(&format!("evaluation.revision_id={revision}")));
    assert!(complete.contains(&format!(
        "evaluation.canonical_digest=\"{canonical_digest}\""
    )));
    assert!(complete.contains(&format!("overrides.7.value_bits={}", 6.25_f64.to_bits())));

    for (id, value) in [(1, 4.5_f64), (2, 9.0_f64), (3, 10.0_f64)] {
        let result = report.node(NodeId(id)).unwrap();
        assert!(complete.contains(&format!(
            "evaluation.nodes.{id}.input_digest=\"{}\"",
            result.input_digest
        )));
        assert!(complete.contains(&format!(
            "evaluation.nodes.{id}.result_digest=\"{}\"",
            result.result_digest
        )));
        assert!(complete.contains(&format!("evaluation.nodes.{id}.status.Evaluated={value:?}")));
    }

    assert!(
        agent
            .contains("summary.counts=evaluator_nodes:3,overrides:1,feature_parameter_bindings:0,")
    );
    assert!(agent.contains("evaluation.current=true"));
    assert!(!agent.contains("evaluation=not_supplied"));

    assert_eq!(snapshot.canonical_digest(), canonical_digest);
    assert_eq!(snapshot.revision_id(), revision);
    assert_eq!(store.current().canonical_digest(), canonical_digest);
    assert_eq!(store.current().revision_id(), revision);
}

fn committed_documents(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if name != "target" && name != "invalid" && !name.starts_with('.') {
                committed_documents(&path, found);
            }
        } else if name.ends_with(".ketchup")
            || path
                .ancestors()
                .any(|ancestor| ancestor.ends_with("fixtures/persistence"))
        {
            found.push(path);
        }
    }
}

/// Every field name and text the native file stores, anywhere in `value`.
fn saved_names(value: &ciborium::Value, names: &mut std::collections::BTreeSet<String>) {
    match value {
        ciborium::Value::Text(text) => {
            names.insert(text.clone());
        }
        ciborium::Value::Array(items) => items.iter().for_each(|item| saved_names(item, names)),
        ciborium::Value::Map(entries) => {
            for (key, value) in entries {
                saved_names(key, names);
                saved_names(value, names);
            }
        }
        ciborium::Value::Tag(_, inner) => saved_names(inner, names),
        _ => {}
    }
}

#[test]
fn complete_view_names_every_field_and_text_each_committed_document_stores() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let mut documents = Vec::new();
    for directory in ["crates", "examples"] {
        committed_documents(&root.join(directory), &mut documents);
    }
    assert!(documents.len() >= 10, "{documents:?}");
    let mut checked_names = 0;
    for path in documents {
        let snapshot = persistence::load(&std::fs::read(&path).unwrap())
            .unwrap()
            .snapshot();
        let mut names = std::collections::BTreeSet::new();
        let _ =
            ketchup_core::testing::rewrite_saved_snapshot(&persistence::save(&snapshot), |saved| {
                saved_names(
                    ketchup_core::testing::cbor_entry(saved, "product"),
                    &mut names,
                );
            });
        let complete = encode_semantic_state(&snapshot).complete();
        for name in &names {
            assert!(
                complete.contains(name.as_str()) || complete.contains(&format!("{name:?}")),
                "{}: the complete view omits {name:?}",
                path.display()
            );
        }
        checked_names += names.len();
    }
    assert!(
        checked_names > 500,
        "only {checked_names} saved names checked"
    );
}
