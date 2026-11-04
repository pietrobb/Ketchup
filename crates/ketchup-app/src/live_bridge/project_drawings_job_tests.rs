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
        sheet_language: None,
        replace: false,
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
    // Review 2026-11-02 MCP-123: it said "DWG import" among other jobs.
    assert_eq!(
        busy[0]["reason"], "The project drawings are being drawn and written.",
        "{busy}"
    );
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

/// Review 2026-11-04 MCP-141: another client taking the window over, or the
/// user's Disconnect, dropped the bridge with the export job in it, so the
/// title block the export kept stayed in the document, and the client was
/// told its request stopped and the document did not change. The job ends as
/// a cancelled one does: its worker stops and the settings are put back; it
/// sends no answer, so the connection says why it ended.
#[test]
fn a_takeover_or_disconnect_puts_back_what_a_running_export_kept() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let mut app = screwed_window();
    let settings = app.stored_sheet_settings().unwrap();
    for takeover in [true, false] {
        let (_, mut bridge) = setup();
        let (answers, cancelled) = started_export(&mut bridge, &mut app, &pdf);
        assert!(exporting(&bridge));
        assert_ne!(app.stored_sheet_settings().unwrap(), settings);
        app.live.bridge = Some(bridge);
        if takeover {
            app.supersede_live_bridge();
        } else {
            app.revoke_live_consent();
        }
        assert!(app.live.bridge.is_none());
        assert!(cancelled.load(Ordering::Acquire), "its worker stops");
        assert_eq!(app.stored_sheet_settings().unwrap(), settings, "{takeover}");
        assert!(!app.drawings.unsaved, "{takeover}");
        assert!(
            matches!(answers.try_recv(), Err(mpsc::TryRecvError::Disconnected)),
            "no answer: the connection says why it ended"
        );
    }
    assert!(!pdf.exists());
}

fn started_export(
    bridge: &mut LiveBridge,
    app: &mut KetchupApp,
    pdf: &Path,
) -> (mpsc::Receiver<Response>, Arc<AtomicBool>) {
    let (reply, answers) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::clone(&cancelled),
    };
    bridge.start_project_drawings_export(
        app,
        &egui::Context::default(),
        queued,
        export(pdf),
        false,
    );
    assert!(exporting(bridge));
    (answers, cancelled)
}

/// Review 2026-11-02 MCP-122: the window stays usable while a set is drawn.
/// New and Open did not end the job, and its end put the first document's
/// settings into the one open then, outside Undo, and marked them saved.
#[test]
fn new_and_open_end_a_drawings_export_without_touching_the_next_document() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let other = directory.path().join("other.ketchup");
    let mut saved = KetchupApp::new();
    assert!(saved.save_document_to(&other), "{}", saved.digest);
    let users = |author: &str| {
        let mut settings = ketchup_manufacturing::title_block::SheetSettings::default();
        settings
            .title_block
            .insert(TitleField::Author, author.to_owned());
        settings
    };
    let context = egui::Context::default();
    for open in [false, true] {
        let (_, mut bridge) = setup();
        let mut app = screwed_window();
        let (answers, _cancelled) = started_export(&mut bridge, &mut app, &pdf);
        app.live.bridge = Some(bridge);
        if open {
            assert!(app.open_document_from(&other), "{}", app.digest);
        } else {
            app.new_document();
        }
        let mut bridge = app.live.bridge.take().unwrap();
        // The user fills in the next document's title block.
        app.store_sheet_settings(users("Owner"));
        app.drawings.unsaved = true;
        bridge.poll_program_check_job(&mut app, &context);

        let answer = answers.try_recv().expect("answered in the next poll");
        assert_eq!(
            answer.error.as_deref(),
            Some("stale_document"),
            "open {open}"
        );
        assert!(bridge.program_check_job.is_none());
        assert_eq!(app.stored_sheet_settings().unwrap(), users("Owner"));
        assert!(
            app.drawings.unsaved,
            "the user's settings are still unsaved"
        );
        assert!(!pdf.exists());
    }
}

/// `app` evaluating its exact solids again, as after an edit.
fn evaluating(app: &mut KetchupApp) {
    app.exact.task = Some(ketchup_application::evaluation::start_exact_evaluation(
        app.document.current(),
        &app.file.container_data,
        &app.exact.results,
        &app.exact.topology_results,
        app.exact.worker_path.clone(),
        || {},
    ));
}

