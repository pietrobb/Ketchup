//! Opening and saving documents, commands, dialogs, tools and keys.

use super::*;

/// Steps the shell until `done` holds, polling between frames, and fails once
/// `within` has passed, naming `what` and the status line. Unit tests wait on
/// background work (exact evaluation, a commit) here instead of sleeping themselves.
pub(crate) fn step_until(
    harness: &mut Harness<'_, KetchupApp>,
    within: Duration,
    what: &str,
    mut done: impl FnMut(&KetchupApp) -> bool,
) {
    let deadline = Instant::now() + within;
    while !done(harness.state()) {
        assert!(
            Instant::now() < deadline,
            "{what}: {}",
            harness.state().digest
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How long a test may hold work before a frame still waiting for it counts as
/// hung: far longer than any frame, far shorter than a CI job's timeout.
const HANG_PATIENCE: Duration = Duration::from_secs(120);

/// While alive, lets go of the held work through `release` once
/// [`HANG_PATIENCE`] has passed, and then fails its test when dropped, naming
/// `what`. A test that holds work with no deadline of its own (a locked file, a
/// held program evaluation) arms one, so a frame that waits for that work ends
/// and fails that test alone instead of blocking the run until the CI job is
/// cancelled; the other tests in the binary still report (review 2026-10-24:
/// ending the process lost their results).
pub(crate) struct HangGuard {
    what: &'static str,
    disarm: Option<std::sync::mpsc::Sender<()>>,
    watcher: Option<std::thread::JoinHandle<bool>>,
}

pub(crate) fn hang_guard(what: &'static str, release: impl FnOnce() + Send + 'static) -> HangGuard {
    hang_guard_within(what, HANG_PATIENCE, release)
}

fn hang_guard_within(
    what: &'static str,
    patience: Duration,
    release: impl FnOnce() + Send + 'static,
) -> HangGuard {
    let (disarm, armed) = std::sync::mpsc::channel::<()>();
    let watcher = std::thread::spawn(move || {
        let hung = armed.recv_timeout(patience) == Err(std::sync::mpsc::RecvTimeoutError::Timeout);
        if hung {
            use std::io::Write as _;
            // Straight to stderr: whatever the frame does next, the cause is named.
            let _ = writeln!(
                std::io::stderr(),
                "hung: {what} was still held after {patience:?}; a frame waited for it, so it is let go"
            );
            release();
        }
        hung
    });
    HangGuard {
        what,
        disarm: Some(disarm),
        watcher: Some(watcher),
    }
}

impl Drop for HangGuard {
    fn drop(&mut self) {
        drop(self.disarm.take());
        let hung = self
            .watcher
            .take()
            .is_some_and(|watcher| watcher.join().unwrap_or(true));
        if hung && !std::thread::panicking() {
            panic!(
                "hung: {} was still held past the guard's patience; a frame waited for it",
                self.what
            );
        }
    }
}

#[test]
fn a_hung_guard_lets_the_work_go_and_fails_only_its_own_test() {
    let (unblock, blocked) = std::sync::mpsc::channel::<()>();
    let guard = hang_guard_within("a test's own hold", Duration::from_millis(50), move || {
        drop(unblock)
    });
    // The frame that waits for the held work ends once the guard lets it go.
    assert!(blocked.recv().is_err());
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(guard)))
        .expect_err("a hung guard fails its test");
    let message = failure.downcast_ref::<String>().expect("message");
    assert!(message.contains("hung: a test's own hold"), "{message}");
    // A guard dropped in time neither lets go nor fails anything.
    drop(hang_guard_within(
        "quick work",
        Duration::from_secs(60),
        || panic!("let go in time"),
    ));
}

#[test]
fn review_only_open_preserves_the_active_document_and_its_history() {
    let directory = tempfile::tempdir().unwrap();
    let active_path = directory.path().join("active.ketchup");
    let review_path = directory.path().join("legacy.ketchup");
    std::fs::write(&review_path, lossy_legacy_document()).unwrap();

    let mut app = KetchupApp::new();
    assert!(app.save_document_to(&active_path));
    let node_id = ketchup_model::document::NodeId(900);
    let active_revision = app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: node_id,
                name: "active parameter".to_owned(),
                dimension: Dimension::new("2", 2.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: node_id,
                dimension: Dimension::new("3", 3.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(app.undo());
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.document.visible_redo_steps(), 1);

    let before = app.document.current();
    assert_eq!(
        before.canonical_digest(),
        active_revision.snapshot().canonical_digest()
    );
    let before_document_id = before.document_id();
    let before_revision = before.revision_id();
    let before_digest = before.canonical_digest();
    let before_canonical_bytes = ketchup_model::persistence::save(&before);
    let before_evaluation = before.evaluate(&Default::default()).unwrap();
    let before_path = app.file.path.clone();
    let before_dirty = app.is_dirty();
    let before_undo_steps = app.document.visible_undo_steps();
    let before_redo_steps = app.document.visible_redo_steps();
    let before_revision_count = app.document.revision_count();
    let before_evaluation_registry = app.document.evaluation_registry_len();

    assert!(!app.open_document_from(&review_path));
    assert!(app.has_review_candidate());
    assert!(!app.file.review_candidate.as_ref().unwrap().is_editable());

    let after = app.document.current();
    assert_eq!(after.document_id(), before_document_id);
    assert_eq!(after.revision_id(), before_revision);
    assert_eq!(after.canonical_digest(), before_digest);
    assert_eq!(
        ketchup_model::persistence::save(&after),
        before_canonical_bytes
    );
    assert_eq!(
        after.evaluate(&Default::default()).unwrap(),
        before_evaluation
    );
    assert_eq!(app.file.path, before_path);
    assert_eq!(app.is_dirty(), before_dirty);
    assert_eq!(app.document.visible_undo_steps(), before_undo_steps);
    assert_eq!(app.document.visible_redo_steps(), before_redo_steps);
    assert_eq!(app.document.revision_count(), before_revision_count);
    assert_eq!(
        app.document.evaluation_registry_len(),
        before_evaluation_registry
    );
}

#[test]
fn migration_confirmation_rejects_review_candidate_tamper_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy-review.ketchup");
    let destination = directory.path().join("tampered-review-migration.ketchup");
    let source_bytes = lossy_legacy_document();
    std::fs::write(&source, &source_bytes).unwrap();

    let mut app = KetchupApp::new();
    let before = app.document.current();
    assert!(!app.open_document_from(&source));
    assert!(app.has_review_candidate());

    let mut alternate_source = source_bytes;
    let value_start = alternate_source.len() - 12;
    alternate_source[value_start..value_start + 8]
        .copy_from_slice(&4.5_f64.to_bits().to_le_bytes());
    app.file.review_candidate = Some(ketchup_model::persistence::load(&alternate_source).unwrap());

    assert!(!app.confirm_review_candidate_migration_to(&destination));
    assert!(
        app.action_digest()
            .contains(&migration_review::MigrationError::ReviewMismatch.to_string()),
        "{}",
        app.action_digest()
    );
    assert!(!destination.exists());
    assert!(app.has_review_candidate());
    let after = app.document.current();
    assert_eq!(after.document_id(), before.document_id());
    assert_eq!(after.revision_id(), before.revision_id());
    assert_eq!(after.canonical_digest(), before.canonical_digest());
}

#[test]
fn lossless_open_replaces_the_document_preserves_history_and_clears_review() {
    let directory = tempfile::tempdir().unwrap();
    let review_path = directory.path().join("legacy.ketchup");
    let lossless_path = directory.path().join("lossless.ketchup");
    std::fs::write(&review_path, lossy_legacy_document()).unwrap();

    let mut source = KetchupApp::new();
    assert!(source.create_box());
    assert!(source.create_box());
    let expected = source.document.current();
    let expected_bytes = ketchup_model::persistence::save(&expected);
    assert!(source.save_document_to(&lossless_path));

    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let replaced_document_id = app.document.current().document_id();
    assert!(app.document.visible_undo_steps() > 0);
    assert!(!app.open_document_from(&review_path));
    assert!(app.has_review_candidate());

    assert!(app.open_document_from(&lossless_path));

    let opened = app.document.current();
    assert_ne!(opened.document_id(), replaced_document_id);
    assert_eq!(opened.document_id(), expected.document_id());
    assert_eq!(opened.revision_id(), expected.revision_id());
    assert_eq!(opened.canonical_digest(), expected.canonical_digest());
    assert_eq!(ketchup_model::persistence::save(&opened), expected_bytes);
    assert_eq!(app.file.path.as_deref(), Some(lossless_path.as_path()));
    assert!(!app.is_dirty());
    assert_eq!(app.document.revision_count(), 1);
    assert!(!app.has_review_candidate());
}

#[test]
fn a_file_whose_history_does_not_decode_opens_without_undo_and_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("damaged-history.ketchup");
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    let expected = app.document.current();
    std::fs::write(
        &path,
        ketchup_model::testing::save_with_history_entry(&app.document, b"KETCHUPHST\x09".to_vec()),
    )
    .unwrap();
    // Review 2026-10-15 status, low: a damaged checkpoint beside the file is
    // named too, not only the first warning of the open.
    let checkpoint = ketchup_model::persistence::work_recovery_path(&path);
    std::fs::write(&checkpoint, b"KETCHUPWRK damaged").unwrap();

    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_from(&path), "{}", reopened.digest);
    // The words between the path and the next value tell the warnings apart.
    let words = |key: &str| {
        let text = reopened.catalog.text(key);
        text.split('}')
            .nth(1)
            .unwrap()
            .split('{')
            .next()
            .unwrap()
            .to_owned()
    };
    for key in [
        "digest-opened-work-recovery-set-aside",
        "digest-opened-without-history",
    ] {
        assert!(
            reopened.digest.contains(&words(key)),
            "{key}: {}",
            reopened.digest
        );
    }
    assert!(!checkpoint.exists());
    assert_eq!(
        reopened.document.current().canonical_digest(),
        expected.canonical_digest()
    );
    assert_eq!(reopened.document.visible_undo_steps(), 0);
    assert!(!reopened.is_dirty());
    let warning = reopened.catalog.text("digest-opened-without-history");
    let prefix = warning.split('{').next().unwrap();
    assert!(reopened.digest.contains(prefix), "{}", reopened.digest);
    assert!(reopened.digest.contains("damaged-history.ketchup"));

    reopened.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(reopened.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert!(reopened.save_document_to(&path));
    let ketchup_model::persistence::LoadOutcome::Editable {
        document, audit, ..
    } = ketchup_model::persistence::load_file(&path).unwrap()
    else {
        panic!("editable document")
    };
    assert_eq!(audit.history_discarded(), None);
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn file_workflow_round_trips_composed_model_and_tracks_dirty_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("composed.ketchup");
    let mut app = KetchupApp::new();
    assert!(!app.is_dirty());

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let expected = app.document.current();
    assert!(app.is_dirty());
    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));

    let mut reopened = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    assert!(reopened.open_document_from(&path));
    let actual = reopened.document.current();
    assert_eq!(actual.canonical_digest(), expected.canonical_digest());
    assert_eq!(actual.revision_id(), expected.revision_id());
    assert_eq!(actual.document_id(), expected.document_id());
    assert_eq!(actual.units(), expected.units());
    assert_eq!(actual.definitions().count(), 1);
    assert_eq!(actual.occurrences().count(), 2);
    assert_eq!(actual.groups().count(), 1);
    assert_eq!(actual.scene_query()[0].shared_occurrence_count, 2);
    assert_eq!(
        actual.occurrence(OccurrenceId(1)).unwrap().parent(),
        actual.occurrence(OccurrenceId(2)).unwrap().parent()
    );
    assert_eq!(
        actual.occurrence(OccurrenceId(2)).unwrap().transform(),
        expected.occurrence(OccurrenceId(2)).unwrap().transform()
    );
    assert_eq!(
        actual.feature(FeatureId(2)).unwrap().kind(),
        expected.feature(FeatureId(2)).unwrap().kind()
    );
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.document.visible_undo_steps(), 0);

    reopened.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(reopened.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert!(reopened.is_dirty());
    assert!(reopened.undo());
    assert!(!reopened.is_dirty());
    assert!(reopened.redo());
    assert!(reopened.is_dirty());
    assert!(reopened.save_document_to(&path));
    let saved_again = ketchup_model::persistence::load_file(&path)
        .unwrap()
        .snapshot();
    assert_eq!(
        saved_again.canonical_digest(),
        reopened.document.current().canonical_digest()
    );
}

