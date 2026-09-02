#[cfg(feature = "named-product-fixtures")]
use ketchup_core::beam_m4ae::BeamWorkspace;
use ketchup_core::document::{
    CanonicalCommand, CanonicalOverride, CollectionId, CommandBatch, DefinitionId, DerivedIdentity,
    Dimension, DocumentStore, EvaluationIdentity, FeatureId, FeatureKind, GroupId, NodeId,
    OccurrenceId, OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution,
    SlotSegment, TagId, Transform,
};
#[cfg(feature = "named-product-fixtures")]
use ketchup_core::state_view::encode_semantic_state_with_results;
use ketchup_core::state_view::{
    AGENT_STATE_VIEW_V1, COMPLETE_STATE_VIEW_V1, encode_semantic_state,
    encode_semantic_state_with_evaluation,
};
use std::path::PathBuf;

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
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [600.0, 0.0], [600.0, 580.0], [0.0, 580.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(12),
            definition_id: DefinitionId(10),
            name: "Extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(11),
                height: Dimension::new("720.000", 720.0).unwrap(),
            },
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
fn complete_and_agent_v1_match_independently_versioned_golden_fixtures() {
    assert_ne!(COMPLETE_STATE_VIEW_V1, AGENT_STATE_VIEW_V1);
    let state = encode_semantic_state(&fixture_document(false).current());
    let complete = state.complete_v1();
    let agent = state.agent_v1();

    assert_golden("complete-v1.txt", &complete);
    assert!(!complete.contains("<durable>"));
    assert!(
        !complete.contains(&format!(
            "source.document_id={}",
            fixture_document(false).current().document_id().0
        )) || complete.contains("source.document_id=")
    );
    assert_golden("agent-v1.txt", &agent);
    assert!(complete.contains("height.f64_bits=4086800000000000"));
    assert!(complete.contains("occurrence.30.transform.f64_bits="));
    assert!(!agent.contains("transform.f64_bits"));
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
                !line.starts_with("source.document_id=")
                    && !line.starts_with("source.canonical_digest=")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        normalize(encode_semantic_state(&first.current()).complete_v1()),
        normalize(encode_semantic_state(&reordered.current()).complete_v1())
    );

    let before = encode_semantic_state(&first.current()).complete_v1();
    let mut changed = first;
    changed
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(31),
                visible: true,
            },
        ]))
        .unwrap();
    let after = encode_semantic_state(&changed.current()).complete_v1();
    assert_ne!(before, after);
    assert!(after.contains("occurrence.31.visible=true"));
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
    let complete = state.complete_v1();
    let agent = state.agent_v1();

    for expected in [
        "evaluator_node.1.kind=parameter",
        "evaluator_node.1.source=\"4.5\"",
        "evaluator_node.1.dependencies=[]",
        "evaluator_node.1.output_port.0.name=\"value\"",
        "evaluator_node.1.output_port.0.type=number",
        "evaluator_node.2.kind=expression",
        "evaluator_node.2.source=\"$1 * 2\"",
        "evaluator_node.2.dependencies=[1]",
        "evaluator_node.2.input_port.0.name=\"node_1\"",
        "evaluator_node.2.input_port.0.type=number",
        "evaluator_node.2.output_port.0.name=\"value\"",
        "evaluator_node.2.output_port.0.type=number",
        "evaluator_node.3.kind=rule",
        "evaluator_node.3.source=\"$2 + 1\"",
        "evaluator_node.3.dependencies=[2]",
        "evaluator_node.3.input_port.0.name=\"source\"",
        "evaluator_node.3.input_port.0.type=number",
        "evaluator_node.3.output_port.0.name=\"items\"",
        "evaluator_node.3.output_port.0.type=number",
        "evaluator_node.3.rule_output.1.slot_path=3:\"items\":\"cabinet\"/3:\"items\":\"drawer\"",
        "evaluator_node.3.override_parameter.\"offset\".merge_policy=replace",
        "override.7.target.root=3",
        "override.7.target.slot_path=3:\"items\":\"cabinet\"/3:\"items\":\"drawer\"",
        "override.7.parameter=\"offset\"",
        "override.7.health=resolved",
        "evaluation.evaluator=\"state-view-test-evaluator\"",
        "evaluation.schema=\"state-view-test-schema\"",
        "evaluation.tolerance=\"state-view-test-tolerance\"",
        "evaluation.backend=Some(\"state-view-test-backend\")",
        "evaluation.current=true",
        "evaluation.recomputed_nodes=[1,2,3]",
        "derived_output.1.slot_path=3:\"items\":\"cabinet\"/3:\"items\":\"drawer\"",
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
    assert!(complete.contains(&format!("evaluation.revision={revision}")));
    assert!(complete.contains(&format!("evaluation.canonical_digest={canonical_digest}")));
    assert!(complete.contains(&format!(
        "override.7.value.f64_bits={:016x}",
        6.25_f64.to_bits()
    )));

    for (id, value) in [(1, 4.5_f64), (2, 9.0_f64), (3, 10.0_f64)] {
        let result = report.node(NodeId(id)).unwrap();
        assert!(complete.contains(&format!(
            "evaluator_node.{id}.evaluation.input_digest={}",
            result.input_digest
        )));
        assert!(complete.contains(&format!(
            "evaluator_node.{id}.evaluation.result_digest={}",
            result.result_digest
        )));
        assert!(complete.contains(&format!(
            "evaluator_node.{id}.evaluation.status=evaluated:{:016x}",
            value.to_bits()
        )));
    }

    assert!(agent.contains(
        "summary.counts=evaluator_nodes:3,overrides:1,parameter_bindings:0,spaces:0,clearance_volumes:0,persistent_dimensions:0,tags:0,collections:0,definitions:0,features:0,occurrences:0,grounded_occurrences:0,assembly_mates:0,groups:0,local_groups:0,local_occurrences:0"
    ));
    assert!(agent.contains("evaluation.current=true"));
    assert!(!agent.contains("evaluation=not_supplied"));

    assert_eq!(snapshot.canonical_digest(), canonical_digest);
    assert_eq!(snapshot.revision_id(), revision);
    assert_eq!(store.current().canonical_digest(), canonical_digest);
    assert_eq!(store.current().revision_id(), revision);
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn complete_and_agent_views_report_exact_and_tolerant_validation_counts_separately() {
    let workspace = BeamWorkspace::load().unwrap();
    let snapshot = workspace.snapshot();
    let report = &workspace.slice().validation_report;
    let state = encode_semantic_state_with_results(&snapshot, None, Some(report));

    for projection in [state.complete_v1(), state.agent_v1()] {
        assert!(projection.contains("validation.evidence.exact_count=12"));
        assert!(projection.contains("validation.evidence.tolerant_count=0"));
    }
}
