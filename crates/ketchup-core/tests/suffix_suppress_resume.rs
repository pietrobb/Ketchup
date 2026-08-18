use ketchup_core::document::{
    BodyId, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, ProposalCommitError, ProposalPrincipal,
};
use ketchup_core::exact_product::{
    ExactFeatureChainRequest, ExactResultRegistry, exact_body_terminal_features,
};
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationError, BodyHistoryMutationRequest, FeatureHistoryQuery,
    FeatureHistoryState, prepare_body_history_mutation, project_feature_history,
};
use ketchup_core::persistence;
use std::collections::BTreeSet;

const DEFINITION: DefinitionId = DefinitionId(1);
const BASE_PROFILE: FeatureId = FeatureId(10);
const BASE_EXTRUSION: FeatureId = FeatureId(11);
const CUT_PROFILE: FeatureId = FeatureId(12);
const POCKET: FeatureId = FeatureId(13);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);

fn profile(size: f64) -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]],
    }
}

fn seed_history() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Rollback part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base profile".to_owned(),
                kind: profile(20.0),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: Dimension::from_decimal("8").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[4.0, 4.0], [8.0, 4.0], [8.0, 8.0], [4.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: BASE_EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::from_decimal("3").unwrap(),
                },
            },
            CanonicalCommand::CreateBody {
                definition_id: DEFINITION,
                id: BodyId(2),
                name: "Unrelated body".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(2),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_PROFILE,
                definition_id: DEFINITION,
                name: "Tool profile".to_owned(),
                kind: profile(2.0),
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Tool extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: Dimension::from_decimal("2").unwrap(),
                },
            },
            CanonicalCommand::SetActiveBody {
                definition_id: DEFINITION,
                id: BodyId(1),
            },
        ]))
        .unwrap();
    document
}

fn request(mutation: BodyHistoryMutation) -> BodyHistoryMutationRequest {
    BodyHistoryMutationRequest {
        definition_id: DEFINITION,
        body_id: BodyId(1),
        mutation,
    }
}

fn stamp(document: &DocumentStore) -> (u64, String, usize, usize) {
    (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    )
}

#[test]
fn suppress_suffix_is_reviewed_atomic_and_body_scoped() {
    let mut document = seed_history();
    let before = stamp(&document);
    let snapshot = document.current();
    let unrelated_body = snapshot
        .definition(DEFINITION)
        .unwrap()
        .body(BodyId(2))
        .unwrap()
        .clone();
    let unrelated_profile = snapshot.feature(TOOL_PROFILE).unwrap().clone();
    let unrelated_extrusion = snapshot.feature(TOOL_EXTRUSION).unwrap().clone();
    assert_eq!(
        exact_body_terminal_features(&snapshot, DEFINITION)
            .unwrap()
            .get(&BodyId(1)),
        Some(&POCKET)
    );
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_body(&snapshot, DEFINITION, BodyId(1))
            .unwrap()
            .pocket_depth_bits
            .is_some()
    );

    let manual = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    let assistant = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    assert_eq!(manual.proposal.batch(), assistant.proposal.batch());
    assert_eq!(manual.suppressed_feature_ids, vec![CUT_PROFILE, POCKET]);
    assert_eq!(manual.affected_feature_ids, vec![CUT_PROFILE, POCKET]);
    assert_eq!(manual.unchanged_body_ids, vec![BodyId(2)]);
    assert_eq!(stamp(&document), before);

    let committed = document.commit_proposal(&manual.proposal).unwrap();
    assert_eq!(
        committed.dirty_features(),
        &BTreeSet::from([CUT_PROFILE, POCKET])
    );
    assert_eq!(document.visible_undo_steps(), before.2 + 1);
    let after = document.current();
    assert_eq!(
        after.suppressed_feature_ids(DEFINITION, BodyId(1)),
        Some(&BTreeSet::from([CUT_PROFILE, POCKET]))
    );
    assert_eq!(
        exact_body_terminal_features(&after, DEFINITION)
            .unwrap()
            .get(&BodyId(1)),
        Some(&BASE_EXTRUSION)
    );
    let rolled_back =
        ExactFeatureChainRequest::from_snapshot_for_body(&after, DEFINITION, BodyId(1)).unwrap();
    assert_eq!(rolled_back.producer_feature_id(), BASE_EXTRUSION);
    assert!(rolled_back.pocket_depth_bits.is_none());
    assert_eq!(
        after.definition(DEFINITION).unwrap().body(BodyId(2)),
        Some(&unrelated_body)
    );
    assert_eq!(after.feature(TOOL_PROFILE), Some(&unrelated_profile));
    assert_eq!(after.feature(TOOL_EXTRUSION), Some(&unrelated_extrusion));

    let history = project_feature_history(
        &after,
        &ExactResultRegistry::default(),
        DEFINITION,
        &FeatureHistoryQuery::default(),
    )
    .unwrap();
    assert_eq!(
        history.bodies[0]
            .features
            .iter()
            .map(|entry| entry.state)
            .collect::<Vec<_>>(),
        vec![
            FeatureHistoryState::Active,
            FeatureHistoryState::Active,
            FeatureHistoryState::RollbackSuppressed,
            FeatureHistoryState::RollbackSuppressed,
        ]
    );
    assert!(
        history.bodies[1]
            .features
            .iter()
            .all(|entry| entry.state == FeatureHistoryState::Active)
    );

    let reopened = persistence::load(&persistence::save(&after)).unwrap();
    let reopened = reopened.snapshot();
    assert_eq!(reopened.canonical_digest(), after.canonical_digest());
    assert_eq!(
        reopened.suppressed_feature_ids(DEFINITION, BodyId(1)),
        after.suppressed_feature_ids(DEFINITION, BodyId(1))
    );
    assert_eq!(reopened.feature(CUT_PROFILE), after.feature(CUT_PROFILE));
    assert_eq!(reopened.feature(POCKET), after.feature(POCKET));
}