#[test]
fn failed_open_and_save_preserve_the_active_document_and_file_identity() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.ketchup");
    std::fs::write(&malformed, b"not a ketchup document").unwrap();
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let before_digest = app.document.current().canonical_digest();
    let before_path = app.file.path.clone();
    let before_saved_digest = app.file.saved_digest.clone();

    assert!(!app.open_document_from(&malformed));
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert_eq!(app.file.path, before_path);
    assert_eq!(app.file.saved_digest, before_saved_digest);
    assert!(app.digest.contains("active model was not changed"));

    assert!(!app.save_document_to(directory.path()));
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert_eq!(app.file.path, before_path);
    assert_eq!(app.file.saved_digest, before_saved_digest);
    assert!(app.is_dirty());
    assert!(app.digest.contains("active model remains unsaved"));
}

#[test]
fn command_registry_exposes_only_complete_modeling_tools() {
    let mut app = KetchupApp::new();
    assert!(app.command_enabled(AppCommand::Select));
    assert!(app.command_enabled(AppCommand::Orbit));
    assert!(app.command_enabled(AppCommand::Rectangle));
    assert!(app.command_enabled(AppCommand::PushPull));
    assert!(app.command_enabled(AppCommand::Move));

    app.dispatch_command(AppCommand::Rectangle);
    assert_eq!(app.active_tool, ActiveTool::Rectangle);
    assert!(app.gesture.sketch.armed);
    app.dispatch_command(AppCommand::PushPull);
    assert_eq!(app.active_tool, ActiveTool::PushPull);
    assert!(!app.gesture.sketch.armed);
}