/// Měchenice MECH-4: `export_drawings` sent while the exact evaluation still
/// ran was refused ("the sheet would miss parts"), so the client had to poll
/// and send it again. It now waits in the window for the evaluation and then
/// draws the set from its solids, in the one request.
#[test]
fn a_drawings_export_sent_during_the_exact_evaluation_waits_for_it() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let (_, mut bridge) = setup();
    let mut app = screwed_window();
    evaluating(&mut app);
    let context = egui::Context::default();
    let (answers, _cancelled) = {
        let (reply, answers) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let queued = QueuedReply {
            id: 1,
            reply,
            cancelled: Arc::clone(&cancelled),
        };
        bridge.start_project_drawings_export(&mut app, &context, queued, export(&pdf), false);
        (answers, cancelled)
    };
    if let Ok(answer) = answers.try_recv() {
        panic!("answered before the evaluation ended: {answer:?}");
    }
    assert!(exporting(&bridge), "the export does not wait as a job");
    let busy = serde_json::to_value(bridge.busy_jobs()).unwrap();
    assert_eq!(busy[0]["target"], "export_drawings", "{busy}");
    assert_eq!(
        busy[0]["reason"],
        "The drawings wait for the exact evaluation, then are drawn and written.",
        "{busy}"
    );

    let deadline = Instant::now() + Duration::from_secs(120);
    let answer = loop {
        assert!(Instant::now() < deadline, "the export did not finish");
        app.refresh_exact_products(&context);
        bridge.poll_program_check_job(&mut app, &context);
        if let Ok(answer) = answers.try_recv() {
            break answer;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(app.exact.task.is_none(), "answered before the evaluation");
    assert!(answer.ok, "{answer:?}");
    assert_eq!(answer.result.unwrap()["exported"], true);
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
}

/// MECH-4 for one drawing sheet, sent through the socket: it waited for no
/// evaluation either ("the sheet would draw old solids").
#[test]
fn a_sheet_exported_during_the_exact_evaluation_waits_for_it() {
    let (mut app, mut bridge) = setup();
    let id = super::drawing_sheet::sheet_of_parts(&mut app, &mut bridge, 1);
    drop(bridge);
    let directory = tempfile::tempdir().unwrap();
    let dxf = directory.path().join("block.dxf");
    let mut wire = Wire::connect(app);
    wire.call(Request::Status {});
    evaluating(&mut wire.app);
    let mut request = export(&dxf);
    if let Request::ExportDrawings {
        sheet_id,
        title_block,
        ..
    } = &mut request
    {
        *sheet_id = Some(id);
        title_block.clear();
    }
    wire.send(request, false);
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
    // The evaluation is not taken in until the window holds the request.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !exporting(wire.app.live.bridge.as_ref().unwrap()) {
        assert!(Instant::now() < deadline, "the sheet never waited as a job");
        wire.app.poll_live_bridge(&wire.context);
        if let Ok(answer) = answers.recv_timeout(Duration::from_millis(5)) {
            panic!("answered before the evaluation ended: {answer:?}");
        }
    }
    let answer = loop {
        assert!(Instant::now() < deadline, "the sheet was not exported");
        wire.app.refresh_exact_products(&wire.context);
        wire.app.poll_live_bridge(&wire.context);
        if let Ok(answer) = answers.recv_timeout(Duration::from_millis(5)) {
            break answer;
        }
    };
    reading.join().unwrap();
    assert!(wire.app.exact.task.is_none());
    assert!(answer.ok, "{answer:?}");
    assert_eq!(answer.result.unwrap()["sheet_id"], id);
    assert!(std::fs::read_to_string(&dxf).unwrap().contains("ENTITIES"));
}

/// A client that gives up while the export waits for the evaluation gets
/// nothing written and leaves the window's settings as they were.
#[test]
fn a_drawings_export_cancelled_while_it_waits_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let (_, mut bridge) = setup();
    let mut app = screwed_window();
    let settings = app.stored_sheet_settings().unwrap();
    evaluating(&mut app);
    let context = egui::Context::default();
    let (reply, answers) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::clone(&cancelled),
    };
    bridge.start_project_drawings_export(&mut app, &context, queued, export(&pdf), false);
    assert!(
        answers.try_recv().is_err(),
        "answered before the evaluation ended"
    );
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &context);

    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert!(bridge.program_check_job.is_none());
    assert_eq!(app.stored_sheet_settings().unwrap(), settings);
    assert!(!app.drawings.unsaved);
    assert!(!pdf.exists());
}

/// The user changes the title block in the window while the set is drawn,
/// then the client gives up: the user's change stays.
#[test]
fn a_cancelled_drawings_export_keeps_what_the_user_changed_meanwhile() {
    let directory = tempfile::tempdir().unwrap();
    let pdf = directory.path().join("screwed.pdf");
    let (_, mut bridge) = setup();
    let mut app = screwed_window();
    let (answers, cancelled) = started_export(&mut bridge, &mut app, &pdf);
    let mut users = app.stored_sheet_settings().unwrap();
    users
        .title_block
        .insert(TitleField::Author, "Owner".to_owned());
    app.store_sheet_settings(users.clone());
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &egui::Context::default());

    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert_eq!(app.stored_sheet_settings().unwrap(), users);
    assert!(app.drawings.unsaved);
}
