use super::*;
use crate::dialogs::ScriptedFileDialogs;
use crate::tests::shell::step_until;
use egui_kittest::{Harness, kittest::Queryable as _};
use std::time::{Duration, Instant};

fn shell() -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        ScriptedFileDialogs::new().always_confirm_high_risk_as(84),
    ));
    crate::tests::install_initial_graph_result(&mut app);
    // Only the job's own check needs the file; these tests replace the worker.
    app.exact.worker_path = Some(std::env::current_exe().unwrap());
    Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app)
}

fn queue(
    harness: &mut Harness<'static, KetchupApp>,
    path: &Path,
    run: impl FnOnce(&ExchangeExportJob, &AtomicBool) -> Result<Vec<u8>, ExportError> + Send + 'static,
) {
    let job = harness
        .state_mut()
        .exchange_export_job(ExchangeFormat::Step, path)
        .unwrap();
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .queue_exchange_export(job, &context, run);
}

#[test]
fn a_running_export_keeps_the_window_painting_and_cancel_stops_its_worker_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancelled.step");
    let mut harness = shell();
    let (stopped, observed) = mpsc::channel();
    queue(&mut harness, &path, move |_, cancelled| {
        // A slow worker: it runs until the user cancels it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::yield_now();
        }
        stopped.send(cancelled.load(Ordering::Acquire)).unwrap();
        Err(ExportError::failed("cancelled"))
    });
    // Had a frame waited for the worker, it would run to its deadline and
    // report that it never saw the cancellation.
    for _ in 0..8 {
        harness.step();
        let label = harness.state().file_activity_label().unwrap();
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some()
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
    assert!(harness.state().file.activity.exchange_export.is_none());
    assert!(
        observed.recv_timeout(Duration::from_secs(10)).unwrap(),
        "the worker saw the cancellation"
    );
    harness.step();
    assert!(!path.exists());
    assert!(!path.with_extension("step.loss.txt").exists());
    assert!(harness.state().last_side_effect_receipt().is_none());
}

#[test]
fn a_finished_export_is_published_with_its_loss_report_after_consent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("published.step");
    let mut harness = shell();
    queue(&mut harness, &path, |_, _| Ok(b"ISO-10303-21;".to_vec()));
    step_until(
        &mut harness,
        Duration::from_secs(10),
        "export did not finish",
        |app| app.file.activity.exchange_export.is_none(),
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"ISO-10303-21;");
    let report = std::fs::read_to_string(path.with_extension("step.loss.txt")).unwrap();
    assert!(report.contains("occurrence_body_count=1"), "{report}");
    assert!(harness.state().last_side_effect_receipt().is_some());
}