#[test]
fn opening_program_document_evaluates_and_frames_actual_scene() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/programs/table.ketchup");
    assert!(fixture.is_file(), "missing fixture: {}", fixture.display());
    // Opening takes the work-recovery lock beside the file and may set a checkpoint
    // aside there, so open a copy and leave the source tree untouched.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("table.ketchup");
    std::fs::copy(&fixture, &path).unwrap();
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .expect("build ketchup-exact-worker before this test");

    let mut app = KetchupApp::new();
    assert!(
        app.open_document_from(&path),
        "table fixture did not open: {}",
        app.digest
    );
    assert_eq!(app.occurrence_count(), 5);
    assert!(app.camera.zoom_fit_pending);
    let zoom_before = app.camera.zoom;
    app.headless_force_exact_worker_path(worker);

    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    step_until(
        &mut harness,
        Duration::from_secs(30),
        "the opened table never painted",
        |app| {
            app.exact.results.len() == 5
                && app.instanced_scene_triangle_count() > 0
                && !app.camera.zoom_fit_pending
        },
    );

    let app = harness.state();
    assert_eq!(app.exact.results.len(), 5, "all table bodies must evaluate");
    assert!(
        app.instanced_scene_triangle_count() > 0,
        "the evaluated table must reach the painted scene"
    );
    assert!(!app.camera.zoom_fit_pending);
    assert_ne!(app.camera.zoom, zoom_before, "the table must be framed");
}

