//! Review 2026-11-01 UI-100: a client's `export_drawings` of the project set ran on the
//! window's thread, with one exact worker evaluation for each part with modelled threads
//! under a flag no one set, so the window froze for the worker's budget times the
//! parts. It runs as a job now: the window keeps polling and names it as busy, and a
//! cancelled request writes nothing and keeps none of its settings.

use super::*;
use crate::live_bridge::program_check::ProgramCheckJob;
use crate::live_bridge::program_dry_run::QueuedReply;
use ketchup_manufacturing::title_block::TitleField;

/// An M5 × 10 bolt in a plate tapped M5.
const SCREWED: &str = "p = box('plate', (20, 20, 12))\n\
    hole(p, 'z+', at = (10, 10), depth = 12, through = True, id = 'a', thread = ('M5', 10))\n\
    s = bolt('s', 'M5', 10, at = (10, 10, 14.7))\n";

/// A window whose threads are modelled, so each sheet needs their cosmetic bodies.
fn screwed_window() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.selection.clear();
    let program = ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "screwed.star".into(),
        source: SCREWED.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), true).unwrap();
    app.program_evaluations.get_blocking(&program).unwrap();
    assert!(app.set_modeled_threads(true));
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    app
}

fn export(path: &Path) -> Request {
    Request::ExportDrawings {
        expected: None,
        path: path.display().to_string(),
        sheet_id: None,
        format: None,
        title_block: BTreeMap::from([(TitleField::Author, "AI".to_owned())]),
        plan_only: false,
        drawing_scale: None,
    }
}

fn exporting(bridge: &LiveBridge) -> bool {
    matches!(
        bridge.program_check_job,
        Some(ProgramCheckJob::ProjectDrawings(_))
    )
}

#[test]
fn a_clients_drawings_of_modelled_threads_are_made_off_the_windows_thread() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let mut wire = Wire::connect(screwed_window());
    wire.call(Request::Status {});

    wire.send(export(&pdf), false);
    let mut reader = wire.stream.try_clone().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(120);
    while !exporting(wire.app.live.bridge.as_ref().unwrap()) {
        assert!(
            Instant::now() < deadline,
            "the export was never a job: the window drew the sheets itself"
        );
        wire.app.poll_live_bridge(&wire.context);
        assert!(
            answers.recv_timeout(Duration::from_millis(1)).is_err(),
            "answered without a job"
        );
    }
    let bridge = wire.app.live.bridge.as_ref().unwrap();
    let busy = serde_json::to_value(bridge.busy_jobs()).unwrap();
    assert_eq!(busy[0]["target"], "export_drawings", "{busy}");
    assert!(!pdf.exists(), "written only when the window publishes it");

    let answer = loop {
        assert!(Instant::now() < deadline, "the export did not finish");
        wire.app.poll_live_bridge(&wire.context);
        if let Ok(answer) = answers.recv_timeout(Duration::from_millis(1)) {
            break answer;
        }
    };
    reading.join().unwrap();
    assert!(answer.ok, "{answer:?}");
    let result = answer.result.unwrap();
    assert_eq!(result["exported"], true, "{result}");
    assert_eq!(result["title_block"]["author"], "AI", "{result}");
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
}

#[test]
fn a_cancelled_drawings_export_writes_nothing_and_keeps_no_settings() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let (_, mut bridge) = setup();
    let mut app = screwed_window();
    let settings = app.stored_sheet_settings().unwrap();
    let context = egui::Context::default();
    let (reply, answers) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::clone(&cancelled),
    };

    bridge.start_project_drawings_export(&mut app, &context, queued, export(&pdf), false);
    assert!(exporting(&bridge));
    assert_ne!(
        app.stored_sheet_settings().unwrap(),
        settings,
        "kept while it runs"
    );
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &context);

    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert!(bridge.program_check_job.is_none());
    assert_eq!(app.stored_sheet_settings().unwrap(), settings);
    assert!(!app.drawings.unsaved);
    assert!(!pdf.exists(), "the window writes only what it publishes");
}
