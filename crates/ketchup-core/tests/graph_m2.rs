use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CanonicalOverride, CommandBatch, Dimension, DocumentStore,
    EvaluationIdentity, EvaluationStatus, GraphError, NodeId, OverrideParameterSpec, PortSpec,
    RuleOutput, SlotPath, SlotResolution, SlotSegment,
};
use ketchup_core::graph::DiagnosticCode;
use ketchup_core::persistence;

fn segment(key: &str) -> SlotSegment {
    SlotSegment::new(NodeId(4), "result", key).unwrap()
}

fn path(key: &str) -> SlotPath {
    SlotPath::new(vec![segment(key)]).unwrap()
}

fn seeded(outputs: Vec<RuleOutput>) -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "a".into(),
                dimension: Dimension::new("10", 10.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(2),
                name: "b".into(),
                dimension: Dimension::new("3", 3.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: NodeId(3),
                name: "precedence".into(),
                expression: "$1 + $2 * 2".into(),
            },
            CanonicalCommand::CreateRuleNode {
                id: NodeId(4),
                name: "rule".into(),
                expression: "$3 / 2".into(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs,
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(5),
                name: "unrelated".into(),
                dimension: Dimension::new("99", 99.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    store
}

#[test]
fn parser_is_bounded_respects_precedence_and_reports_diagnostics_and_cycles() {
    let mut store = seeded(vec![RuleOutput::new(segment("left"), vec![]).unwrap()]);
    let report = store
        .current()
        .evaluate(&EvaluationIdentity::default())
        .unwrap();
    assert_eq!(
        report.node(NodeId(3)).unwrap().status,
        EvaluationStatus::Evaluated(16.0)
    );
    assert_eq!(
        report.node(NodeId(4)).unwrap().status,
        EvaluationStatus::Evaluated(8.0)
    );
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateExpressionNode {
                id: NodeId(6),
                name: "bad".into(),
                expression: "$1 / 0".into(),
            },
        ]))
        .unwrap();
    let report = store
        .current()
        .evaluate(&EvaluationIdentity::default())
        .unwrap();
    let EvaluationStatus::Error(items) = &report.node(NodeId(6)).unwrap().status else {
        panic!()
    };
    assert_eq!(items[0].code, DiagnosticCode::DivisionByZero);
    let before = store.current().canonical_digest();
    let error = match store.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateExpressionNode {
            id: NodeId(10),
            name: "x".into(),
            expression: "$11".into(),
        },
        CanonicalCommand::CreateExpressionNode {
            id: NodeId(11),
            name: "y".into(),
            expression: "$10".into(),
        },
    ])) {
        Ok(_) => panic!("cycle accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        CanonicalError::Graph(GraphError::DependencyCycle(_))
    ));
    assert_eq!(store.current().canonical_digest(), before);
    let too_deep = format!("{}1{}", "(".repeat(80), ")".repeat(80));
    let error = match store.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateExpressionNode {
            id: NodeId(12),
            name: "deep".into(),
            expression: too_deep,
        },
    ])) {
        Ok(_) => panic!("deep expression accepted"),
        Err(error) => error,
    };
    assert_eq!(error, CanonicalError::Graph(GraphError::ExpressionLimit));
}

#[test]
fn affected_only_recomputation_preserves_unrelated_results_and_identity_fields_change_digests() {
    let mut store = seeded(vec![RuleOutput::new(segment("left"), vec![]).unwrap()]);
    let before = store
        .current()
        .evaluate(&EvaluationIdentity::default())
        .unwrap();
    let unrelated = before.node(NodeId(5)).unwrap().clone();
    let revision = store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(
        revision
            .recomputed_nodes()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![NodeId(1), NodeId(3), NodeId(4)]
    );
    assert_eq!(
        revision.evaluation().unwrap().node(NodeId(5)).unwrap(),
        &unrelated
    );
    let snapshot = store.current();
    let base = EvaluationIdentity::default();
    let base_digest = snapshot
        .evaluate(&base)
        .unwrap()
        .node(NodeId(4))
        .unwrap()
        .input_digest
        .clone();
    for changed in [
        EvaluationIdentity {
            evaluator: "other".into(),
            ..base.clone()
        },
        EvaluationIdentity {
            schema: "other".into(),
            ..base.clone()
        },
        EvaluationIdentity {
            tolerance: "other".into(),
            ..base.clone()
        },
        EvaluationIdentity {
            backend: Some("other".into()),
            ..base.clone()
        },
    ] {
        assert_ne!(
            snapshot
                .evaluate(&changed)
                .unwrap()
                .node(NodeId(4))
                .unwrap()
                .input_digest,
            base_digest
        );
    }
}