/// A live client that opens a window and immediately asks for Zoom Fit must
/// not get a silently unframed camera just because no frame was laid out yet.
#[test]
fn zoom_fit_before_first_layout_is_applied_on_the_first_frame() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    assert!(app.camera.viewport_rect.is_none());
    let zoom_before = app.camera.zoom;
    app.dispatch_command(AppCommand::ZoomFit);
    assert!(app.camera.zoom_fit_pending);
    assert_eq!(app.camera.zoom, zoom_before);

    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let app = harness.state();
    assert!(!app.camera.zoom_fit_pending);
    assert_ne!(
        app.camera.zoom, zoom_before,
        "the pending fit must frame the model"
    );

    let mut app = KetchupApp::new();
    app.dispatch_command(AppCommand::ZoomFit);
    app.dispatch_command(AppCommand::ViewTop);
    assert!(
        !app.camera.zoom_fit_pending,
        "an explicit later view choice supersedes a pending fit"
    );
}

/// The fit after a new program frames its parts once their exact bounds are
/// known, but a message the user got in between stays on the status line.
#[test]
fn the_fit_after_a_new_program_frames_quietly() {
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .expect("build ketchup-exact-worker before this test");
    let mut app = KetchupApp::new();
    app.headless_force_exact_worker_path(worker);
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "board.star".into(),
            source: "box(\"doska\", (600, 18, 400))\n".into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    assert!(app.camera.viewport_rect.is_none());
    assert!(app.camera.zoom_fit_pending);
    let zoom_before = app.camera.zoom;
    app.digest = "the answer to the user's last action".to_owned();

    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    step_until(
        &mut harness,
        Duration::from_secs(30),
        "the new program's parts were never framed",
        |app| !app.camera.zoom_fit_pending,
    );
    let app = harness.state();
    assert_ne!(app.camera.zoom, zoom_before, "the parts must be framed");
    assert_eq!(app.digest, "the answer to the user's last action");
}

