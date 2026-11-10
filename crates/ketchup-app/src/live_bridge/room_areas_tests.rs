//! `drawing action=rooms` (review 2026-11-08 §5.3): the rooms of an architect's plan get a
//! solid hatch, their name and area, and a table of rooms with the total, written into the
//! drawing as one Undo step and exported as HATCH and TEXT that import again as fills.

use super::underlay_objects::ask;
use super::*;

/// A closed polyline on layer `walls` through `corners`, with handle `handle`.
fn walls(handle: &str, corners: &[[f64; 2]]) -> String {
    let points: String = corners
        .iter()
        .map(|[x, y]| format!("10\n{x}\n20\n{y}\n"))
        .collect();
    format!(
        "0\nLWPOLYLINE\n5\n{handle}\n100\nAcDbEntity\n8\nwalls\n100\nAcDbPolyline\n90\n{}\n70\n1\n{points}",
        corners.len()
    )
}

/// Room A 4 × 3 m with a column 0.5 × 0.5 m in it (11.75 m²) and room B 3 × 2.5 m
/// (7.5 m²), and the label of A on layer `areas`, in a drawing of version `version`.
fn plan(version: Option<&str>) -> String {
    let version = version.map_or(String::new(), |version| {
        format!("9\n$ACADVER\n1\n{version}\n9\n$HANDSEED\n5\nFFFF\n")
    });
    format!(
        "0\nSECTION\n2\nHEADER\n{version}9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
         {}{}{}\
         0\nTEXT\n5\n40\n100\nAcDbEntity\n8\nareas\n100\nAcDbText\n10\n300\n20\n2600\n30\n0\n40\n100\n1\nKitchen\n100\nAcDbText\n\
         0\nENDSEC\n0\nEOF\n",
        walls(
            "10",
            &[[0.0, 0.0], [4000.0, 0.0], [4000.0, 3000.0], [0.0, 3000.0]]
        ),
        walls(
            "20",
            &[
                [1000.0, 1000.0],
                [1500.0, 1000.0],
                [1500.0, 1500.0],
                [1000.0, 1500.0]
            ]
        ),
        walls(
            "30",
            &[
                [5000.0, 0.0],
                [8000.0, 0.0],
                [8000.0, 2500.0],
                [5000.0, 2500.0]
            ]
        ),
    )
}

fn import(path: &std::path::Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
        revision_of: None,
        code_page: None,
    }
}

/// The plan of `version` written into `directory` and imported, and how far the import
/// moved it.
fn imported(
    directory: &std::path::Path,
    version: Option<&str>,
) -> (KetchupApp, LiveBridge, [f64; 2]) {
    let source = directory.join("plan.dxf");
    std::fs::write(&source, plan(version)).unwrap();
    let (_, mut bridge) = setup();
    let mut app = KetchupApp::with_catalog_and_initial_box(crate::LocaleCatalog::english(), false);
    app.selection.clear();
    let answer = bridge.execute(&mut app, import(&source), false).unwrap();
    let offset = serde_json::from_value(answer["underlay"]["offset_mm"].clone()).unwrap();
    (app, bridge, offset)
}

