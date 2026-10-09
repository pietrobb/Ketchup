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
