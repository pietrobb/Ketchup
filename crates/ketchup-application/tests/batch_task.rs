use ketchup_application::DocumentSession;
use ketchup_application::batch_task::{
    MAX_OCCURRENCE_BATCH_ITEMS, OccurrenceBatchError, OccurrenceBatchOperation,
    OccurrenceBatchState,
};
use ketchup_application::model_query::{EntityKind, ModelQuery, PageRequest, QueryError};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, OccurrenceId, Transform,
};
use std::cell::Cell;

fn request(kind: EntityKind) -> PageRequest {
    PageRequest {
        kind,
        limit: 100,
        search: String::new(),
        definition_id: None,
        tag_id: None,
        classification_dimension_id: None,
        classification_category_id: None,
        world_bounds_mm: None,
        cursor: None,
    }
}

fn session_with_occurrences(count: u64) -> DocumentSession {
    let mut session = DocumentSession::default();
    let definition = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Batch part".into(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&definition).unwrap();

    let commands = (1..=count)
        .map(|id| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1),
            name: format!("part-{id:05}"),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        })
        .collect::<Vec<_>>();
    for chunk in commands.chunks(MAX_OCCURRENCE_BATCH_ITEMS) {
        let proposal = session
            .plan_commands(CommandBatch::new(chunk.to_vec()))
            .unwrap();
        session.apply_proposal(&proposal).unwrap();
    }
    session
}

fn occurrence_task(
    session: &DocumentSession,
    query: &ModelQuery,
) -> ketchup_application::batch_task::OccurrenceBatchTask {
    let snapshot = session.snapshot();
    let workset = query
        .create_workset(&snapshot, &request(EntityKind::Occurrences))
        .unwrap();
    query
        .create_occurrence_batch_task(
            session,
            workset["workset_handle"].as_str().unwrap(),
            OccurrenceBatchOperation::SetColor {
                color: Some([10, 20, 30]),
            },
        )
        .unwrap()
}

#[test]
fn occurrence_workset_runs_as_bounded_atomic_transactions_with_compact_receipts() {
    let mut session = session_with_occurrences(513);
    let query = ModelQuery::default();
    let mut task = occurrence_task(&session, &query);
    let undo_before = session.visible_undo_steps();
    let pending = task.status(&session);
    assert_eq!(pending.state, OccurrenceBatchState::Pending);
    assert_eq!(pending.total_count, 513);
    assert_eq!(pending.completed_count, 0);

    let first = task.commit_next(&mut session).unwrap().unwrap();
    assert_eq!(first.batch_index, 0);
    assert_eq!(first.applied_count, MAX_OCCURRENCE_BATCH_ITEMS);
    assert_eq!(first.completed_count, MAX_OCCURRENCE_BATCH_ITEMS);
    assert_eq!(first.remaining_count, 1);
    assert!(!first.complete);
    assert_eq!(first.verified_write_count, MAX_OCCURRENCE_BATCH_ITEMS);
    assert_eq!(
        first.operation_payload,
        OccurrenceBatchOperation::SetColor {
            color: Some([10, 20, 30])
        }
    );
    assert_eq!(session.visible_undo_steps(), undo_before + 1);
    assert_eq!(task.status(&session).state, OccurrenceBatchState::Running);
    assert_eq!(
        session
            .snapshot()
            .occurrence(OccurrenceId(512))
            .unwrap()
            .color(),
        Some([10, 20, 30])
    );
    assert_eq!(
        session
            .snapshot()
            .occurrence(OccurrenceId(513))
            .unwrap()
            .color(),
        None
    );
    let encoded = serde_json::to_vec(&first).unwrap();
    assert!(encoded.len() < 32 * 1024);
    assert!(
        !String::from_utf8(encoded)
            .unwrap()
            .contains("occurrence_ids")
    );

    let second = task.commit_next(&mut session).unwrap().unwrap();
    assert_eq!(second.batch_index, 1);
    assert_eq!(second.applied_count, 1);
    assert_eq!(second.completed_count, 513);
    assert_eq!(second.remaining_count, 0);
    assert!(second.complete);
    assert!(task.is_complete());
    assert_eq!(task.status(&session).state, OccurrenceBatchState::Completed);
    task.cancel();
    assert!(task.commit_next(&mut session).unwrap().is_none());
    assert_eq!(session.visible_undo_steps(), undo_before + 2);

    session.undo().unwrap();
    assert_eq!(
        session
            .snapshot()
            .occurrence(OccurrenceId(512))
            .unwrap()
            .color(),
        Some([10, 20, 30])
    );
    assert_eq!(
        session
            .snapshot()
            .occurrence(OccurrenceId(513))
            .unwrap()
            .color(),
        None
    );
}