/// The areas of the hatches of underlay `id`, in m², smallest first.
fn hatch_areas(bridge: &mut LiveBridge, app: &mut KetchupApp, id: u64) -> Vec<f64> {
    let found = bridge
        .execute(
            app,
            ask(
                "underlay_objects",
                json!({"underlay_id": id, "object_kind": "HATCH"}),
            ),
            false,
        )
        .unwrap();
    let mut areas = found["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["area_m2"].as_f64().unwrap())
        .collect::<Vec<_>>();
    areas.sort_by(f64::total_cmp);
    areas
}

#[test]
fn a_client_writes_the_areas_of_rooms_into_the_drawing_and_exports_them_as_hatch_and_text() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, mut bridge, offset) = imported(directory.path(), Some("AC1015"));
    let at = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];
    let digest = app.canonical_digest();
    let undo = app.undo_step_count();
    let rooms = json!([
        {"at_mm": at([500.0, 2500.0]), "name": "1.01 Kuchyňa"},
        {"at_mm": at([6000.0, 1000.0]), "name": " 1.02 Izba "},
    ]);

    let answer = bridge
        .execute(
            &mut app,
            ask(
                "drawing_rooms",
                json!({"layer": "areas", "rooms": rooms, "language": "sk", "text_height_mm": 100}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(answer["rooms"][0]["area_m2"], 11.75, "{answer}");
    assert_eq!(answer["rooms"][0]["holes"], 1, "the column: {answer}");
    assert_eq!(answer["rooms"][1]["area_m2"], 7.5, "{answer}");
    assert_eq!(answer["rooms"][1]["name"], "1.02 Izba", "{answer}");
    assert_eq!(answer["total_m2"], 19.25, "{answer}");
    assert_eq!(answer["language"], "sk", "{answer}");
    // Per room a hatch and two texts; the table its frame, 3 rules across, 2 down and the
    // texts of its header (3), rooms (2 × 3) and total (2).
    assert_eq!(answer["drawn"], 6 + 6 + 11, "{answer}");
    assert_eq!(answer["table"]["object_ids"].as_array().unwrap().len(), 17);
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    assert_eq!(hatch_areas(&mut bridge, &mut app, 1), [7.5, 11.75]);
    let texts = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"text": "m²"})),
            false,
        )
        .unwrap();
    let texts = texts.to_string();
    for expected in ["11,75 m²", "7,50 m²", "Plocha [m²]"] {
        assert!(texts.contains(expected), "{expected} in {texts}");
    }
    let total = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"text": "Spolu"})),
            false,
        )
        .unwrap();
    assert_eq!(total["matched"], 1, "{total}");

    // Exported, the drawing carries them as HATCH and TEXT, and imports again with them.
    let output = directory.path().join("areas.dxf");
    bridge
        .execute(
            &mut app,
            ask(
                "drawing_export",
                json!({"path": output.display().to_string()}),
            ),
            false,
        )
        .unwrap();
    let written = std::fs::read_to_string(&output).unwrap();
    for expected in [
        "AcDbHatch",
        "SOLID",
        "11,75 m",
        "19,25",
        "Spolu",
        "1.02 Izba",
    ] {
        assert!(
            written.contains(expected),
            "{expected} in the exported drawing"
        );
    }
    bridge.execute(&mut app, import(&output), false).unwrap();
    assert_eq!(hatch_areas(&mut bridge, &mut app, 2), [7.5, 11.75]);

    // Refused, naming the room, and nothing is drawn.
    let undo = app.undo_step_count();
    let room = |at_mm: [f64; 2], name: &str| json!({"at_mm": at_mm, "name": name});
    for (rooms, target) in [
        (
            json!([
                room(at([500.0, 2500.0]), "A"),
                room(at([3500.0, 500.0]), "B")
            ]),
            "rooms[1]",
        ),
        (json!([room(at([-9000.0, -9000.0]), "A")]), "rooms[0]"),
        (json!([room(at([500.0, 2500.0]), "  ")]), "rooms[0]"),
        (json!([]), "rooms"),
    ] {
        let arguments = json!({"underlay_id": 1, "layer": "areas", "rooms": rooms});
        assert_eq!(
            bridge
                .execute(&mut app, ask("drawing_rooms", arguments.clone()), false)
                .unwrap_err(),
            "invalid_params",
            "{arguments}"
        );
        let details = take_error_details().unwrap();
        assert_eq!(details["target"], target, "{details}");
    }
    let unknown =
        json!({"underlay_id": 1, "layer": "nowhere", "rooms": [room(at([500.0, 2500.0]), "A")]});
    assert_eq!(
        bridge
            .execute(&mut app, ask("drawing_rooms", unknown), false)
            .unwrap_err(),
        "invalid_params"
    );
    assert_eq!(app.undo_step_count(), undo);

    // Undo takes them all out again: the second import, then the rooms.
    for _ in 0..2 {
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
    }
    assert_eq!(app.canonical_digest(), digest);
}

/// A DXF 12 drawing has no HATCH: the export says so rather than leaving the fills out.
#[test]
fn rooms_drawn_into_a_dxf_12_drawing_refuse_its_export() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, mut bridge, offset) = imported(directory.path(), None);
    let at = [500.0 - offset[0], 2500.0 - offset[1]];
    bridge
        .execute(
            &mut app,
            ask(
                "drawing_rooms",
                json!({"layer": "areas", "rooms": [{"at_mm": at, "name": "Kitchen"}]}),
            ),
            false,
        )
        .unwrap();
    let output = directory.path().join("areas.dxf");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_export",
                    json!({"path": output.display().to_string()})
                ),
                false,
            )
            .unwrap_err(),
        "drawing_edit_not_writable"
    );
    let details = take_error_details().unwrap();
    assert!(details.to_string().contains("HATCH"), "{details}");
    assert!(!output.exists());
}