#[test]
fn every_view_switch_is_one_command_with_a_label_and_both_reports() {
    let catalog = ketchup_interaction::LocaleCatalog::english();
    let mut app = KetchupApp::new();
    // Hidden objects is offered only while something is hidden.
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    assert!(app.set_selection_visibility(false));
    for flag in ViewFlag::ALL {
        assert!(app.command_enabled(AppCommand::View(flag)), "{flag:?}");
        let spec = CommandRegistry::spec(AppCommand::View(flag));
        assert!(
            catalog.contains(spec.label_key),
            "{flag:?}: {}",
            spec.label_key
        );
        let name = spec.label_key.trim_start_matches("view-");
        for state in ["shown", "hidden"] {
            let key = format!("digest-{name}-{state}");
            assert!(catalog.contains(&key), "{flag:?}: {key}");
        }
        let before = app.view_visible(flag);
        app.dispatch_command(AppCommand::View(flag));
        assert_eq!(app.view_visible(flag), !before, "{flag:?}");
        assert_eq!(
            app.digest,
            catalog.text(&format!(
                "digest-{name}-{}",
                if before { "hidden" } else { "shown" }
            ))
        );
        // Previous View restores every switch together with the camera.
        app.previous_view();
        assert_eq!(
            app.view_visible(flag),
            before,
            "{flag:?} after Previous View"
        );
    }
}

#[test]
fn opening_a_dialog_replaces_the_one_that_was_open() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.begin_definition_rename();
    assert!(app.modal.get::<PendingDefinitionRename>().is_some());
    app.begin_occurrence_rename();
    assert!(app.modal.get::<PendingOccurrenceRename>().is_some());
    assert!(app.modal.get::<PendingDefinitionRename>().is_none());
    app.begin_tag_creation(None);
    assert!(app.tag_creation_visible());
    assert!(app.modal.get::<PendingOccurrenceRename>().is_none());

    // Closing or removing a dialog that is not open leaves the open one.
    app.modal.close::<PendingOccurrenceRename>();
    assert!(app.modal.remove::<PendingDefinitionRename>().is_none());
    assert!(app.tag_creation_visible());
    assert!(app.modal.remove::<PendingTagCreation>().is_some());
    assert!(app.modal.is_none());
}

#[test]
fn a_new_tool_preview_replaces_the_shown_one_and_cancel_clears_it() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview(), "{}", app.digest);
    assert!(app.has_preview());
    assert_eq!(
        app.push_pull_preview_definition(),
        Some(INITIAL_BOX_DEFINITION)
    );

    assert!(app.preview_linear_pattern(OccurrenceId(1), Axis::Z, 50.0, 4));
    assert!(app.has_occurrence_operation_preview());
    assert!(app.tool_preview.get::<EphemeralBoxPreview>().is_none());
    assert!(!app.has_preview());
    assert_eq!(app.push_pull_preview_definition(), None);

    app.cancel_preview();
    assert!(app.tool_preview.is_none());
    assert!(!app.has_occurrence_operation_preview());
}

