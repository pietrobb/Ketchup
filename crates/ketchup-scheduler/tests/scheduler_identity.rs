use ketchup_core::document::{DerivedIdentity, NodeId, SlotPath, SlotSegment};
use ketchup_scheduler::{
    AcceptanceIdentity, DerivedResult, EvaluationScheduler, InsertOutcome, JobToken, SchedulerError,
};

const NODE: NodeId = NodeId(7);
const OTHER_NODE: NodeId = NodeId(8);

fn slot_path(root: NodeId, semantic_key: &str) -> SlotPath {
    SlotPath::new(vec![SlotSegment::new(root, "items", semantic_key).unwrap()]).unwrap()
}

fn identity(semantic_key: &str) -> AcceptanceIdentity {
    AcceptanceIdentity {
        document_scope: 42,
        derived_identity: DerivedIdentity::new(NODE, slot_path(NODE, semantic_key)).unwrap(),
        input_digest: "input-v1".to_owned(),
        evaluator: "evaluator-v1".to_owned(),
        backend: Some("backend-v1".to_owned()),
        schema: "schema-v1".to_owned(),
        tolerance: "tolerance-v1".to_owned(),
    }
}

fn result(token: JobToken, fingerprint: &str) -> DerivedResult {
    DerivedResult {
        token,
        result_fingerprint: fingerprint.to_owned(),
        charge_bytes: 64,
    }
}

#[test]
fn simultaneous_slot_paths_are_accepted_and_retrieved_separately() {
    let mut scheduler = EvaluationScheduler::new(1024);
    scheduler.advance_revision(1, [NODE]).unwrap();
    let left = identity("left");
    let right = identity("right");
    let left_token = scheduler
        .schedule_with_identity(NODE, left.clone())
        .unwrap();
    let right_token = scheduler
        .schedule_with_identity(NODE, right.clone())
        .unwrap();

    assert_eq!(
        scheduler.accept(result(left_token, "left-result")),
        InsertOutcome::Current
    );
    assert_eq!(
        scheduler.accept(result(right_token, "right-result")),
        InsertOutcome::Current
    );
    assert_eq!(
        scheduler.current_result_fingerprint_for(NODE, &left),
        Some("left-result")
    );
    assert_eq!(
        scheduler.current_result_fingerprint_for(NODE, &right),
        Some("right-result")
    );
    assert_eq!(scheduler.current_result_fingerprint(NODE), None);
}

#[test]
fn accept_rejects_every_mutated_identity_and_version_field() {
    let mut scheduler = EvaluationScheduler::new(1024);
    scheduler.advance_revision(1, [NODE]).unwrap();
    let token = scheduler
        .schedule_with_identity(NODE, identity("left"))
        .unwrap();

    let mut forged = token.clone();
    forged.acceptance.document_scope += 1;
    assert_eq!(
        scheduler.accept(result(forged, "wrong-scope")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.derived_identity.root_rule_node_id = OTHER_NODE;
    assert_eq!(
        scheduler.accept(result(forged, "wrong-root")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.derived_identity.slot_path = slot_path(NODE, "right");
    assert_eq!(
        scheduler.accept(result(forged, "wrong-path")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.input_digest.push_str("-changed");
    assert_eq!(
        scheduler.accept(result(forged, "wrong-input")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.evaluator.push_str("-changed");
    assert_eq!(
        scheduler.accept(result(forged, "wrong-evaluator")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.backend = None;
    assert_eq!(
        scheduler.accept(result(forged, "wrong-backend")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.schema.push_str("-changed");
    assert_eq!(
        scheduler.accept(result(forged, "wrong-schema")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.acceptance.tolerance.push_str("-changed");
    assert_eq!(
        scheduler.accept(result(forged, "wrong-tolerance")),
        InsertOutcome::Stale
    );

    let mut forged = token.clone();
    forged.revision_id += 1;
    assert_eq!(
        scheduler.accept(result(forged, "wrong-revision")),
        InsertOutcome::Stale
    );

    let mut forged = token;
    forged.generation += 1;
    assert_eq!(
        scheduler.accept(result(forged, "wrong-generation")),
        InsertOutcome::Stale
    );
}

#[test]
fn malformed_scope_root_fields_and_typed_paths_are_rejected() {
    let mut scheduler = EvaluationScheduler::new(1024);

    let mut malformed = identity("left");
    malformed.document_scope = 0;
    assert_eq!(
        scheduler.schedule_with_identity(NODE, malformed),
        Err(SchedulerError::InvalidAcceptanceIdentity)
    );

    let mut malformed = identity("left");
    malformed.derived_identity.root_rule_node_id = OTHER_NODE;
    assert_eq!(
        scheduler.schedule_with_identity(NODE, malformed),
        Err(SchedulerError::InvalidAcceptanceIdentity)
    );

    let mut malformed = identity("left");
    malformed.derived_identity.root_rule_node_id = NodeId(0);
    assert_eq!(
        scheduler.schedule_with_identity(NODE, malformed),
        Err(SchedulerError::InvalidAcceptanceIdentity)
    );

    let mut malformed = identity("left");
    malformed.derived_identity.slot_path = slot_path(OTHER_NODE, "left");
    assert_eq!(
        scheduler.schedule_with_identity(NODE, malformed),
        Err(SchedulerError::InvalidAcceptanceIdentity)
    );

    let mut segment = SlotSegment::new(NODE, "items", "left").unwrap();
    segment.output_port.clear();
    let mut malformed = identity("left");
    malformed.derived_identity.slot_path = SlotPath::new(vec![segment]).unwrap();
    assert_eq!(
        scheduler.schedule_with_identity(NODE, malformed),
        Err(SchedulerError::InvalidAcceptanceIdentity)
    );

    for malformed in [
        AcceptanceIdentity {
            input_digest: String::new(),
            ..identity("input")
        },
        AcceptanceIdentity {
            evaluator: String::new(),
            ..identity("evaluator")
        },
        AcceptanceIdentity {
            backend: Some(String::new()),
            ..identity("backend")
        },
        AcceptanceIdentity {
            schema: String::new(),
            ..identity("schema")
        },
        AcceptanceIdentity {
            tolerance: String::new(),
            ..identity("tolerance")
        },
    ] {
        assert_eq!(
            scheduler.schedule_with_identity(NODE, malformed),
            Err(SchedulerError::InvalidAcceptanceIdentity)
        );
    }
}

#[test]
fn dirtying_a_node_invalidates_all_scheduled_identities_and_generation() {
    let mut scheduler = EvaluationScheduler::new(1024);
    scheduler.advance_revision(1, [NODE]).unwrap();
    let left = identity("left");
    let right = identity("right");
    let left_token = scheduler
        .schedule_with_identity(NODE, left.clone())
        .unwrap();
    let right_token = scheduler
        .schedule_with_identity(NODE, right.clone())
        .unwrap();
    assert_eq!(
        scheduler.accept(result(left_token.clone(), "left-result")),
        InsertOutcome::Current
    );
    assert_eq!(
        scheduler.accept(result(right_token.clone(), "right-result")),
        InsertOutcome::Current
    );

    scheduler.advance_revision(2, [NODE]).unwrap();

    assert_eq!(scheduler.current_result_fingerprint_for(NODE, &left), None);
    assert_eq!(scheduler.current_result_fingerprint_for(NODE, &right), None);
    for mut stale in [left_token, right_token] {
        stale.revision_id = 2;
        stale.generation += 1;
        assert_eq!(
            scheduler.accept(result(stale, "forged-current-version")),
            InsertOutcome::Stale
        );
    }
}
