use super::super::measurement::tests::integration_support;
use super::super::tests::setup;
use super::*;

fn source(drilled: bool) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "validation.star".into(),
        source: format!(
            "p=box('bottom',(100,100,18))\n{}\nq=box('shelf',(100,100,18),at=(0,0,668))\nexpect_gap(p,q,650)\n",
            if drilled {
                "hole(p,'z-',at=(50,50),diameter=8,depth=12)"
            } else {
                ""
            }
        ),
        overrides: BTreeMap::new(),
    }
}

#[test]
fn program_validation_is_read_only_and_keeps_exact_coverage_after_removing_holes() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    for drilled in [true, false] {
        let program = source(drilled);
        app.apply_program_source(program.clone(), false).unwrap();
        let snapshot = app.document.current();
        let id = snapshot.occurrences().next().unwrap().id();
        app.selection.select_occurrence(id, true);
        let before = app.live_bridge_stamp();
        let history = (app.undo_step_count(), app.redo_step_count());
        let selection = LiveBridge::selection(&app);
        let result = bridge
            .validate_program(&app, Some(before.clone()), Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert_eq!(result["canonical_mutation"], false);
        assert_eq!(result["validation_only"], true);
        assert_eq!(result["geometry_evaluated"], true, "{result}");
        assert_eq!(result["exact_collisions"]["state"], "verified", "{result}");
        if drilled {
            assert_eq!(
                result["exact_collisions"]["measurements"][0]["distance_mm"], 650.0,
                "{result}"
            );
        }
        assert_eq!(app.document.current().scene_query(), snapshot.scene_query());
        assert_eq!(app.document.current_rule_program(), Some(&program));
        assert_eq!(app.live_bridge_stamp(), before);
        assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
        assert_eq!(LiveBridge::selection(&app), selection);
    }
}

#[test]
fn program_validation_does_not_publish_results_for_a_replaced_document() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    app.apply_program_source(source(true), false).unwrap();
    let before = app.live_bridge_stamp();
    let (reply, response) = mpsc::sync_channel(1);
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let job = ProgramValidationJob {
        id: 1,
        reply,
        cancelled: cancelled.clone(),
        before,
        started: Instant::now(),
        receiver,
    };
    app.new_document();
    let snapshot = app.document.current();
    sender
        .send(Ok((
            json!({"geometry_evaluated": true}),
            ketchup_program::run("test.star", &source(true).source, &BTreeMap::new())
                .unwrap()
                .1,
        )))
        .unwrap();
    bridge.poll_program_validation(&app, &egui::Context::default(), job);
    let reply = response.try_recv().unwrap();
    assert!(!reply.ok);
    assert!(cancelled.load(Ordering::Acquire));
    assert_eq!(app.document.current().scene_query(), snapshot.scene_query());
    assert!(app.document.current_rule_program().is_none());
    assert_eq!(app.undo_step_count(), 0);
}

#[test]
fn program_validation_reports_preexisting_box_collisions() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let mut program = source(false);
    program.source = "box('a',(10,10,10))\nbox('b',(10,10,10),at=(5,0,0))\n".into();
    app.apply_program_source(program, false).unwrap();
    let result = bridge
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(result["exact_collisions"]["state"], "verified", "{result}");
    assert_eq!(result["exact_collisions"]["collisions"], 1, "{result}");
    assert_eq!(result["report"]["ok"], false, "{result}");
}

#[test]
fn an_impossible_join_remains_editable_without_fabricated_holes_or_hardware() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    app.new_document();
    let mut program = source(false);
    program.source =
        "a=box('a',(100,100,18))\nb=box('b',(18,100,80),at=(0,0,18))\ndowels(a,b,count=30)\n"
            .into();
    let undo = app.undo_step_count();
    app.apply_program_source(program.clone(), false).unwrap();
    assert_eq!(app.undo_step_count(), undo + 1);
    assert_eq!(app.document.current().scene_query().len(), 2);
    let result = bridge
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(result["validation"]["program"]["state"], "accepted");
    assert_eq!(result["validation"]["state"], "failed");
    assert_eq!(result["validation"]["joints_and_holes"]["holes"], 0);
    assert!(
        result["report"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["kind"] == "program_condition_failed")
    );
    let unfinished = app.document.current().scene_query();
    program.source = program.source.replace("count=30", "count=2,margin=20");
    app.apply_program_source(program, false).unwrap();
    let repaired = bridge
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(repaired["validation"]["joints_and_holes"]["holes"], 4);
    assert_eq!(repaired["report"]["errors"], 0, "{repaired}");
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().scene_query(), unfinished);
}
