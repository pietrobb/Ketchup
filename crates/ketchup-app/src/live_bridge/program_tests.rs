use super::*;

const TABLE: &str = include_str!("../../../../examples/programs/table.star");
const APRON: &str = "\napron = board(\"table/apron-front\", (WIDTH - 2 * (INSET + LEG), 20, 80), at = (INSET + LEG, INSET + 25, HEIGHT - 100))\ndowels(legs[0], apron, dowel = \"8x30\", margin = 15)\n";

fn apply(source: &str, replace_document: bool) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: BTreeMap::new(),
        file_name: Some("table.star".to_owned()),
        replace_document,
    }
}

fn names(app: &KetchupApp) -> BTreeMap<String, u64> {
    app.document
        .current()
        .occurrences()
        .map(|occurrence| (occurrence.name().to_owned(), occurrence.id().0))
        .collect()
}

#[test]
fn ai_reads_and_edits_the_window_program_in_one_call_each() {
    let (mut app, mut bridge) = setup();
    let manual = app.live_bridge_stamp();

    assert_eq!(
        bridge.execute(&mut app, apply(TABLE, false), false),
        Err("program_rejected")
    );
    assert_eq!(
        take_error_details().unwrap()["reason"],
        "not_program_document"
    );
    assert_eq!(app.live_bridge_stamp(), manual);
    let empty = bridge
        .execute(&mut app, Request::Program { expected: None }, false)
        .unwrap();
    assert!(empty["source"].is_null());

    let created = bridge.execute(&mut app, apply(TABLE, true), false).unwrap();
    assert_eq!(created["change"], "created", "{created}");
    assert_eq!(created["parts"], 5);
    assert_eq!(created["report"]["ok"], true);
    let table = names(&app);
    let undo_steps = app.undo_step_count();

    let program = bridge
        .execute(&mut app, Request::Program { expected: None }, false)
        .unwrap();
    assert_eq!(program["source"], TABLE);
    let leg = program["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "table/leg-back-left")
        .unwrap();
    assert_eq!(leg["occurrence_id"], table["table/leg-back-left"]);
    assert_eq!(leg["lines"], json!([[13, 13], [17, 17]]));

    let with_apron = format!("{TABLE}{APRON}");
    let added = bridge
        .execute(&mut app, apply(&with_apron, false), false)
        .unwrap();
    assert_eq!(added["change"], "incremental", "{added}");
    assert_eq!(added["added"][0]["name"], "table/apron-front");
    assert_eq!(added["undo_steps"], undo_steps + 1);
    let after = names(&app);
    assert!(table.iter().all(|(name, id)| after[name] == *id));
    assert_eq!(after.len(), 6);

    let unchanged = bridge
        .execute(&mut app, apply(&with_apron, false), false)
        .unwrap();
    assert_eq!(unchanged["change"], "unchanged");
    assert_eq!(unchanged["undo_steps"], undo_steps + 1);

    let before_error = app.live_bridge_stamp();
    assert_eq!(
        bridge.execute(
            &mut app,
            apply(&format!("{with_apron}board(\"broken\", (1, 2)\n"), false),
            false
        ),
        Err("program_rejected")
    );
    let details = take_error_details().unwrap();
    assert_eq!(details["published"], false);
    assert!(
        details["message"]
            .as_str()
            .unwrap()
            .contains("table.star:21:"),
        "{details}"
    );
    assert_eq!(app.live_bridge_stamp(), before_error);

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(names(&app), table);
    assert_eq!(app.document.current_rule_program().unwrap().source, TABLE);
}
