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
        verdict: None,
        completion: None,
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
        verdict: None,
        completion: None,
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

/// Boxes that overlap, one through a hole (unverified on boxes), one plainly,
/// and a part far off: issues of other checks than collisions, and relations.
const OVERLAPS: &str = "a=box('a',(10,10,10))\nhole(a,'z+',at=(5,5),diameter=2,depth=2)\n\
    box('b',(10,10,10),at=(5,0,0))\nbox('c',(10,10,10),at=(12,0,0))\nbox('d',(5,5,5),at=(0,0,40))\n";

/// The applied program with today's whole report, and the report an exact
/// check would follow instead, which lists only the collisions.
fn applied_before_exact(app: &mut KetchupApp, bridge: &mut LiveBridge) -> (AppliedProgram, Report) {
    let applied = bridge.apply_program(app, apply_source(OVERLAPS), false, &AtomicBool::new(false));
    let applied = applied.unwrap();
    let overrides = BTreeMap::from([("width".to_owned(), 25.)]);
    let evaluated = ketchup_program::evaluate("local.star", OVERLAPS, &overrides).unwrap();
    let pending = ketchup_program::report_before_exact(&evaluated);
    assert!(pending.checks_pending());
    let whole = &applied.report;
    assert!(
        !whole.relations.is_empty() && whole.issues.len() > pending.issues.len(),
        "{whole:#?}"
    );
    (applied, pending)
}

fn check_job(
    applied: AppliedProgram,
    started: Instant,
    receiver: mpsc::Receiver<(Report, Option<Value>, Option<ExactBaseline>)>,
    cancelled: bool,
) -> (ProgramCheckJob, mpsc::Receiver<Response>) {
    let (reply, response) = mpsc::sync_channel(1);
    let job = ExactCheckJob {
        id: 1,
        reply: Some(reply),
        cancelled: Arc::new(AtomicBool::new(cancelled)),
        worker_cancelled: Arc::default(),
        started,
        time: PROGRAM_EXACT_TIMEOUT,
        applied,
        receiver,
        verdict: None,
        completion: None,
    };
    (ProgramCheckJob::Checking(Box::new(job)), response)
}

/// Polls like the window's frames until the job answers, waiting between
/// polls for the report's own checks, which the answer waits for.
fn answer(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    response: &mpsc::Receiver<Response>,
) -> Value {
    let context = egui::Context::default();
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        bridge.poll_program_check_job(app, &context);
        if let Ok(response) = response.try_recv() {
            return response.result.unwrap();
        }
        let Some(ProgramCheckJob::Checking(job)) = &mut bridge.program_check_job else {
            panic!("the job ended without an answer");
        };
        let completion = job.completion.as_mut().expect("the report's checks run");
        // Their report goes back to the job's next poll; a check thread gone
        // leaves the channel disconnected, as it was.
        let (sender, receiver) = mpsc::sync_channel(1);
        match completion
            .receiver
            .recv_timeout(until.saturating_duration_since(Instant::now()))
        {
            Ok(report) => sender.send(report).unwrap(),
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("no answer"),
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        completion.receiver = receiver;
    }
}

/// Review #136: a report that left its issues and relations to the exact check
/// lists them, as today's report does, on every path that publishes it without
/// that check: a timeout, a cancelled request, a worker gone, a missing result.
#[test]
fn a_report_left_for_the_exact_check_answers_whole_when_the_check_does_not() {
    let late =
        Instant::now() - PROGRAM_EXACT_TIMEOUT - PROGRAM_EXACT_GRACE - Duration::from_secs(1);
    for (reason, cancelled, started) in [
        ("exact_collision_timeout", false, late),
        ("request_cancelled", true, Instant::now()),
        ("exact_collision_worker_disconnected", false, Instant::now()),
        ("exact_result_missing", false, Instant::now()),
    ] {
        let (mut app, mut bridge) = setup();
        let (mut applied, pending) = applied_before_exact(&mut app, &mut bridge);
        let expected = applied.result(Some(unfinished(reason)));
        let whole = std::mem::replace(&mut applied.report, pending);
        let (sender, receiver) = mpsc::sync_channel(1);
        // The check thread holds its sender until it answers.
        let _thread = match reason {
            "exact_collision_worker_disconnected" => {
                drop(sender);
                None
            }
            "exact_result_missing" => {
                sender.send((applied.report.clone(), None, None)).unwrap();
                None
            }
            _ => Some(sender),
        };
        let (job, response) = check_job(applied, started, receiver, cancelled);
        bridge.program_check_job = Some(job);
        assert_eq!(
            answer(&mut app, &mut bridge, &response),
            expected,
            "{reason}"
        );
        assert!(bridge.program_check_job.is_none());
        let (_, report, _, exact) = bridge.program_report.as_ref().unwrap();
        assert_eq!(report, &whole, "{reason}");
        assert_eq!(
            exact.as_ref().unwrap()["not_evaluated"][0]["reason"],
            reason
        );
    }
}

