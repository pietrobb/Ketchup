//! File > Import STEP and IGES in the window (review 2026-10-19 UI-14): the
//! exact worker reads the file on a thread while the window shows it, Cancel
//! stops the worker, and confirming the review does not read the file again.
use crate::tests::shell::step_until;
use crate::*;
use egui_kittest::kittest::Queryable as _;
use std::sync::atomic::Ordering;

fn shell(app: KetchupApp) -> egui_kittest::Harness<'static, KetchupApp> {
    egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app)
}

/// A read that never ends by itself keeps the window painting and says what
/// it reads; Escape stops it, and the reader sees that it was cancelled.
#[test]
fn a_step_file_being_read_leaves_the_window_working_and_cancel_stops_the_worker() {
    let mut harness = shell(KetchupApp::new());
    harness.step();
    let digest = harness.state().document.history_digest();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let (stopped, saw_cancel) = std::sync::mpsc::channel();
    let context = harness.ctx.clone();
    harness.state_mut().queue_exact_import_read(
        ImportFormat::Step,
        PathBuf::from("large.step"),
        &context,
        move |_, cancelled| {
            // Held until the test has pressed Escape, as a worker still reading.
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            stopped.send(cancelled.load(Ordering::Acquire)).unwrap();
            Err(ImportError::failed(ImportFormat::Step, "cancelled"))
        },
    );
    let label = harness.state().file_activity_label().expect("busy");
    for _ in 0..4 {
        harness.step();
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some(),
            "{label}"
        );
    }
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    assert!(harness.state().file.activity.exact_import_reading.is_none());
    release.send(()).unwrap();
    assert_eq!(saw_cancel.recv_timeout(Duration::from_secs(10)), Ok(true));
    harness.step();
    assert!(harness.state().modal.get::<PendingStepImport>().is_none());
    assert_eq!(harness.state().document.history_digest(), digest);
}

/// The menu's STEP import reads the file on a thread and opens the review;
/// confirming it publishes from what the worker read, so it succeeds even
/// with no worker left to read the file again.
#[test]
fn a_step_review_is_read_on_a_thread_and_confirmed_without_reading_it_again() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Step, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.exact.worker_path = Some(crate::tests::exact_worker_executable());
    app.exact.worker_attempted = true;
    let mut harness = shell(app);
    harness.step();
    let occurrences = harness.state().document.current().occurrences().count();

    let context = harness.ctx.clone();
    let app = harness.state_mut();
    app.file.activity.context = Some(context);
    app.dispatch_file_command(AppCommand::ImportExactStep);
    assert!(app.file.activity.exact_import_reading.is_some());
    assert!(app.modal.get::<PendingStepImport>().is_none());
    step_until(
        &mut harness,
        Duration::from_secs(60),
        "the STEP file was not read",
        |app| app.file.activity.exact_import_reading.is_none(),
    );
    let app = harness.state_mut();
    let pending = app
        .modal
        .get::<PendingStepImport>()
        .cloned()
        .unwrap_or_else(|| panic!("no review: {}", app.action_digest()));

    app.exact.worker_path = Some(std::env::temp_dir().join("no-such-exact-worker.exe"));
    app.import_step_from(&pending)
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(app.document.current().occurrences().count() > occurrences);
}
