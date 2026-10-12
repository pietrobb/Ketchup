use ketchup_assistant::intent::{
    IntentCapability, IntentError, IntentGrant, IntentRequest, RequestingPrincipal, WorkflowIntent,
    propose_intent,
};
use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use ketchup_model::document::{
    AuthenticatedApprover, AuthoritativeDependency, CanonicalCommand, CollectionId, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, GroupId, HighRiskClass,
    HighRiskScope, HumanConfirmationError, MAX_HUMAN_CONFIRMATION_LIFETIME_MS, NodeId,
    OccurrenceId, PortSpec, Proposal, ProposalBudget, ProposalCommitError, ProposalConfirmation,
    ProposalContext, ProposalGoal, ProposalPrepareError, ProposalPrincipal, ProposalRisk,
    ProposalValue, RuleOutput, SlotSegment, TagId, Transform, TrustedConfirmationSurface,
};

const RULE: NodeId = NodeId(1);
const EXPRESSION_INPUT: NodeId = NodeId(2);
const EXPRESSION: NodeId = NodeId(3);
const RULE_OUTPUTS: NodeId = NodeId(4);
const DEFINITION: DefinitionId = DefinitionId(10);
const SECOND_DEFINITION: DefinitionId = DefinitionId(15);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const OCCURRENCE: OccurrenceId = OccurrenceId(13);
const TAG: TagId = TagId(14);
const GROUP: GroupId = GroupId(16);
const COLLECTION: CollectionId = CollectionId(17);

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
            CanonicalCommand::CreateEvaluatorNode {
                id: EXPRESSION_INPUT,
                name: "allowance".to_owned(),
                dimension: dimension("25", 25.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: EXPRESSION,
                name: "derived width".to_owned(),
                expression: "$1 * 2".to_owned(),
            },
            CanonicalCommand::CreateRuleNode {
                id: RULE_OUTPUTS,
                name: "layout".to_owned(),
                expression: "$3".to_owned(),
                input_ports: vec![PortSpec::number("source").unwrap()],
                output_ports: vec![PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(RULE_OUTPUTS, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateDefinition {
                id: SECOND_DEFINITION,
                name: "Housing".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::extrusion(PROFILE, dimension("20", 20.0)),
            },
            CanonicalCommand::CreateTag {
                id: TAG,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateCollection {
                id: COLLECTION,
                name: "Selection set".to_owned(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Box occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    store
}

fn high_risk_proposal(
    store: &DocumentStore,
    principal: ProposalPrincipal,
    value_text: &str,
    scope: HighRiskScope,
) -> Proposal {
    store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal(value_text).unwrap(),
            }]),
            ProposalContext {
                principal,
                goal: ProposalGoal::SetFeatureDimension(EXTRUSION),
                assumptions: vec![],
                risk: ProposalRisk::High(scope.class()),
                confirmation: ProposalConfirmation::HumanOnly(scope),
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            },
        )
        .unwrap()
}

#[test]
fn rule_intent_exposes_authoritative_review_and_commits_one_verified_batch() {
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
        &ProposalConfirmation::ReviewRequired
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
fn profile_points_are_observational_typed_and_undoable() {
    let mut store = seed();
    let revision_before = store.current().revision_id();
    let digest_before = store.current().canonical_digest();
    let undo_before = store.visible_undo_steps();
    let requested = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetProfilePoints {
            target: PROFILE,
            points_mm: requested.clone(),
        }),
    )
    .unwrap();

    assert_eq!(proposal.goal(), ProposalGoal::SetProfilePoints(PROFILE));
    assert_eq!(
        proposal.assumptions(),
        &[ketchup_model::document::ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(PROFILE)
        )]
    );
    assert_eq!(
        proposal.authoritative_writes(),
        &std::collections::BTreeSet::from([AuthoritativeDependency::Feature(PROFILE)])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ProfilePoints(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ProfilePoints(requested.clone())
    );
    assert_eq!(store.current().revision_id(), revision_before);
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.visible_undo_steps(), undo_before);

    store.commit_verified_proposal(&proposal).unwrap();
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &requested
    ));
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    store.undo().unwrap();
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm)
            if points_mm == &vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]
    ));
}