#[test]
fn affected_recompute_after_open_skips_an_unrelated_uncached_branch() {
    let seeded = seeded(vec![RuleOutput::new(segment("left"), vec![]).unwrap()]);
    let mut reopened = persistence::load(&persistence::save(&seeded.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();

    let revision = reopened
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetNodeExpression {
                id: NodeId(4),
                expression: "$3 / 4".to_owned(),
            },
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
            },
        ]))
        .unwrap();

    let report = revision.evaluation().unwrap();
    assert_eq!(report.recomputed_nodes, [NodeId(4)].into_iter().collect());
    assert!(report.node(NodeId(1)).is_some());
    assert!(report.node(NodeId(2)).is_some());
    assert!(report.node(NodeId(3)).is_some());
    assert!(report.node(NodeId(5)).is_none());
}

#[test]
fn slot_paths_are_semantic_and_overrides_never_retarget() {
    let left = RuleOutput::new(segment("left"), vec![]).unwrap();
    let right = RuleOutput::new(segment("right"), vec![]).unwrap();
    let mut store = seeded(vec![left.clone(), right.clone()]);
    let target = ketchup_core::document::DerivedIdentity::new(NodeId(4), path("left")).unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertOverride(
            CanonicalOverride::new(
                1,
                target.clone(),
                "offset",
                2.0,
                SlotResolution::Lost { segment_index: 9 },
            )
            .unwrap(),
        )]))
        .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: NodeId(4),
            outputs: vec![right, left],
        }]))
        .unwrap();
    assert_eq!(store.current().override_by_id(1).unwrap().target, target);
    assert_eq!(
        store.current().override_by_id(1).unwrap().health,
        SlotResolution::Resolved
    );
    store
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetRuleOutputs {
            id: NodeId(4),
            outputs: vec![
                RuleOutput::new(segment("left"), vec![]).unwrap(),
                RuleOutput::new(segment("left"), vec![]).unwrap(),
            ],
        }]))
        .unwrap();
    assert_eq!(
        store.current().resolve_slot(&target),
        SlotResolution::Ambiguous { segment_index: 0 }
    );
    assert_eq!(
        store.current().override_by_id(1).unwrap().health,
        SlotResolution::Ambiguous { segment_index: 0 }
    );
    assert_eq!(
        store.current().resolve_slot(
            &ketchup_core::document::DerivedIdentity::new(NodeId(4), path("missing")).unwrap()
        ),
        SlotResolution::Lost { segment_index: 0 }
    );
}

#[test]
fn registration_is_strict_and_canonical_history_is_invariant() {
    let mut store = seeded(vec![RuleOutput::new(segment("left"), vec![]).unwrap()]);
    let snapshot = store.current();
    let report = snapshot.evaluate(&EvaluationIdentity::default()).unwrap();
    let digest = snapshot.canonical_digest();
    let revision = snapshot.revision_id();
    let undo = store.visible_undo_steps();
    store
        .register_evaluation(NodeId(4), path("left"), &report)
        .unwrap();
    assert_eq!(store.current().canonical_digest(), digest);
    assert_eq!(store.current().revision_id(), revision);
    assert_eq!(store.visible_undo_steps(), undo);
    let mut forged = report.clone();
    forged
        .outputs
        .get_mut(&ketchup_core::document::DerivedIdentity::new(NodeId(4), path("left")).unwrap())
        .unwrap()
        .value = 123.0;
    assert_eq!(
        store
            .register_evaluation(NodeId(4), path("left"), &forged)
            .unwrap_err(),
        CanonicalError::EvaluationEvidenceMismatch
    );
    let mut stale = report;
    stale.revision_id = Some(revision + 1);
    assert_eq!(
        store
            .register_evaluation(NodeId(4), path("left"), &stale)
            .unwrap_err(),
        CanonicalError::EvaluationEnvelopeMismatch
    );
}
