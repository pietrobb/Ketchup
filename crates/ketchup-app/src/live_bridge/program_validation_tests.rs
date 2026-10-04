use super::super::measurement::tests::integration_support;
use super::super::tests::setup;
use super::*;

#[test]
fn validator_catalog_uses_existing_document_checks_without_running_or_editing() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    for has_program in [false, true] {
        if has_program {
            app.apply_program_source(source(false), false).unwrap();
        }
        let before = app.document.current().scene_query();
        let history = (app.undo_step_count(), app.redo_step_count());
        let selection = LiveBridge::selection(&app);
        let owner = app.document.current_rule_program().cloned();
        let request = serde_json::from_value(json!({"method":"list_validators"})).unwrap();
        let result = bridge.execute(&mut app, request, true).unwrap();
        assert_eq!(result["catalog_only"], true);
        let rows = result["validators"].as_array().unwrap();
        let ids = rows
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), rows.len());
        for existing in ketchup_application::validation::assistant_validator_catalog() {
            let row = rows.iter().find(|row| row["id"] == existing["id"]).unwrap();
            assert_eq!(row["checks"], existing["checks"]);
            assert_eq!(row["required_roles"], existing["required_roles"]);
            assert_eq!(
                row["run"]["arguments"]["validators"],
                json!([existing["id"]])
            );
        }
        for id in ["program_geometry", "motion", "assembly_path", "tool_access"] {
            assert!(ids.contains(id));
        }
        for row in rows {
            assert!(!row["required_inputs"].as_array().unwrap().is_empty());
            assert!(!row["limitations"].as_array().unwrap().is_empty());
            assert_eq!(row["run"]["tool"], "program");
            assert_eq!(row["run"]["arguments"]["action"], "validate");
        }
        assert_eq!(app.document.current().scene_query(), before);
        assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
        assert_eq!(LiveBridge::selection(&app), selection);
        assert_eq!(app.document.current_rule_program(), owner.as_ref());
    }
}

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
            .validate_program(
                &app,
                Some(before.clone()),
                Arc::new(AtomicBool::new(false)),
                None,
                None,
            )
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
fn requested_motion_finds_hidden_path_collision_without_editing() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    app.new_document();
    let source = RuleProgramSource { file_name: "motion.star".into(), source: "a=box('a',(2,2,2),at=(-100,0,0))\nb=box('b',(2,2,2))\nbox('obstacle',(2,2,2),at=(10,0,0))\njoint(a,b,kind='motion',name='travel',motion=slide((1,0,0),0,20))".into(), overrides: BTreeMap::new() };
    app.apply_program_source(source.clone(), false).unwrap();
    let before = app.document.current().scene_query();
    let history = (app.undo_step_count(), app.redo_step_count());
    for (to, state) in [(3., "passed"), (20., "failed"), (21., "incomplete")] {
        let request = serde_json::from_value(
            json!({"method":"validate_program","motion":{"name":"travel","from":0,"to":to}}),
        )
        .unwrap();
        let result = bridge.execute(&mut app, request, false).unwrap();
        assert_eq!(result["validation"]["motion"]["state"], state, "{result}");
        assert_eq!(result["validation"]["state"], state, "{result}");
        assert_eq!(app.document.current().scene_query(), before);
        assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
        assert_eq!(app.document.current_rule_program(), Some(&source));
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
fn requested_document_checks_use_program_metadata_and_never_mutate_or_detach() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    app.new_document();
    for (material, state) in [
        ("steel", "passed"),
        ("engineered_wood", "failed"),
        ("unknown", "incomplete"),
    ] {
        let source = RuleProgramSource {
            file_name: "structural.star".into(),
            source: format!(
                "box('shelf',(1000,300,20),material='{material}',attributes={{'classification:ketchup.validator-role.v1':'physics.beam.xy'}})"
            ),
            overrides: BTreeMap::new(),
        };
        app.apply_program_source(source.clone(), false).unwrap();
        let before = app.document.current();
        let history = (app.undo_step_count(), app.redo_step_count());
        let selection = LiveBridge::selection(&app);
        let request = serde_json::from_value(
            json!({"method":"validate_program", "validators":["beam_deflection"]}),
        )
        .unwrap();
        let result = bridge.execute(&mut app, request, false).unwrap();
        assert_eq!(result["validation"]["state"], state, "{result:#}");
        let check = &result["validation"]["document_checks"]["beam_deflection"];
        assert_eq!(
            check["state"],
            if state == "incomplete" {
                "not_evaluated"
            } else {
                state
            }
        );
        if material != "unknown" {
            assert_eq!(check["evaluations"][0]["material"], material);
            assert_eq!(check["evaluations"][0]["span_mm"], 1000.0);
        }
        assert_eq!(app.document.current().scene_query(), before.scene_query());
        assert_eq!(
            app.document.current().canonical_digest(),
            before.canonical_digest()
        );
        assert_eq!(app.document.current_rule_program(), Some(&source));
        assert_eq!((app.undo_step_count(), app.redo_step_count()), history);
        assert_eq!(LiveBridge::selection(&app), selection);
    }
    for names in [vec![], vec!["not_a_validator".to_owned()]] {
        let result = bridge
            .validate_program(
                &app,
                None,
                Arc::new(AtomicBool::new(false)),
                Some(names),
                None,
            )
            .unwrap();
        assert_eq!(result["validation"]["state"], "incomplete", "{result:#}");
        assert_ne!(
            result["validation"]["document_checks"]["selection_error"],
            Value::Null
        );
    }
}

#[test]
fn program_validation_reports_preexisting_box_collisions() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let mut program = source(false);
    program.source = "box('a',(10,10,10))\nbox('b',(10,10,10),at=(5,0,0))\n".into();
    app.apply_program_source(program, false).unwrap();
    let result = bridge
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)), None, None)
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
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)), None, None)
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
        .validate_program(&app, None, Arc::new(AtomicBool::new(false)), None, None)
        .unwrap();
    assert_eq!(repaired["validation"]["joints_and_holes"]["holes"], 4);
    assert_eq!(repaired["report"]["errors"], 0, "{repaired}");
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().scene_query(), unfinished);
}
