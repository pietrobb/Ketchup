use ketchup_core::document::{
    AuthoritativeDependency, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, NodeId, ProposalBudget, ProposalCommitError,
    ProposalConfirmation, ProposalGoal, ProposalPrepareError, ProposalPrincipal, ProposalRisk,
    ProposalValue,
};
use ketchup_core::intent::{
    IntentCapability, IntentError, IntentGrant, IntentRequest, RequestingPrincipal, WorkflowIntent,
    propose_intent,
};

const RULE: NodeId = NodeId(1);
const DEFINITION: DefinitionId = DefinitionId(10);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);

fn dimension(token: &str, value: f64) -> Dimension {
    Dimension::new(token, value).unwrap()
}

fn seed() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: RULE,
                name: "width".to_owned(),
                dimension: dimension("600", 600.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: dimension("20", 20.0),
                },
            },
        ]))
        .unwrap();
    store
}

#[test]
fn gate_d_rule_intent_exposes_authoritative_review_and_commits_one_verified_batch() {
    let mut store = seed();
    let undo_before = store.visible_undo_steps();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "720.125".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.principal(), ProposalPrincipal::LocalAssistant);
    assert_eq!(proposal.goal(), ProposalGoal::SetRuleDimension(RULE));
    assert_eq!(proposal.risk(), ProposalRisk::Standard);
    assert_eq!(
        proposal.confirmation(),
        ProposalConfirmation::ReviewRequired
    );
    assert_eq!(proposal.cost().commands, 1);
    assert_eq!(proposal.cost().write_targets, 1);
    assert!(proposal.cost().read_dependencies <= 64);
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::EvaluatorNode(RULE)])
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    assert!(matches!(
        &proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(value) if value.millimetres() == 600.0
    ));
    assert!(matches!(
        &proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(value) if value.millimetres() == 720.125
    ));

    let committed = store.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(committed.command_digest(), proposal.command_digest());
    assert_eq!(committed.result_digest(), proposal.intended_result_digest());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        store
            .current()
            .evaluator_node(RULE)
            .unwrap()
            .dimension()
            .unwrap()
            .millimetres(),
        720.125
    );
}

#[test]
fn gate_d_feature_intent_uses_the_same_safe_path_and_is_not_replayable() {
    let mut store = seed();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "35".to_owned(),
        }),
    )
    .unwrap();

    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetFeatureDimension(EXTRUSION)
    );
    assert_eq!(proposal.authoritative_diff().len(), 1);
    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    let snapshot = store.current();
    let FeatureKind::Extrusion { height, .. } = snapshot.feature(EXTRUSION).unwrap().kind() else {
        panic!("fixture extrusion changed kind");
    };
    assert_eq!(height.millimetres(), 35.0);
}

#[test]
fn gate_d_revalidates_unrelated_edits_but_rejects_relevant_changes() {
    let mut store = seed();
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(99),
                name: "Unrelated".to_owned(),
            },
        ]))
        .unwrap();
    store.commit_verified_proposal(&proposal).unwrap();

    let stale = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "40".to_owned(),
        }),
    )
    .unwrap();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]],
            },
        ]))
        .unwrap();
    assert!(matches!(
        store.commit_verified_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
}

#[test]
fn gate_d_capability_value_and_budget_failures_leave_the_document_unchanged() {
    let store = seed();
    let digest = store.current().canonical_digest();
    let revisions = store.revision_count();

    let denied = IntentRequest {
        grant: IntentGrant::new(RequestingPrincipal::LocalAssistant, []),
        intent: WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "700".to_owned(),
        },
        requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
    };
    assert_eq!(
        propose_intent(&store, denied),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetRuleDimension
        ))
    );

    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetFeatureDimension {
                target: EXTRUSION,
                value_text: "not-a-number".to_owned(),
            })
        ),
        Err(IntentError::Canonical(_))
    ));

    let oversized = IntentRequest {
        grant: IntentGrant::m7a_local_assistant(),
        intent: WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        },
        requested_budget: ProposalBudget {
            max_commands: ProposalBudget::HOST_MAX.max_commands + 1,
            max_read_dependencies: 64,
            max_write_targets: 1,
        },
    };
    assert_eq!(
        propose_intent(&store, oversized),
        Err(IntentError::Proposal(
            ProposalPrepareError::HostBudgetExceeded
        ))
    );

    let insufficient = IntentRequest {
        grant: IntentGrant::m7a_local_assistant(),
        intent: WorkflowIntent::SetFeatureDimension {
            target: EXTRUSION,
            value_text: "30".to_owned(),
        },
        requested_budget: ProposalBudget {
            max_commands: 0,
            max_read_dependencies: 64,
            max_write_targets: 1,
        },
    };
    assert_eq!(
        propose_intent(&store, insufficient),
        Err(IntentError::Proposal(
            ProposalPrepareError::RequestedBudgetExceeded
        ))
    );

    assert_eq!(store.current().canonical_digest(), digest);
    assert_eq!(store.revision_count(), revisions);
}

#[test]
fn gate_d_invalid_target_and_cross_document_proposal_fail_closed() {
    let store = seed();
    assert!(matches!(
        propose_intent(
            &store,
            IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
                target: NodeId(999),
                value_text: "10".to_owned(),
            })
        ),
        Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
    ));

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetRuleDimension {
            target: RULE,
            value_text: "700".to_owned(),
        }),
    )
    .unwrap();
    let mut other = seed();
    let other_digest = other.current().canonical_digest();
    assert!(matches!(
        other.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(other.current().canonical_digest(), other_digest);
}
