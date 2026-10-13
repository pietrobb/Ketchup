//! #2477 step 2: program variants through the bridge.
use super::super::tests::setup;
use super::*;

const CABINET: &str = "W = param(\"width\", 600)\nH = param(\"height\", 720)\n\
box(\"left\", (18, 400, H))\nbox(\"right\", (18, 400, H), at = (W, 0, 0))\n";

/// Where the right side stands and how tall it is in the window's program.
fn right_side(app: &KetchupApp) -> (f64, f64) {
    let program = app.document.current_rule_program().unwrap();
    let evaluated = app.program_evaluations.get_blocking(program).unwrap();
    let (min, max) = evaluated.model.part("right").unwrap().world_bounds();
    (min[0], max[2] - min[2])
}

fn variant(app: &KetchupApp, action: &str, name: &str) -> Request {
    let (expected, name) = (app.live_bridge_stamp(), name.to_owned());
    match action {
        "save" => Request::SaveProgramVariant { expected, name },
        "use" => Request::UseProgramVariant { expected, name },
        _ => Request::DeleteProgramVariant { expected, name },
    }
}

fn run(bridge: &mut LiveBridge, app: &mut KetchupApp, request: Request) -> Value {
    bridge
        .execute(app, request, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()))
}

fn act(bridge: &mut LiveBridge, app: &mut KetchupApp, action: &str, name: &str) -> Value {
    let request = variant(app, action, name);
    run(bridge, app, request)
}

fn apply(source: &str) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: None,
        file_name: Some("cabinet.star".into()),
        replace_document: true,
    }
}

fn saved(app: &KetchupApp) -> Vec<String> {
    let program = app.document.current_rule_program().unwrap();
    program.variants.keys().cloned().collect()
}

#[test]
fn a_variant_switches_every_parameter_in_one_undo_step() {
    let (mut app, mut bridge) = setup();
    run(&mut bridge, &mut app, apply(CABINET));
    let context = Request::ProgramContext {
        expected: None,
        selection_context: false,
    };
    let read = run(&mut bridge, &mut app, context);
    assert!(read.get("variants").is_none(), "{read}");
    for (name, width, height) in [
        ("600", 600., 720.),
        ("800", 800., 720.),
        ("1000", 1000., 900.),
    ] {
        let params = BTreeMap::from([("width".to_owned(), width), ("height".to_owned(), height)]);
        let expected = app.live_bridge_stamp();
        run(
            &mut bridge,
            &mut app,
            Request::SetProgramParams { expected, params },
        );
        let undo = app.undo_step_count();
        let answer = act(&mut bridge, &mut app, "save", name);
        assert_eq!(answer["saved"], name, "{answer}");
        assert_eq!(app.undo_step_count(), undo + 1, "saving is one Undo step");
        assert_eq!(right_side(&app), (width, height), "saving changes no part");
    }
    assert_eq!(saved(&app), ["1000", "600", "800"]);
    let values = &app.document.current_rule_program().unwrap().variants["1000"];
    assert_eq!(
        values,
        &BTreeMap::from([("height".to_owned(), 900.), ("width".to_owned(), 1000.)])
    );

    act(&mut bridge, &mut app, "use", "600");
    assert_eq!(right_side(&app), (600., 720.));
    let undo = app.undo_step_count();
    act(&mut bridge, &mut app, "use", "800");
    assert_eq!(right_side(&app), (800., 720.));
    assert_eq!(app.undo_step_count(), undo + 1);
    let read = run(
        &mut bridge,
        &mut app,
        Request::ProgramContext {
            expected: None,
            selection_context: false,
        },
    );
    assert_eq!(read["variants"]["active"], "800", "{read}");
    assert_eq!(read["variants"]["saved"]["600"]["width"], 600.0, "{read}");
    let full = run(&mut bridge, &mut app, Request::Program { expected: None });
    assert_eq!(full["variants"], read["variants"], "{full}");
    run(&mut bridge, &mut app, Request::Undo { expected: None });
    assert_eq!(right_side(&app), (600., 720.));

    // Width and height change together and one Undo restores both.
    let undo = app.undo_step_count();
    act(&mut bridge, &mut app, "use", "1000");
    assert_eq!(right_side(&app), (1000., 900.));
    assert_eq!(app.undo_step_count(), undo + 1);
    run(&mut bridge, &mut app, Request::Undo { expected: None });
    assert_eq!(right_side(&app), (600., 720.));
    assert_eq!(saved(&app).len(), 3, "Undo of a use keeps the variants");

    act(&mut bridge, &mut app, "delete", "800");
    assert_eq!(saved(&app), ["1000", "600"]);
    run(&mut bridge, &mut app, Request::Undo { expected: None });
    assert_eq!(saved(&app), ["1000", "600", "800"]);
}

#[test]
fn an_unknown_or_outdated_variant_is_refused_without_a_change() {
    let (mut app, mut bridge) = setup();
    run(&mut bridge, &mut app, apply(CABINET));
    act(&mut bridge, &mut app, "save", "600");
    let refused = |bridge: &mut LiveBridge, app: &mut KetchupApp, action: &str, name: &str| {
        let request = variant(app, action, name);
        let (undo, program) = (
            app.undo_step_count(),
            app.document.current_rule_program().cloned(),
        );
        assert_eq!(bridge.execute(app, request, false), Err("invalid_params"));
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(app.document.current_rule_program().cloned(), program);
        take_error_details().unwrap().to_string()
    };
    let unknown = refused(&mut bridge, &mut app, "use", "1200");
    assert!(
        unknown.contains("\\\"1200\\\"") && unknown.contains("600"),
        "{unknown}"
    );
    let unknown = refused(&mut bridge, &mut app, "delete", "1200");
    assert!(unknown.contains("1200"), "{unknown}");
    let empty = refused(&mut bridge, &mut app, "save", " ");
    assert!(empty.contains("name"), "{empty}");

    // The program loses its height parameter; the saved variant still sets it.
    let fixed = CABINET.replace("H = param(\"height\", 720)", "H = 720");
    run(&mut bridge, &mut app, apply(&fixed));
    assert_eq!(
        saved(&app),
        ["600"],
        "a new source of the file keeps its variants"
    );
    let outdated = refused(&mut bridge, &mut app, "use", "600");
    assert!(outdated.contains("height"), "{outdated}");
}
