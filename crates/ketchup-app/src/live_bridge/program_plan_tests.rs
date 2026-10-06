use super::super::tests::setup;
use super::*;

fn source() -> RuleProgramSource {
    RuleProgramSource {
        file_name: "queued.star".into(),
        source: "box(\"part\", (10, 20, 30))\n".into(),
        overrides: BTreeMap::new(),
    }
}

#[test]
fn cancelled_or_replaced_document_never_publishes_a_finished_plan() {
    for cancel in [true, false] {
        let (mut app, mut bridge) = setup();
        app.new_document();
        let context = egui::Context::default();
        let source = source();
        let original = app.live_bridge_stamp();
        let plan = plan_with_report(&app.document.fork_for_planning(), &source).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (reply, response) = mpsc::sync_channel(1);
        let (sender, receiver) = mpsc::sync_channel(1);
        bridge.program_check_job = Some(ProgramCheckJob::Planning(ProgramPlanJob {
            id: 1,
            reply,
            cancelled: cancelled.clone(),
            source,
            replace: false,
            before: original,
            receiver,
        }));
        bridge.poll_program_check_job(&mut app, &context);
        assert!(bridge.program_check_job.is_some());
        assert!(app.document.current().occurrences().next().is_none());
        // The queue remains usable while the planner has not answered.
        assert!(bridge.execute(&mut app, Request::Status {}, false).is_ok());
        if cancel {
            cancelled.store(true, Ordering::Release);
        } else {
            app.new_document();
        }
        let history = (app.undo_step_count(), app.redo_step_count());
        sender.send(Ok(plan)).unwrap();
        bridge.poll_program_check_job(&mut app, &context);
        assert!(!response.try_recv().unwrap().ok);
        assert!(bridge.program_check_job.is_none());
        assert!(app.document.current_rule_program().is_none());
        assert!(app.document.current().occurrences().next().is_none());
        assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
    }
}

#[test]
fn client_eof_while_program_plan_is_pending_prevents_publication() {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpStream};
    let (mut app, mut bridge) = setup();
    app.new_document();
    let context = egui::Context::default();
    let source = source();
    let mut stream = TcpStream::connect(bridge.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let bytes = serde_json::to_vec(&json!({
        "version": 1, "id": 1, "token": bridge.token,
        "request": {"method": "apply_program", "source": source.source}
    }))
    .unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
    let queued = bridge.queue.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(queued.request, Request::ApplyProgram { .. }));
    let cancelled = queued.cancelled.clone();
    let before = app.live_bridge_stamp();
    let history = (app.undo_step_count(), app.redo_step_count());
    let (sender, receiver) = mpsc::sync_channel(1);
    bridge.program_check_job = Some(ProgramCheckJob::Planning(ProgramPlanJob {
        id: queued.id,
        reply: queued.reply,
        cancelled: queued.cancelled,
        source: source.clone(),
        replace: false,
        before,
        receiver,
    }));
    bridge.poll_program_check_job(&mut app, &context);
    assert!(bridge.execute(&mut app, Request::Status {}, false).is_ok());
    // The real transport observes EOF before the held planner result is released.
    stream.shutdown(Shutdown::Write).unwrap();
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    assert!(cancelled.load(Ordering::Acquire));
    let plan = plan_with_report(&app.document.fork_for_planning(), &source).unwrap();
    sender.send(Ok(plan)).unwrap();
    bridge.poll_program_check_job(&mut app, &context);
    assert!(bridge.program_check_job.is_none());
    assert!(app.document.current().occurrences().next().is_none());
    assert!(app.document.current_rule_program().is_none());
    assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
}

#[test]
fn live_program_planner_publishes_one_undo_step() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let context = egui::Context::default();
    let (reply, response) = mpsc::sync_channel(1);
    let source = source();
    let history = app.undo_step_count();
    bridge.start_queued_apply_program(
        &mut app,
        &context,
        1,
        reply,
        Arc::new(AtomicBool::new(false)),
        Request::ApplyProgram {
            expected: None,
            source: source.source.clone(),
            file_name: Some(source.file_name.clone()),
            overrides: source.overrides.clone(),
            replace_document: false,
        },
        false,
    );
    assert!(bridge.program_check_job.is_some());
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        bridge.poll_program_check_job(&mut app, &context);
        if let Ok(result) = response.try_recv() {
            break result;
        }
        assert!(Instant::now() < deadline, "program planner did not finish");
        std::thread::yield_now();
    };
    assert!(result.ok, "{result:?}");
    assert_eq!(app.document.current().occurrences().count(), 1);
    assert_eq!(app.document.current_rule_program(), Some(&source));
    assert_eq!(app.undo_step_count(), history + 1);
    app.document.undo().unwrap();
    assert!(app.document.current().occurrences().next().is_none());
    app.document.redo().unwrap();
    assert_eq!(app.document.current().occurrences().count(), 1);
    assert_eq!(app.document.current_rule_program(), Some(&source));
}