#[test]
fn cancellation_after_planning_publishes_nothing_and_stale_tasks_fail_closed() {
    let mut session = session_with_occurrences(2);
    let query = ModelQuery::default();
    let mut cancelled = occurrence_task(&session, &query);
    let before = session.snapshot();
    let undo_before = session.visible_undo_steps();
    let checks = Cell::new(0);
    let result = cancelled.commit_next_with_cancel(&mut session, || {
        let check = checks.get();
        checks.set(check + 1);
        check == 1
    });
    assert!(matches!(result, Err(OccurrenceBatchError::Cancelled)));
    assert!(cancelled.is_cancelled());
    assert_eq!(cancelled.completed_count(), 0);
    assert_eq!(
        session.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(session.visible_undo_steps(), undo_before);

    let mut stale = occurrence_task(&session, &query);
    let rename = session
        .plan_commands(CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "changed outside batch".into(),
        }]))
        .unwrap();
    session.apply_proposal(&rename).unwrap();
    assert!(matches!(
        stale.commit_next(&mut session),
        Err(OccurrenceBatchError::StaleTask { .. })
    ));
    assert_eq!(stale.completed_count(), 0);
}

#[test]
fn undo_redo_aba_cannot_revive_a_partially_completed_task() {
    let mut session = session_with_occurrences(513);
    let query = ModelQuery::default();
    let mut task = occurrence_task(&session, &query);
    task.commit_next(&mut session).unwrap().unwrap();
    let expected = session.snapshot();
    let expected_epoch = session.mutation_epoch();

    session.undo().unwrap();
    session.redo().unwrap();
    assert_eq!(session.snapshot().revision_id(), expected.revision_id());
    assert_eq!(
        session.snapshot().canonical_digest(),
        expected.canonical_digest()
    );
    assert_ne!(session.mutation_epoch(), expected_epoch);
    assert!(matches!(
        task.commit_next(&mut session),
        Err(OccurrenceBatchError::StaleTask { .. })
    ));
    assert_eq!(task.completed_count(), MAX_OCCURRENCE_BATCH_ITEMS);
}

#[test]
fn batch_operation_requires_explicit_color_and_rejects_unknown_fields() {
    assert_eq!(
        serde_json::from_value::<OccurrenceBatchOperation>(
            serde_json::json!({"type":"set_color","color":null}),
        )
        .unwrap(),
        OccurrenceBatchOperation::SetColor { color: None }
    );
    for invalid in [
        serde_json::json!({"type":"set_color"}),
        serde_json::json!({"type":"set_color","colour":null}),
        serde_json::json!({"type":"set_color","color":null,"extra":true}),
    ] {
        assert!(serde_json::from_value::<OccurrenceBatchOperation>(invalid).is_err());
    }
}

#[test]
fn non_occurrence_worksets_cannot_be_coerced_into_occurrence_batches() {
    let session = session_with_occurrences(2);
    let snapshot = session.snapshot();
    let query = ModelQuery::default();
    let workset = query
        .create_workset(&snapshot, &request(EntityKind::Relations))
        .unwrap();
    let result = query.create_occurrence_batch_task(
        &session,
        workset["workset_handle"].as_str().unwrap(),
        OccurrenceBatchOperation::SetColor { color: None },
    );
    assert!(matches!(result, Err(QueryError::UnsupportedWorksetScope)));
}
