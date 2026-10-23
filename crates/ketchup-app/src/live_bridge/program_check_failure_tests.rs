use super::super::tests::{program_access::apply_source, setup};
use super::*;

/// A late check: the apply answers incomplete, the report busy, then its verdict.
#[test]
fn a_late_exact_check_answers_incomplete_then_pages_its_verdict() {
    let (mut app, mut bridge) = setup();
    let request = apply_source("box('a',(10,10,10))");
    let applied = bridge.apply_program(&mut app, request, false, &AtomicBool::new(false));
    let (applied, verdict) = (applied.unwrap(), json!({"state": "verified"}));
    let (stamp, report) = (applied.stamp.clone(), applied.report.clone());
    let ((reply, response), (sender, receiver)) = (mpsc::sync_channel(1), mpsc::sync_channel(1));
    bridge.program_check_job = Some(ProgramCheckJob::Checking(Box::new(ExactCheckJob {
        id: 1,
        reply: Some(reply),
        cancelled: Arc::default(),
        worker_cancelled: Arc::default(),
        started: Instant::now() - PROGRAM_EXACT_RUNNING_AFTER,
        time: PROGRAM_EXACT_TIMEOUT,
        applied,
        receiver,
    })));
    let context = egui::Context::default();
    bridge.poll_program_check_job(&mut app, &context);
    let running = response.try_recv().unwrap().result.unwrap();
    assert_eq!(running["validation"]["state"], "incomplete", "{running}");
    let page = |bridge: &mut LiveBridge, app: &KetchupApp| {
        bridge.program_report_page(app, stamp.clone(), ReportSection::Issues, 0, 50)
    };
    assert_eq!(page(&mut bridge, &app), Err("busy"));
    sender.send((report, Some(verdict.clone()), None)).unwrap();
    bridge.poll_program_check_job(&mut app, &context);
    let paged = page(&mut bridge, &app).unwrap();
    assert_eq!(paged["exact_collisions"], verdict);
}

/// Review 10-21: the early answer says how to get the verdict, and a check that
/// already answered gives way to the next request that needs the window instead
/// of answering it busy; a read waits for it.
#[test]
fn an_answered_exact_check_names_its_report_and_gives_way_to_the_next_edit() {
    let (mut app, mut bridge) = setup();
    let request = apply_source("box('a',(10,10,10))");
    let applied = bridge.apply_program(&mut app, request, false, &AtomicBool::new(false));
    let applied = applied.unwrap();
    let stamp = applied.stamp.clone();
    let ((reply, response), (_sender, receiver)) = (mpsc::sync_channel(1), mpsc::sync_channel(1));
    let worker_cancelled = Arc::new(AtomicBool::new(false));
    bridge.program_check_job = Some(ProgramCheckJob::Checking(Box::new(ExactCheckJob {
        id: 1,
        reply: Some(reply),
        cancelled: Arc::default(),
        worker_cancelled: Arc::clone(&worker_cancelled),
        started: Instant::now() - PROGRAM_EXACT_RUNNING_AFTER,
        time: PROGRAM_EXACT_TIMEOUT,
        applied,
        receiver,
    })));
    bridge.poll_program_check_job(&mut app, &egui::Context::default());
    let running = response.try_recv().unwrap().result.unwrap();
    let next = &running["exact_collisions"]["not_evaluated"][0]["next"];
    assert!(
        next.as_str()
            .unwrap_or_default()
            .contains("program action=report"),
        "{running}"
    );

    let (queue, queued) = mpsc::sync_channel(4);
    bridge.queue = queued;
    let other_client = bridge.session + 1;
    app.live.bridge = Some(bridge);
    let send = |app: &mut KetchupApp, request: Request| {
        let (reply, answer) = mpsc::sync_channel(1);
        queue
            .send(Queued {
                session: other_client,
                id: 2,
                request,
                connection_closed: false,
                cancelled: Arc::default(),
                reply,
            })
            .unwrap();
        app.poll_live_bridge(&egui::Context::default());
        answer.try_recv().unwrap()
    };
    let status = send(&mut app, Request::Status {});
    assert_eq!(status.error.as_deref(), Some("busy"));
    assert!(!worker_cancelled.load(Ordering::Acquire));
    let undo = send(&mut app, Request::Undo { expected: None });
    assert_eq!(undo.error, None, "{:?}", undo.result);
    assert!(worker_cancelled.load(Ordering::Acquire));
    let bridge = app.live.bridge.as_mut().unwrap();
    assert!(bridge.program_check_job.is_none());
    let (paged_stamp, _, _, exact) = bridge.program_report.as_ref().unwrap();
    assert_eq!(*paged_stamp, stamp);
    assert_eq!(
        exact.as_ref().unwrap()["not_evaluated"][0]["reason"],
        "exact_check_superseded"
    );
}

#[test]
fn missing_exact_result_and_worker_failure_never_claim_verified_or_revert_the_edit() {
    for missing in [true, false] {
        let (mut app, mut bridge) = setup();
        app.new_document();
        let applied = bridge
            .apply_program(
                &mut app,
                Request::ApplyProgram {
                    expected: None,
                    source: "a=box('a',(10,10,10))\nhole(a,'z+',at=(5,5),diameter=2,depth=2)"
                        .into(),
                    file_name: None,
                    overrides: Some(BTreeMap::new()),
                    replace_document: false,
                },
                false,
                &AtomicBool::new(false),
            )
            .unwrap();
        let original = app.document.current();
        let history = app.undo_step_count();
        let (reply, response) = mpsc::sync_channel(1);
        let (sender, receiver) = mpsc::sync_channel(1);
        if missing {
            sender.send((applied.report.clone(), None, None)).unwrap();
        }
        drop(sender);
        bridge.program_check_job = Some(ProgramCheckJob::Checking(Box::new(ExactCheckJob {
            id: 1,
            reply: Some(reply),
            cancelled: Arc::new(AtomicBool::new(false)),
            worker_cancelled: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            time: PROGRAM_EXACT_TIMEOUT,
            applied,
            receiver,
        })));
        bridge.poll_program_check_job(&mut app, &egui::Context::default());
        let response = response.try_recv().unwrap();
        assert!(response.ok, "the edit was already published: {response:?}");
        let result = response.result.unwrap();
        assert_eq!(result["geometry_evaluated"], false);
        assert_eq!(result["exact_collisions"]["state"], "incomplete");
        assert_eq!(result["validation"]["state"], "incomplete");
        assert_eq!(app.document.current().scene_query(), original.scene_query());
        assert_eq!(app.undo_step_count(), history);
        app.document.undo().unwrap();
        assert!(app.document.current().occurrences().next().is_none());
        app.document.redo().unwrap();
        assert_eq!(app.document.current().scene_query(), original.scene_query());
    }
}
