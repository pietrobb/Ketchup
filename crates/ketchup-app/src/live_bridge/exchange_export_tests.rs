//! Review 2026-10-18 EX-10: a client's STEP or IGES export does not wait for the
//! exact worker on the window's thread. The poll that takes the request starts a
//! job and returns; the window keeps polling, names the export as busy, and
//! answers in a later poll. A cancelled export writes nothing.

use super::*;
use crate::live_bridge::program_check::ProgramCheckJob;
use crate::live_bridge::program_dry_run::QueuedReply;

fn export(path: &Path, export_format: &str) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: export_format.to_owned(),
        drawing_scale: None,
    }
}

fn exporting(bridge: &LiveBridge) -> bool {
    matches!(
        bridge.program_check_job,
        Some(ProgramCheckJob::ExchangeExport(_))
    )
}

#[test]
fn a_clients_iges_export_runs_off_the_windows_thread() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("model.iges");
    let mut app = KetchupApp::new();
    app.selection.clear();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let mut wire = Wire::connect(app);
    wire.call(Request::Status {});

    wire.send(export(&target, "iges"), false);
    let mut reader = wire.stream.try_clone().unwrap();
    let (sender, answers) = mpsc::channel();
    let reading = std::thread::spawn(move || {
        let mut header = [0; 4];
        reader.read_exact(&mut header).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
        reader.read_exact(&mut bytes).unwrap();
        sender
            .send(serde_json::from_slice::<Response>(&bytes).unwrap())
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    while !exporting(wire.app.live.bridge.as_ref().unwrap()) {
        assert!(
            Instant::now() < deadline,
            "the export was never a job: the window waited for the worker"
        );
        assert!(answers.try_recv().is_err(), "answered without a job");
        wire.app.poll_live_bridge(&wire.context);
        std::thread::yield_now();
    }
    let busy = serde_json::to_value(wire.app.live.bridge.as_ref().unwrap().busy_jobs()).unwrap();
    assert_eq!(busy[0]["target"], "export", "{busy}");

    let answer = loop {
        if let Ok(answer) = answers.try_recv() {
            break answer;
        }
        assert!(Instant::now() < deadline, "the export did not finish");
        wire.app.poll_live_bridge(&wire.context);
        std::thread::yield_now();
    };
    reading.join().unwrap();
    assert!(answer.ok, "{answer:?}");
    let result = answer.result.unwrap();
    assert_eq!(result["export_format"], "iges", "{result}");
    assert!(std::fs::metadata(&target).unwrap().len() > 0);
    assert!(!exporting(wire.app.live.bridge.as_ref().unwrap()));
}

#[test]
fn a_cancelled_step_export_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("model.step");
    let (mut app, mut bridge) = setup();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let context = egui::Context::default();
    let (reply, answers) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::clone(&cancelled),
    };

    bridge.start_exchange_export(&mut app, &context, queued, export(&target, "step"), false);
    assert!(exporting(&bridge));
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &context);

    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert!(bridge.program_check_job.is_none());
    // Only the window publishes the file, and the job that would is gone; the
    // worker writes to a directory of its own.
    assert!(!target.exists());
}
