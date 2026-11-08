//! Review 2026-10-09 §5.1 (5): a large program is read in pieces and its
//! parameters change without sending the source.
use super::super::tests::setup;
use super::*;

const SOURCE: &str = "# Stool\nW = param(\"width\", 300)\nH = param(\"height\", 450)\n\n# --- seat ---\ndef seat(name):\n    box(name, (W, W, 20), at = (0, 0, H))\n\nseat(\"seat\")\n\n# --- legs ---\nbox(\"leg\", (30, 30, H))\n";

fn texts(answer: &Value) -> Vec<(u64, String)> {
    answer["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["line"].as_u64().unwrap(),
                row["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn pieces_return_numbered_lines_a_search_or_the_outline() {
    let read = |piece: &Piece| read_piece("stool.star", SOURCE, None, piece).unwrap();
    let range = read(&Piece::Lines(2, 3));
    assert_eq!(
        texts(&range),
        [
            (2, "W = param(\"width\", 300)\n".to_owned()),
            (3, "H = param(\"height\", 450)\n".to_owned())
        ]
    );
    assert_eq!(range["line_count"], 12);
    assert_eq!(range["truncated"], false);
    // A range past the end stops at the last line.
    assert_eq!(texts(&read(&Piece::Lines(12, 40))).len(), 1);

    let found = read(&Piece::Search("BOX(".into()));
    assert_eq!(found["matches"], 2);
    assert_eq!(
        texts(&found)
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [7, 12]
    );

    let outline = read(&Piece::Outline);
    let entries = outline["outline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["line"].as_u64().unwrap(),
                row["kind"].as_str().unwrap(),
                row["last"].as_u64(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        [
            (1, "section", Some(4)),
            (2, "param", None),
            (3, "param", None),
            (5, "section", Some(10)),
            (6, "def", None),
            (11, "section", Some(12)),
        ]
    );

    for (lines, search, part, outline) in [
        (None, None, None, false),
        (Some([3, 2]), None, None, false),
        (Some([0, 2]), None, None, false),
        (None, Some(String::new()), None, false),
        (Some([1, 2]), None, None, true),
    ] {
        assert!(Piece::from_request(lines, search, part, outline).is_err());
    }
}

/// Review 2026-11-06 P3: the program builds with its default where an
/// override is outside the range, so `set_params` refuses such a value
/// itself, naming the parameter and its range, before it is stored.
#[test]
fn set_params_refuses_a_value_outside_the_declared_range() {
    let (mut app, mut bridge) = setup();
    bridge
        .execute(
            &mut app,
            Request::ApplyProgram {
                expected: None,
                source:
                    "W = param(\"width\", 300, min = 100, max = 600)\nbox(\"seat\", (W, W, 20))\n"
                        .into(),
                overrides: None,
                file_name: Some("seat.star".into()),
                replace_document: true,
            },
            false,
        )
        .unwrap();
    let undo = app.undo_step_count();
    let set = |width: f64, stamp: Stamp| Request::SetProgramParams {
        expected: stamp,
        params: BTreeMap::from([("width".to_owned(), width)]),
    };
    let stamp = app.live_bridge_stamp();
    assert!(
        bridge
            .execute(&mut app, set(900., stamp.clone()), false)
            .is_err()
    );
    let details = crate::live_bridge::take_error_details().unwrap();
    assert!(
        details
            .to_string()
            .contains("width = 900 is outside [100, 600]"),
        "{details}"
    );
    assert_eq!(app.undo_step_count(), undo);
    assert!(
        app.document
            .current_rule_program()
            .unwrap()
            .overrides
            .is_empty()
    );
    bridge.execute(&mut app, set(500., stamp), false).unwrap();
    assert_eq!(app.undo_step_count(), undo + 1);
}

/// Review 2026-11-06 r23 P3: program apply and check took `overrides` outside
/// a parameter's range, or no whole number of a count, and apply stored it,
/// while the model was built with the default. Both refuse it now, as
/// set_params does; a value the document already stored still applies.
#[test]
fn apply_and_check_refuse_an_override_the_program_refuses() {
    const NARROW: &str = "W = param(\"width\", 300, min = 100, max = 600)\nN = param(\"count\", 2, whole = True)\nbox(\"seat\", (W, W, 20 * N))\n";
    let (mut app, mut bridge) = setup();
    let apply = |source: &str, overrides: Option<BTreeMap<String, f64>>| Request::ApplyProgram {
        expected: None,
        source: source.into(),
        overrides,
        file_name: Some("seat.star".into()),
        replace_document: true,
    };
    let check = |source: &str, overrides: BTreeMap<String, f64>| Request::CheckProgram {
        expected: None,
        source: Some(source.into()),
        edits: None,
        overrides: Some(overrides),
        file_name: Some("seat.star".into()),
    };
    let wide = NARROW.replace("max = 600", "max = 1000");
    bridge
        .execute(
            &mut app,
            apply(&wide, Some(BTreeMap::from([("width".into(), 900.)]))),
            false,
        )
        .unwrap();
    let undo = app.undo_step_count();
    for (name, value, said) in [
        ("width", 1200., "width = 1200 is outside [100, 1000]"),
        (
            "count",
            2.5,
            "count = 2.5 takes whole numbers under 1e9 only",
        ),
    ] {
        let given = BTreeMap::from([(name.to_owned(), value)]);
        for request in [
            apply(&wide, Some(given.clone())),
            check(&wide, given.clone()),
        ] {
            assert!(bridge.execute(&mut app, request, false).is_err(), "{name}");
            let details = crate::live_bridge::take_error_details().unwrap();
            assert!(details.to_string().contains(said), "{details}");
            assert!(
                details.to_string().contains("param_override_refused"),
                "{details}"
            );
        }
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(
            app.document.current_rule_program().unwrap().overrides,
            BTreeMap::from([("width".to_owned(), 900.)])
        );
    }
    // The program narrowed below the stored 900: kept or sent again, it is
    // only reported, and the model is built with the default.
    for overrides in [None, Some(BTreeMap::from([("width".into(), 900.)]))] {
        let applied = bridge
            .execute(&mut app, apply(NARROW, overrides), false)
            .unwrap();
        assert!(
            applied.to_string().contains("param_out_of_range"),
            "{applied}"
        );
    }
}

#[test]
fn part_piece_and_set_params_work_on_the_open_program() {
    let (mut app, mut bridge) = setup();
    bridge
        .execute(
            &mut app,
            Request::ApplyProgram {
                expected: None,
                source: SOURCE.into(),
                overrides: Some(BTreeMap::from([("height".into(), 400.)])),
                file_name: Some("stool.star".into()),
                replace_document: true,
            },
            false,
        )
        .unwrap();
    let piece = |bridge: &mut LiveBridge, app: &mut KetchupApp, part: &str| {
        bridge.execute(
            app,
            Request::ProgramPiece {
                expected: None,
                lines: None,
                search: None,
                part: Some(part.into()),
                outline: false,
            },
            false,
        )
    };
    let seat = piece(&mut bridge, &mut app, "seat").unwrap();
    assert_eq!(seat["lines_of_part"], json!([[7, 7], [9, 9]]), "{seat}");
    assert_eq!(
        texts(&seat).first().map(|(line, _)| *line),
        Some(5),
        "{seat}"
    );
    assert!(piece(&mut bridge, &mut app, "table").is_err());

    let undo = app.undo_step_count();
    let stamp = app.live_bridge_stamp();
    let before = app.document.current();
    let set = |params: &[(&str, f64)], stamp: Stamp| Request::SetProgramParams {
        expected: stamp,
        params: params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
    };
    assert!(
        bridge
            .execute(&mut app, set(&[("depth", 1.)], stamp.clone()), false)
            .is_err()
    );
    assert_eq!(app.undo_step_count(), undo);
    bridge
        .execute(&mut app, set(&[("width", 500.)], stamp), false)
        .unwrap();
    let program = app.document.current_rule_program().unwrap();
    assert_eq!(program.source, SOURCE);
    assert_eq!(
        program.overrides,
        BTreeMap::from([("height".into(), 400.), ("width".into(), 500.)])
    );
    assert_eq!(app.undo_step_count(), undo + 1);
    assert_ne!(
        app.document.current().features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
}
