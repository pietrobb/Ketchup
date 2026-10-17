use super::super::tests::setup;
use super::*;

const SOURCE: &str = "a=box(\"a\", (10,20,30))\nb=box(\"b\", (10,20,30), at=(100,0,0))";

fn check(expected: Option<Stamp>, source: Option<&str>, edits: Option<Vec<SourceEdit>>) -> Request {
    Request::CheckProgram {
        expected,
        source: source.map(str::to_owned),
        edits,
        overrides: None,
        file_name: None,
    }
}

fn apply(source: &str) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.into(),
        overrides: None,
        file_name: Some("check.star".into()),
        replace_document: true,
    }
}

/// Review 2026-10-09 §5.1 (3): an agent can try a program or a patch, with the
/// same answer and exact check as apply, before anything reaches the window.
#[test]
fn check_answers_like_apply_and_publishes_nothing() {
    let (mut app, mut bridge) = setup();
    app.new_document();

    let checked = bridge
        .execute(&mut app, check(None, Some(SOURCE), None), false)
        .unwrap();
    assert_eq!(checked["change"], "created");
    assert_eq!(checked["parts"], 2);
    assert_eq!(checked["requires_replace_document"], false);
    assert!(app.document.current_rule_program().is_none());
    assert_eq!(app.document.current().occurrences().count(), 0);

    bridge.execute(&mut app, apply(SOURCE), false).unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    // A third box that overlaps the second, so the exact check has work to do.
    let patch = vec![SourceEdit {
        old: "at=(100,0,0))".into(),
        new: "at=(100,0,0))\nc=box(\"c\", (10,20,30), at=(105,0,0))".into(),
    }];
    let checked = bridge
        .execute(
            &mut app,
            check(Some(stamp.clone()), None, Some(patch.clone())),
            false,
        )
        .unwrap();
    assert_eq!(checked["check_only"], true);
    assert_eq!(checked["canonical_mutation"], false);
    assert_eq!(checked["change"], "incremental");
    assert_eq!(checked["added_total"], 1);
    assert_eq!(checked["parts"], 3);
    assert_eq!(checked["source_diff"]["added_lines"], 1);

    assert_eq!(app.document.current().scene_query(), before);
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(app.document.current_rule_program().unwrap().source, SOURCE);

    let applied = bridge
        .execute(
            &mut app,
            Request::PatchProgram {
                expected: stamp,
                edits: patch,
            },
            false,
        )
        .unwrap();
    for key in [
        "change",
        "parts",
        "added_total",
        "removed_total",
        "report",
        "geometry_evaluated",
        "validation",
    ] {
        assert_eq!(checked[key], applied[key], "{key}");
    }
}

/// Review 2026-10-16: a check gave its report and exact check one 20 s deadline,
/// an apply its report 60 s and its exact check a fresh 20 s, so the same program
/// could pass one and not the other, and an apply could outlast the connection's
/// 45 s. Both now share the request's one deadline: with it spent, the queued
/// apply's exact check is unfinished like the check's.
#[test]
fn a_queued_check_and_apply_share_one_request_deadline() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let context = egui::Context::default();
    // A prism overlapping a box, so the exact check has work to do.
    let source = "t=extrude(\"t\",profile=[(0,0),(100,0),(0,100)],distance=20)\n\
                  h=box(\"hit\",(20,20,20),at=(10,10,0))";
    // A request answers before the connection gives up on it.
    assert!(
        super::super::program_check::program_request_deadline().remaining() + PROGRAM_EXACT_GRACE
            <= ketchup_mcp::PROGRAM_RESPONSE_WAIT
    );
    let spent = || CheckDeadline::after(Duration::ZERO);
    let answer =
        |bridge: &mut LiveBridge, app: &mut KetchupApp, response: mpsc::Receiver<Response>| {
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                bridge.poll_program_check_job(app, &context);
                if let Ok(response) = response.try_recv() {
                    assert!(response.ok, "{response:?}");
                    break response.result.unwrap();
                }
                assert!(
                    Instant::now() < deadline,
                    "the program request did not answer"
                );
                std::thread::yield_now();
            }
        };

    let (reply, response) = mpsc::sync_channel(1);
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    bridge.start_queued_check_program(
        &app,
        &context,
        queued,
        check(None, Some(source), None),
        false,
        spent(),
    );
    let checked = answer(&mut bridge, &mut app, response);

    let (reply, response) = mpsc::sync_channel(1);
    bridge.start_queued_apply_program(
        &mut app,
        &context,
        2,
        reply,
        Arc::new(AtomicBool::new(false)),
        apply(source),
        false,
        spent(),
    );
    let applied = answer(&mut bridge, &mut app, response);
    assert_eq!(app.document.current().occurrences().count(), 2);

    assert_eq!(
        checked["exact_collisions"]["state"], "incomplete",
        "{checked}"
    );
    for key in [
        "exact_collisions",
        "geometry_evaluated",
        "report",
        "validation",
    ] {
        assert_eq!(checked[key], applied[key], "{key}");
    }
}

#[test]
fn a_rejected_or_ambiguous_check_publishes_nothing() {
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(SOURCE), false).unwrap();
    let before = app.document.current().scene_query();
    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    let edit = vec![SourceEdit {
        old: "(10,20,30))".into(),
        new: "(10,20))".into(),
    }];
    for request in [
        check(None, Some("a=box(\"a\", (10,20))"), None),
        check(Some(stamp.clone()), Some(SOURCE), Some(edit.clone())),
        check(None, None, Some(edit)),
        check(Some(stamp), None, None),
    ] {
        assert!(bridge.execute(&mut app, request, false).is_err());
        assert_eq!(app.document.current().scene_query(), before);
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(app.document.current_rule_program().unwrap().source, SOURCE);
    }
}
