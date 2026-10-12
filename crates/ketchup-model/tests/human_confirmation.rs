//! High-risk proposals: a change or side effect that needs a distinct,
//! authenticated human approval, bound to its scope, payload and revision.

use ketchup_model::document::{
    AuthenticatedApprover, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, HighRiskClass, HighRiskScope, HumanConfirmationError,
    MAX_HUMAN_CONFIRMATION_LIFETIME_MS, Proposal, ProposalBudget, ProposalCommitError,
    ProposalConfirmation, ProposalContext, ProposalGoal, ProposalPrepareError, ProposalPrincipal,
    ProposalRisk, TrustedConfirmationSurface,
};

const DEFINITION: DefinitionId = DefinitionId(10);
const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);

fn seed() -> DocumentStore {
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Box".to_owned(),
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
                kind: FeatureKind::extrusion(PROFILE, Dimension::new("20", 20.0).unwrap()),
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
