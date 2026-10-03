use super::super::tests::setup;
use super::*;

#[test]
fn queued_validation_returns_native_distance_without_changing_the_document() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let source = RuleProgramSource { file_name: "readonly.star".into(), source: "a=box('a',(100,100,18))\nhole(a,'z+',at=(50,50),diameter=8,depth=12)\nb=box('b',(100,100,18),at=(0,0,668))\nexpect_gap(a,b,650)".into(), overrides: BTreeMap::new() };
    app.apply_program_source(source.clone(), false).unwrap();
    let before = app.document.current();
    let history = (app.undo_step_count(), app.redo_step_count());
    let selection = LiveBridge::selection(&app);
    let context = egui::Context::default();
    let (reply, response) = mpsc::sync_channel(1);
    bridge.start_queued_validate_program(
        &app,
        &context,
        1,
        reply,
        Arc::new(AtomicBool::new(false)),
        Some(app.live_bridge_stamp()),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let response = loop {
        bridge.poll_program_check_job(&mut app, &context);
        if let Ok(response) = response.try_recv() {
            break response;
        }
        assert!(Instant::now() < deadline, "validation did not complete");
        std::thread::yield_now();
    };
    assert!(response.ok, "{response:?}");
    let result = response.result.unwrap();
    assert_eq!(
        result["exact_collisions"]["measurements"][0]["distance_mm"], 650.0,
        "{result}"
    );
    assert_eq!(result["validation"]["state"], "passed", "{result}");
    assert_eq!(
        bridge.program_report.as_ref().unwrap().2,
        "explicit_validation"
    );
    assert_eq!(app.document.current().scene_query(), before.scene_query());
    assert_eq!(app.document.current_rule_program(), Some(&source));
    assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
    assert_eq!(LiveBridge::selection(&app), selection);
}

#[test]
fn cancelled_expired_and_disconnected_validation_leave_the_model_and_history_untouched() {
    for mode in ["cancelled", "timeout", "disconnected"] {
        let (mut app, mut bridge) = setup();
        app.new_document();
        app.apply_program_source(
            RuleProgramSource {
                file_name: "part.star".into(),
                source: "box('part',(10,20,30))".into(),
                overrides: BTreeMap::new(),
            },
            false,
        )
        .unwrap();
        let before = app.document.current();
        let history = (app.undo_step_count(), app.redo_step_count());
        let (reply, response) = mpsc::sync_channel(1);
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(mode == "cancelled"));
        if mode != "disconnected" {
            sender
                .send(Ok((
                    json!({"geometry_evaluated": true}),
                    ketchup_program::run("test.star", "box('part',(10,20,30))", &BTreeMap::new())
                        .unwrap()
                        .1,
                )))
                .unwrap();
        }
        drop(sender);
        let job = ProgramValidationJob {
            id: 1,
            reply,
            cancelled,
            before: app.live_bridge_stamp(),
            started: if mode == "timeout" {
                Instant::now() - Duration::from_secs(30)
            } else {
                Instant::now()
            },
            receiver,
        };
        bridge.poll_program_validation(&app, &egui::Context::default(), job);
        assert!(!response.try_recv().unwrap().ok, "{mode}");
        assert_eq!(
            app.document.current().scene_query(),
            before.scene_query(),
            "{mode}"
        );
        assert_eq!(
            (app.undo_step_count(), app.redo_step_count()),
            history,
            "{mode}"
        );
    }
}