/// Review #136: the early answer of a long check lists the whole report too,
/// and so does the report a later edit leaves behind when it ends the check.
#[test]
fn a_report_left_for_the_exact_check_answers_whole_while_the_check_runs() {
    let (mut app, mut bridge) = setup();
    let (mut applied, pending) = applied_before_exact(&mut app, &mut bridge);
    let mut running = unfinished("exact_check_running");
    running["not_evaluated"][0]["next"] = json!(EXACT_CHECK_RUNNING_NEXT);
    let expected = applied.result(Some(running));
    let whole = std::mem::replace(&mut applied.report, pending);
    let (_sender, receiver) = mpsc::sync_channel(1);
    let started = Instant::now() - PROGRAM_EXACT_RUNNING_AFTER;
    let (job, response) = check_job(applied, started, receiver, false);
    bridge.program_check_job = Some(job);
    assert_eq!(answer(&mut app, &mut bridge, &response), expected);
    let Some(ProgramCheckJob::Checking(job)) = &bridge.program_check_job else {
        panic!("the check goes on after its early answer");
    };
    assert_eq!(job.applied.report, whole);
    bridge.supersede_answered_check();
    let (_, report, _, exact) = bridge.program_report.as_ref().unwrap();
    assert_eq!(report, &whole);
    assert_eq!(
        exact.as_ref().unwrap()["not_evaluated"][0]["reason"],
        "exact_check_superseded"
    );
}

/// Review #136: a check the next edit ends before the report's own checks
/// finished stops them, and its report says they did not finish instead of
/// looking whole.
#[test]
fn a_check_superseded_before_its_report_is_whole_stops_its_checks_and_says_so() {
    let (mut app, mut bridge) = setup();
    let (mut applied, pending) = applied_before_exact(&mut app, &mut bridge);
    applied.report = pending;
    let (_sender, receiver) = mpsc::sync_channel(1);
    let (job, _response) = check_job(applied, Instant::now(), receiver, false);
    let ProgramCheckJob::Checking(mut job) = job else {
        unreachable!()
    };
    job.reply = None;
    job.complete_report(&egui::Context::default());
    let checks = Arc::clone(&job.completion.as_ref().unwrap().cancelled);
    bridge.program_check_job = Some(ProgramCheckJob::Checking(job));
    bridge.supersede_answered_check();
    assert!(checks.load(Ordering::Acquire));
    assert!(bridge.program_check_job.is_none());
    let (_, report, _, exact) = bridge.program_report.as_ref().unwrap();
    assert!(!report.checks_pending());
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.message.contains("(superseded:")),
        "{report:#?}"
    );
    assert_eq!(
        exact.as_ref().unwrap()["not_evaluated"][0]["reason"],
        "exact_check_superseded"
    );
}

/// Review #136: when the exact check re-derives the issues, the report's own
/// checks never run: the verdict answers in the frame it arrives.
#[test]
fn a_finished_exact_check_answers_without_the_checks_it_made_redundant() {
    let (mut app, mut bridge) = setup();
    let (mut applied, pending) = applied_before_exact(&mut app, &mut bridge);
    let verdict = json!({"state": "verified"});
    let expected = applied.result(Some(verdict.clone()));
    let refined = std::mem::replace(&mut applied.report, pending);
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.send((refined.clone(), Some(verdict), None)).unwrap();
    let (job, response) = check_job(applied, Instant::now(), receiver, false);
    bridge.program_check_job = Some(job);
    bridge.poll_program_check_job(&mut app, &egui::Context::default());
    assert_eq!(response.try_recv().unwrap().result.unwrap(), expected);
    assert_eq!(bridge.program_report.as_ref().unwrap().1, refined);
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
            verdict: None,
            completion: None,
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
