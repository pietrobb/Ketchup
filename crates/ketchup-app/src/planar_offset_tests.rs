use super::*;

#[test]
fn planar_offset_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_planar_offset() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_closed_polyline(vec![
            [120.0, 0.0],
            [220.0, 0.0],
            [220.0, 60.0],
            [120.0, 60.0],
        ]));
        app.dispatch_command(AppCommand::PlanarOffset);
        app.value_box.input = "5".to_owned();
        assert!(app.refresh_planar_offset_preview());
        assert!(app.planar_offset_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_planar_offset();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .tool_preview
        .get_mut::<PlanarOffsetPreview>()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.planar_offset_preview_is_current());
    assert!(!batch_tamper.confirm_planar_offset_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_planar_offset();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper
        .tool_preview
        .get_mut::<PlanarOffsetPreview>()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_planar_offset_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_planar_offset();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .tool_preview
        .get_mut::<PlanarOffsetPreview>()
        .unwrap()
        .plan
        .exact_graph
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_planar_offset_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_planar_offset();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .tool_preview
        .get_mut::<PlanarOffsetPreview>()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    assert!(!source_tamper.confirm_planar_offset_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_planar_offset();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_planar_offset_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_planar_offset();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_planar_offset_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_planar_offset();
    let occurrence_id = stale
        .tool_preview
        .get::<PlanarOffsetPreview>()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_planar_offset_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_planar_offset();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_planar_offset_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_planar_offset_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}