#[test]
fn profile_points_reject_denied_invalid_wrong_kind_missing_and_stale() {
    let mut store = seed();
    let digest_before = store.current().canonical_digest();
    let requested = vec![[0.0, 0.0], [12.0, 0.0], [0.0, 8.0]];
    assert_eq!(
        propose_intent(
            &store,
            IntentRequest {
                grant: IntentGrant::new(
                    RequestingPrincipal::LocalAssistant,
                    [IntentCapability::SetFeatureDimension],
                ),
                intent: WorkflowIntent::SetProfilePoints {
                    target: PROFILE,
                    points_mm: requested.clone(),
                },
                requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
            }
        ),
        Err(IntentError::CapabilityDenied(
            IntentCapability::SetProfilePoints
        ))
    );
    for (target, points_mm) in [
        (PROFILE, vec![[0.0, 0.0], [1.0, 0.0]]),
        (EXTRUSION, requested.clone()),
        (FeatureId(999), requested.clone()),
    ] {
        assert!(matches!(
            propose_intent(
                &store,
                IntentRequest::m7a(WorkflowIntent::SetProfilePoints { target, points_mm })
            ),
            Err(IntentError::Proposal(ProposalPrepareError::Canonical(_)))
        ));
    }
    assert_eq!(store.current().canonical_digest(), digest_before);

    let proposal = propose_intent(
        &store,
        IntentRequest::m7a(WorkflowIntent::SetProfilePoints {
            target: PROFILE,
            points_mm: requested,
        }),
    )
    .unwrap();
    let concurrent = vec![[0.0, 0.0], [11.0, 0.0], [0.0, 9.0]];
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: concurrent.clone(),
            },
        ]))
        .unwrap();
    let changed_digest = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store.current().canonical_digest(), changed_digest);
    assert!(matches!(
        store.current().feature(PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm) if points_mm == &concurrent
    ));
}

#[test]
fn feature_intent_uses_the_same_safe_path_and_is_not_replayable() {
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
    let FeatureKind::Pad(PadSpec {
        profile: PadProfile::Feature(_),
        extent: FeatureExtent::Blind(height),
        operation: PadOperation::NewBody,
        ..
    }) = snapshot.feature(EXTRUSION).unwrap().kind()
    else {
        panic!("fixture extrusion changed kind");
    };
    assert_eq!(height.millimetres(), 35.0);
}

#[test]
fn revalidates_unrelated_edits_but_rejects_relevant_changes() {
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
fn capability_value_and_budget_failures_leave_the_document_unchanged() {
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
    assert!(matches!(
        propose_intent(&store, insufficient),
        Err(IntentError::Proposal(
            ProposalPrepareError::RequestedBudgetExceeded { budget, cost }
        )) if budget.max_commands == 0 && cost.commands == 1
    ));

    assert_eq!(store.current().canonical_digest(), digest);
    assert_eq!(store.revision_count(), revisions);
}

#[test]
fn invalid_target_and_cross_document_proposal_fail_closed() {
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

#[test]
fn high_risk_commit_requires_distinct_authenticated_human_and_consumes_token_once() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([7; 32], 1).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 1)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.example.test:443".to_owned()),
        Some("example-cloud".to_owned()),
        None,
    )
    .unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::LocalAssistant, "31", scope);
    let digest_before = store.current().canonical_digest();

    assert!(matches!(
        store.commit_verified_proposal(&proposal),
        Err(ProposalCommitError::HumanApprovalRequired)
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(
        authority.issue(
            &proposal,
            AuthenticatedApprover::Machine(ProposalPrincipal::Plugin(9)),
            1_000,
            2_000,
        ),
        Err(HumanConfirmationError::MachineCannotApprove)
    );

    let approval = authority
        .issue(&proposal, AuthenticatedApprover::Human(77), 1_000, 2_000)
        .unwrap();
    let undo_before = store.visible_undo_steps();
    let committed = store
        .commit_high_risk_proposal(&proposal, &approval, 1_500)
        .unwrap();
    assert_eq!(committed.command_digest(), proposal.command_digest());
    assert_eq!(store.visible_undo_steps(), undo_before + 1);
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 1_500),
        Err(ProposalCommitError::HumanApprovalReplayed)
    ));
}

#[test]
fn high_risk_token_fails_closed_on_scope_signature_expiry_and_policy_change() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([11; 32], 4).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 4)
        .unwrap();
    let first_scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.first.test:443".to_owned()),
        Some("first-provider".to_owned()),
        None,
    )
    .unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::Plugin(41), "32", first_scope);
    let approval = authority
        .issue(&proposal, AuthenticatedApprover::Human(88), 10_000, 11_000)
        .unwrap();
    let digest_before = store.current().canonical_digest();

    let other_scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some("api.second.test:443".to_owned()),
        Some("second-provider".to_owned()),
        None,
    )
    .unwrap();
    let other_proposal =
        high_risk_proposal(&store, ProposalPrincipal::Plugin(41), "32", other_scope);
    assert!(matches!(
        store.commit_high_risk_proposal(&other_proposal, &approval, 10_500),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));

    let untrusted = TrustedConfirmationSurface::new([12; 32], 4).unwrap();
    let wrong_signature = untrusted
        .issue(&proposal, AuthenticatedApprover::Human(88), 10_000, 11_000)
        .unwrap();
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &wrong_signature, 10_500),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 11_001),
        Err(ProposalCommitError::HumanApprovalInvalid)
    ));

    store
        .configure_human_confirmation_policy(authority.verifying_key(), 5)
        .unwrap();
    assert!(matches!(
        store.commit_high_risk_proposal(&proposal, &approval, 10_500),
        Err(ProposalCommitError::HumanApprovalPolicyStale)
    ));
    assert_eq!(store.current().canonical_digest(), digest_before);
}