#[test]
fn resume_restores_the_suffix_with_one_undo_and_redo() {
    let mut document = seed_history();
    let suppression = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document.commit_proposal(&suppression.proposal).unwrap();
    let suppressed_digest = document.current().canonical_digest();
    let suppressed_undo_steps = document.visible_undo_steps();

    let resume = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::Resume),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    assert!(resume.suppressed_feature_ids.is_empty());
    assert_eq!(resume.affected_feature_ids, vec![CUT_PROFILE, POCKET]);
    let committed = document.commit_proposal(&resume.proposal).unwrap();
    assert_eq!(
        committed.dirty_features(),
        &BTreeSet::from([CUT_PROFILE, POCKET])
    );
    assert_eq!(document.visible_undo_steps(), suppressed_undo_steps + 1);
    assert_eq!(
        document
            .current()
            .suppressed_feature_ids(DEFINITION, BodyId(1)),
        None
    );
    let resumed = document.current();
    assert_eq!(
        exact_body_terminal_features(&resumed, DEFINITION)
            .unwrap()
            .get(&BodyId(1)),
        Some(&POCKET)
    );
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_body(&resumed, DEFINITION, BodyId(1))
            .unwrap()
            .pocket_depth_bits
            .is_some()
    );

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), suppressed_digest);
    assert_eq!(
        document
            .current()
            .suppressed_feature_ids(DEFINITION, BodyId(1)),
        Some(&BTreeSet::from([CUT_PROFILE, POCKET]))
    );
    document.redo().unwrap();
    assert_eq!(
        document
            .current()
            .suppressed_feature_ids(DEFINITION, BodyId(1)),
        None
    );
}

#[test]
fn invalid_duplicate_cross_body_and_interior_requests_are_non_mutating() {
    let mut document = seed_history();
    let before = stamp(&document);

    assert!(matches!(
        prepare_body_history_mutation(
            &document,
            request(BodyHistoryMutation::SuppressFrom(TOOL_EXTRUSION)),
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyHistoryMutationError::History(_))
    ));
    for invalid in [
        vec![BASE_EXTRUSION, POCKET],
        vec![CUT_PROFILE, CUT_PROFILE, POCKET],
        vec![TOOL_EXTRUSION],
    ] {
        assert!(matches!(
            document.preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetBodyFeatureSuppression {
                    definition_id: DEFINITION,
                    body_id: BodyId(1),
                    suppressed_feature_ids: invalid,
                },
            ])),
            Err(CanonicalError::InvalidFeatureSuppression(
                DEFINITION,
                BodyId(1)
            ))
        ));
    }
    assert_eq!(stamp(&document), before);

    let proposal = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::ManualClient,
    )
    .unwrap();
    document.commit_proposal(&proposal.proposal).unwrap();
    let suppressed = stamp(&document);
    assert_eq!(
        prepare_body_history_mutation(
            &document,
            request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
            ProposalPrincipal::ManualClient,
        ),
        Err(BodyHistoryMutationError::Unchanged(BodyId(1)))
    );
    assert_eq!(stamp(&document), suppressed);
}

#[test]
fn stale_suppression_proposal_cannot_partially_publish() {
    let mut document = seed_history();
    let proposal = prepare_body_history_mutation(
        &document,
        request(BodyHistoryMutation::SuppressFrom(CUT_PROFILE)),
        ProposalPrincipal::LocalAssistant,
    )
    .unwrap();
    let unrelated = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetBodyVisibility {
                definition_id: DEFINITION,
                id: BodyId(2),
                visible: false,
            },
        ]))
        .unwrap();
    document.commit_proposal(&unrelated).unwrap();
    let before_rejection = stamp(&document);

    assert!(matches!(
        document.commit_proposal(&proposal.proposal),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_rejection);
    assert_eq!(
        document
            .current()
            .suppressed_feature_ids(DEFINITION, BodyId(1)),
        None
    );
}
