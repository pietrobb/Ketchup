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
    sender.send((report, Some(verdict.clone()))).unwrap();
    bridge.poll_program_check_job(&mut app, &context);
    let paged = page(&mut bridge, &app).unwrap();
    assert_eq!(paged["exact_collisions"], verdict);
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
            sender.send((applied.report.clone(), None)).unwrap();
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