#[test]
fn high_risk_preparation_and_issuance_reject_ambiguous_identity_or_scope() {
    assert_eq!(
        HighRiskScope::new(
            HighRiskClass::ExternalDisclosure,
            Some("api.example.test:443".to_owned()),
            None,
            None,
        ),
        Err(HumanConfirmationError::InvalidScope)
    );

    let store = seed();
    let overwrite = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let unidentified = store.prepare_proposal_with_context(
        CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
            id: EXTRUSION,
            dimension: Dimension::from_decimal("33").unwrap(),
        }]),
        ProposalContext {
            principal: ProposalPrincipal::ManualClient,
            goal: ProposalGoal::SetFeatureDimension(EXTRUSION),
            assumptions: vec![],
            risk: ProposalRisk::High(HighRiskClass::Overwrite),
            confirmation: ProposalConfirmation::HumanOnly(overwrite),
            requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
        },
    );
    assert!(matches!(
        unidentified,
        Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::UnidentifiedRequester
        ))
    ));

    let scope = HighRiskScope::new(HighRiskClass::CapabilityExpansion, None, None, None).unwrap();
    let proposal = high_risk_proposal(&store, ProposalPrincipal::Human(55), "33", scope);
    let authority = TrustedConfirmationSurface::new([13; 32], 1).unwrap();
    assert_eq!(
        authority.issue(&proposal, AuthenticatedApprover::Human(55), 1_000, 2_000,),
        Err(HumanConfirmationError::RequesterCannotApprove)
    );
    assert_eq!(
        authority.issue(
            &proposal,
            AuthenticatedApprover::Human(56),
            1_000,
            1_000 + MAX_HUMAN_CONFIRMATION_LIFETIME_MS + 1,
        ),
        Err(HumanConfirmationError::InvalidLifetime)
    );
}

#[test]
fn side_effect_receipt_is_payload_bound_one_use_and_non_canonical() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([17; 32], 3).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 3)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let proposal = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope,
            b"exact bytes displayed for overwrite",
        )
        .unwrap();
    let approval = authority
        .issue_side_effect(&proposal, AuthenticatedApprover::Human(91), 20_000, 21_000)
        .unwrap();
    let digest_before = store.current().canonical_digest();
    let revision_count_before = store.revision_count();
    let undo_before = store.visible_undo_steps();

    let receipt = store
        .authorize_high_risk_side_effect(&proposal, &approval, 20_500)
        .unwrap();
    assert_eq!(receipt.approving_human(), 91);
    assert_eq!(receipt.document_id(), proposal.document_id());
    assert_eq!(receipt.revision_id(), proposal.provenance_revision());
    assert_eq!(receipt.operation(), "overwrite-native-document");
    assert_eq!(receipt.operation_digest(), proposal.operation_digest());
    assert_eq!(receipt.payload_digest(), proposal.payload_digest());
    assert_eq!(receipt.scope(), proposal.scope());
    assert_eq!(store.current().canonical_digest(), digest_before);
    assert_eq!(store.revision_count(), revision_count_before);
    assert_eq!(store.visible_undo_steps(), undo_before);
    assert_eq!(
        store.authorize_high_risk_side_effect(&proposal, &approval, 20_500),
        Err(ketchup_model::document::SideEffectAuthorizationError::Replayed)
    );
}

#[test]
fn side_effect_authorization_rejects_payload_substitution_and_stale_snapshot() {
    let mut store = seed();
    let authority = TrustedConfirmationSurface::new([19; 32], 2).unwrap();
    store
        .configure_human_confirmation_policy(authority.verifying_key(), 2)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::Overwrite,
        None,
        None,
        Some("C:/models/part.ketchup".to_owned()),
    )
    .unwrap();
    let original = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope.clone(),
            b"approved bytes",
        )
        .unwrap();
    let approval = authority
        .issue_side_effect(&original, AuthenticatedApprover::Human(92), 30_000, 31_000)
        .unwrap();
    let substituted = store
        .prepare_high_risk_side_effect(
            "overwrite-native-document",
            ProposalPrincipal::LocalAssistant,
            scope,
            b"different bytes",
        )
        .unwrap();
    assert_eq!(
        store.authorize_high_risk_side_effect(&substituted, &approval, 30_500),
        Err(ketchup_model::document::SideEffectAuthorizationError::Invalid)
    );

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(99),
                name: "Changed after review".to_owned(),
            },
        ]))
        .unwrap();
    assert_eq!(
        store.authorize_high_risk_side_effect(&original, &approval, 30_500),
        Err(ketchup_model::document::SideEffectAuthorizationError::Invalid)
    );
}