#[test]
fn starting_a_pointer_drag_ends_the_one_that_was_running() {
    let mut app = KetchupApp::new();
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    app.gesture.drag.open(ZoomWindowDrag {
        start: viewport.left_top(),
        cursor: viewport.center(),
    });
    app.gesture.drag.open(SelectionWindowDrag {
        start: viewport.center(),
        cursor: viewport.right_bottom(),
        additive: false,
    });
    assert!(app.gesture.drag.get::<ZoomWindowDrag>().is_none());
    assert!(app.gesture.drag.get::<SelectionWindowDrag>().is_some());

    app.gesture.drag.close::<ZoomWindowDrag>();
    assert!(
        app.gesture.drag.get::<SelectionWindowDrag>().is_some(),
        "closing a drag that is not running leaves the running one alone"
    );

    app.gesture.drag.open(ZoomWindowDrag {
        start: viewport.left_top(),
        cursor: viewport.left_top(),
    });
    assert!(app.gesture.drag.get::<SelectionWindowDrag>().is_none());
    app.gesture.drag.close::<ZoomWindowDrag>();
    assert!(app.gesture.drag.is_none());
}

#[test]
fn every_shortcut_in_the_keymap_runs_its_command_and_prints_the_same_chord() {
    let english = LocaleCatalog::english();
    let slovak = LocaleCatalog::slovak();
    for binding in keymap::KEYMAP {
        for chord in binding.chords {
            assert_eq!(
                press_in_a_frame(vec![key_event(chord)], false),
                Some(binding.command),
                "{chord:?} must run {:?}, not an earlier binding",
                binding.command
            );
            assert_eq!(
                press_in_a_frame(vec![key_event(chord)], true),
                binding.while_typing.then_some(binding.command),
                "{chord:?} while typing"
            );
            for catalog in [&english, &slovak] {
                let text = keymap::chord_text(catalog, chord);
                assert!(!text.is_empty() && !text.contains('['), "{text}");
            }
        }
        assert_eq!(
            keymap::shortcut_text(&english, binding.command),
            keymap::chord_text(&english, &binding.chords[0])
        );
        assert!(
            CommandRegistry::COMMANDS
                .iter()
                .any(|spec| spec.id == binding.command && spec.implemented),
            "{:?} is bound but not an implemented command",
            binding.command
        );
    }
    assert_eq!(
        keymap::shortcut_text(&english, AppCommand::SaveAs),
        if cfg!(target_os = "macos") {
            "Shift+Cmd+S"
        } else {
            "Ctrl+Shift+S"
        }
    );
    assert_eq!(
        keymap::shortcut_text(&slovak, AppCommand::Select),
        "Medzerník"
    );
    assert_eq!(keymap::shortcut_text(&english, AppCommand::Orbit), "O");
    assert!(keymap::shortcut_text(&english, AppCommand::ImportMeshStl).is_empty());
    // Review 2026-10-09 (P3): Ctrl+Shift+Z undid instead of redoing.
    assert_eq!(
        press_in_a_frame(
            vec![key_event(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::Z
            ))],
            false
        ),
        Some(AppCommand::Redo)
    );
}

#[test]
fn platform_clipboard_requests_run_the_clipboard_commands() {
    // The desktop shell turns Ctrl+C, Ctrl+X and Ctrl+V into clipboard
    // events instead of key presses.
    assert_eq!(
        press_in_a_frame(vec![egui::Event::Copy], false),
        Some(AppCommand::Copy)
    );
    assert_eq!(
        press_in_a_frame(vec![egui::Event::Cut], false),
        Some(AppCommand::Cut)
    );
    assert_eq!(
        press_in_a_frame(vec![egui::Event::Paste("text".to_owned())], false),
        Some(AppCommand::Paste)
    );
    assert_eq!(press_in_a_frame(vec![egui::Event::Cut], true), None);
}

#[test]
fn the_shell_state_stays_grouped_into_a_few_owned_parts() {
    // Loose fields are how two dialogs, two previews or two drags ended up
    // alive at once; new state joins the part it serves instead.
    let source = include_str!("../lib.rs");
    let start = source
        .find("pub struct KetchupApp {")
        .expect("KetchupApp is declared in lib.rs");
    let end = start + source[start..].find("\n}\n").expect("KetchupApp ends");
    let fields = source[start..end]
        .lines()
        .filter(|line| {
            line.strip_prefix("    ").is_some_and(|field| {
                field.starts_with(|c: char| c.is_ascii_lowercase()) && field.contains(": ")
            })
        })
        .count();
    assert!(fields <= 60, "KetchupApp has {fields} fields");
}
