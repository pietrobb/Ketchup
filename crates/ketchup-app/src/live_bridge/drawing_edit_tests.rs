//! `drawing action=remove`: a client takes objects out of an imported drawing by the ids
//! `underlay_objects` gave or by a box, one Undo step; the others keep their ids, an object placed by an
//! insert is refused with the insert to remove instead, and Save and Open keep the edit.

use super::underlay_objects::{DRAWING, ask, imported};
use super::*;

fn top_level(bridge: &mut LiveBridge, app: &mut KetchupApp) -> Vec<(u64, String)> {
    let answer = bridge
        .execute(app, ask("underlay_objects", json!({"limit": 100})), false)
        .unwrap();
    answer["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_u64().unwrap(),
                row["handle"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn id_of(objects: &[(u64, String)], handle: &str) -> u64 {
    objects.iter().find(|(_, h)| h == handle).unwrap().0
}

#[test]
fn a_client_removes_objects_of_a_drawing_in_one_undo_step_that_save_and_open_keep() {
    let (mut app, mut bridge, offset) = imported(DRAWING);
    let before = top_level(&mut bridge, &mut app);
    assert_eq!(before.len(), 5, "{before:?}");
    let door = id_of(&before, "1F");
    let wall = id_of(&before, "2A");
    let digest = app.canonical_digest();
    let undo = app.undo_step_count();

    // A line the door's insert placed is refused, naming the insert.
    let child = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"object_id": door})),
            false,
        )
        .unwrap()["object"]["child_ids"][0]
        .clone();
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"object_ids": [child]})),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    let details = take_error_details().unwrap();
    assert_eq!(details["details"]["remove_instead"], door, "{details}");
    assert_eq!(app.canonical_digest(), digest);

    let removed = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"object_ids": [door, wall, door]})),
            false,
        )
        .unwrap();
    assert_eq!(removed["removed"], 2, "{removed}");
    assert_eq!(removed["objects_left"], 6);
    assert_eq!(app.undo_step_count(), undo + 1);
    let after = top_level(&mut bridge, &mut app);
    assert_eq!(after.len(), 3, "{after:?}");
    // The others keep their ids.
    for (id, handle) in &after {
        assert_eq!(id_of(&before, handle), *id);
    }
    let summary = bridge
        .execute(&mut app, ask("underlay_objects", json!({})), false)
        .unwrap();
    assert_eq!(summary["kinds"]["INSERT"], json!([1, 1]), "{summary}");
    assert!(summary["layers"].get("walls").is_none(), "{summary}");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"object_ids": [wall]})),
                false
            )
            .unwrap_err(),
        "entity_not_found"
    );

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(top_level(&mut bridge, &mut app), before);
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(top_level(&mut bridge, &mut app), after);

    // By a box: what meets it goes, here the second door.
    let near = [5000.0 - offset[0], -offset[1]];
    let around = |reach: f64| {
        json!([
            [near[0] - reach, near[1] - reach],
            [near[0] + reach, near[1] + reach]
        ])
    };
    let boxed = bridge
        .execute(
            &mut app,
            ask("drawing_remove", json!({"within_mm": around(10.0)})),
            false,
        )
        .unwrap();
    assert_eq!(
        boxed["object_ids"],
        json!([id_of(&before, "20")]),
        "{boxed}"
    );
    let after = top_level(&mut bridge, &mut app);
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_remove", json!({"within_mm": around(10.0)})),
                false
            )
            .unwrap_err(),
        "entity_not_found",
        "nothing is left there"
    );
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_remove",
                    json!({"object_ids": [after[0].0], "within_mm": around(1.0)})
                ),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    let edited = app.canonical_digest();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.ketchup");
    bridge
        .execute(
            &mut app,
            Request::SaveAs {
                expected: None,
                path: path.display().to_string(),
            },
            false,
        )
        .unwrap();
    drop(bridge);
    drop(app);
    let (mut opened, mut bridge) = setup();
    bridge
        .execute(
            &mut opened,
            Request::Open {
                expected: None,
                path: path.display().to_string(),
            },
            false,
        )
        .unwrap();
    assert_eq!(opened.canonical_digest(), edited);
    assert_eq!(top_level(&mut bridge, &mut opened), after);
}

fn row(bridge: &mut LiveBridge, app: &mut KetchupApp, id: &Value) -> Value {
    bridge
        .execute(
            app,
            ask("underlay_objects", json!({"object_id": id})),
            false,
        )
        .unwrap()["object"]
        .clone()
}

fn close(value: &Value, want: [[f64; 2]; 2]) -> bool {
    let got: [[f64; 2]; 2] = serde_json::from_value(value.clone()).unwrap();
    got.iter()
        .flatten()
        .zip(want.iter().flatten())
        .all(|(got, want)| (got - want).abs() < 1e-6)
}

#[test]
fn a_client_draws_moves_and_copies_objects_of_a_drawing_in_document_millimetres() {
    let (mut app, mut bridge, _) = imported(DRAWING);
    let before = top_level(&mut bridge, &mut app);
    let undo = app.undo_step_count();

    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_draw",
                    json!({"layer": "roof", "shapes": [{"shape": "line", "points_mm": [[0, 0], [1, 0]]}]})
                ),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    assert_eq!(take_error_details().unwrap()["details"]["layer"], "roof");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_draw",
                    json!({"layer": "walls", "shapes": [{"shape": "circle", "center_mm": [0, 0], "radius_mm": -1}]})
                ),
                false
            )
            .unwrap_err(),
        "invalid_params"
    );
    assert_eq!(take_error_details().unwrap()["details"]["shape_index"], 0);

    let drawn = bridge
        .execute(
            &mut app,
            ask(
                "drawing_draw",
                json!({"layer": "walls", "shapes": [
                    {"shape": "line", "points_mm": [[100, 1000], [600, 1000]]},
                    {"shape": "text", "anchor_mm": [100, 1200], "height_mm": 50, "text": "Spálňa"},
                ]}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(app.undo_step_count(), undo + 1);
    let [line, text] = [
        drawn["object_ids"][0].clone(),
        drawn["object_ids"][1].clone(),
    ];
    let line_row = row(&mut bridge, &mut app, &line);
    assert_eq!(line_row["kind"], "LINE", "{line_row}");
    assert_eq!(line_row["layer"], "walls");
    assert!(
        close(&line_row["bounds_mm"], [[100.0, 1000.0], [600.0, 1000.0]]),
        "{line_row}"
    );
    let text_row = row(&mut bridge, &mut app, &text);
    assert_eq!(text_row["texts"][0]["text"], "Spálňa", "{text_row}");
    assert!(close(
        &json!([text_row["texts"][0]["anchor_mm"], [0, 0]]),
        [[100.0, 1200.0], [0.0, 0.0]]
    ));
    assert_eq!(top_level(&mut bridge, &mut app).len(), before.len() + 2);

    // Turned a quarter about its start and moved up: it stands from (100, 1100) to (100, 1600).
    let moved = bridge
        .execute(
            &mut app,
            ask(
                "drawing_move",
                json!({"object_ids": [line], "rotate_deg": 90, "about_mm": [100, 1000], "by_mm": [0, 100]}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(moved["copy"], false, "{moved}");
    assert!(close(
        &row(&mut bridge, &mut app, &line)["bounds_mm"],
        [[100.0, 1100.0], [100.0, 1600.0]]
    ));

    // A mirrored copy of the wall line, and of a door: its block is placed again.
    let wall = id_of(&before, "2A");
    let door = id_of(&before, "1F");
    let copied = bridge
        .execute(
            &mut app,
            ask(
                "drawing_move",
                json!({"object_ids": [wall, door], "mirror_line_mm": [[0, 0], [1, 0]], "copy": true}),
            ),
            false,
        )
        .unwrap();
    let new = copied["new_object_ids"].as_array().unwrap().clone();
    assert_eq!(new.len(), 2, "{copied}");
    let wall_row = row(&mut bridge, &mut app, &json!(wall));
    // Copies come in the order of the ids, the door's first.
    let door_copy = row(&mut bridge, &mut app, &new[0]);
    assert_eq!(door_copy["block"], "DOOR_80", "{door_copy}");
    assert_eq!(door_copy["mirrored"], true);
    let copy_row = row(&mut bridge, &mut app, &new[1]);
    let [[x0, y0], [x1, y1]]: [[f64; 2]; 2] =
        serde_json::from_value(wall_row["bounds_mm"].clone()).unwrap();
    assert!(
        close(&copy_row["bounds_mm"], [[x0, -y1], [x1, -y0]]),
        "{copy_row}"
    );
    let summary = bridge
        .execute(&mut app, ask("underlay_objects", json!({})), false)
        .unwrap();
    assert_eq!(summary["kinds"]["INSERT"], json!([3, 3]), "{summary}");
    assert_eq!(app.undo_step_count(), undo + 3);

    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("drawing_move", json!({"object_ids": [wall]})),
                false
            )
            .unwrap_err(),
        "invalid_params",
        "a move needs a map"
    );
    for _ in 0..3 {
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
    }
    assert_eq!(top_level(&mut bridge, &mut app), before);
}

/// 2D-2: `drawing action=trim/extend/offset` on straight lines, in document millimetres:
/// trim cuts back to the crossing line, extend reaches on to it, offset adds a parallel
/// line; one Undo step each, and anything but a straight line is refused with its id.
#[test]
fn a_client_trims_extends_and_offsets_lines_of_a_drawing() {
    let (mut app, mut bridge, _) = imported(DRAWING);
    let before = top_level(&mut bridge, &mut app);
    let undo = app.undo_step_count();
    let drawn = bridge
        .execute(
            &mut app,
            ask(
                "drawing_draw",
                json!({"layer": "walls", "shapes": [
                    {"shape": "line", "points_mm": [[5000, 5000], [6000, 5000]]},
                    {"shape": "line", "points_mm": [[5500, 4500], [5500, 5500]]},
                    {"shape": "line", "points_mm": [[5100, 5300], [5200, 5300]]},
                ]}),
            ),
            false,
        )
        .unwrap();
    let [long, cross, short] = [0, 1, 2].map(|index| drawn["object_ids"][index].clone());
    let mut edit = |app: &mut KetchupApp, method: &str, args: Value| {
        bridge.execute(app, ask(method, args), false)
    };
    let one_new = |answer: &Value| {
        let new = answer["new_object_ids"].as_array().unwrap();
        assert_eq!(new.len(), 1, "{answer}");
        new[0].clone()
    };

    let trimmed = edit(
        &mut app,
        "drawing_trim",
        json!({"object_id": long, "at_mm": [5800, 5000]}),
    )
    .unwrap();
    assert_eq!(trimmed["removed"], true, "{trimmed}");
    let left = one_new(&trimmed);
    let extended = edit(
        &mut app,
        "drawing_extend",
        json!({"object_id": short, "at_mm": [5190, 5301]}),
    )
    .unwrap();
    let reached = one_new(&extended);
    let offset = edit(
        &mut app,
        "drawing_offset",
        json!({"object_id": cross, "at_mm": [5900, 5000], "distance_mm": 100}),
    )
    .unwrap();
    assert_eq!(offset["removed"], false, "{offset}");
    let parallel = one_new(&offset);
    let door = id_of(&before, "1F");
    assert_eq!(
        edit(
            &mut app,
            "drawing_trim",
            json!({"object_id": door, "at_mm": [0, 0]})
        )
        .unwrap_err(),
        "invalid_params"
    );
    assert_eq!(take_error_details().unwrap()["details"]["object_id"], door);
    assert_eq!(app.undo_step_count(), undo + 4);

    for (id, want) in [
        (&left, [[5000.0, 5000.0], [5500.0, 5000.0]]),
        (&reached, [[5100.0, 5300.0], [5500.0, 5300.0]]),
        (&parallel, [[5600.0, 4500.0], [5600.0, 5500.0]]),
    ] {
        let found = row(&mut bridge, &mut app, id);
        assert_eq!(found["kind"], "LINE", "{found}");
        assert_eq!(found["layer"], "walls");
        assert!(close(&found["bounds_mm"], want), "{found}");
    }

    for _ in 0..4 {
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
    }
    assert_eq!(top_level(&mut bridge, &mut app), before);
}

/// `drawing action=export`: a drawing with a door taken out, its wall moved and a line drawn
/// is written as DXF; the file imported again holds the objects of each kind and layer the
/// edited drawing holds, the file it was imported from is unchanged, and a source that is
/// not the drawing is refused.
#[test]
fn a_client_exports_the_edited_drawing_as_a_dxf_that_imports_as_edited() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dxf");
    std::fs::write(&source, DRAWING).unwrap();
    let (_, mut bridge) = setup();
    let mut app = KetchupApp::with_catalog_and_initial_box(crate::LocaleCatalog::english(), false);
    app.selection.clear();
    let import = |path: &std::path::Path| Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
    };
    bridge.execute(&mut app, import(&source), false).unwrap();
    let objects = top_level(&mut bridge, &mut app);
    let door = id_of(&objects, "1F");
    let wall = id_of(&objects, "2A");
    for (method, arguments) in [
        ("drawing_remove", json!({"object_ids": [door]})),
        (
            "drawing_move",
            json!({"object_ids": [wall], "by_mm": [0, 300]}),
        ),
        (
            "drawing_draw",
            json!({"layer": "walls", "shapes": [{"shape": "line", "points_mm": [[0, 1000], [3000, 1000]]}]}),
        ),
    ] {
        bridge
            .execute(&mut app, ask(method, arguments), false)
            .unwrap();
    }
    let edited = bridge
        .execute(&mut app, ask("underlay_objects", json!({})), false)
        .unwrap();
    let digest = app.canonical_digest();

    let output = directory.path().join("edited.dxf");
    let answer = bridge
        .execute(
            &mut app,
            ask(
                "drawing_export",
                json!({"path": output.display().to_string()}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(
        answer["changes"],
        json!({"removed": 1, "moved": 1, "added": 1}),
        "{answer}"
    );
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), DRAWING);
    let written = std::fs::read_to_string(&output).unwrap();
    // The block the removed door placed stays, as the other door still places it.
    assert!(written.contains("DOOR_80") && written.contains("1.01 Kitchen"));

    bridge.execute(&mut app, import(&output), false).unwrap();
    let again = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"underlay_id": 2})),
            false,
        )
        .unwrap();
    assert_eq!(again["kinds"], edited["kinds"], "{again}\n{edited}");
    assert_eq!(again["layers"], edited["layers"], "{again}\n{edited}");

    // To a .dwg path the same drawing is written as DWG.
    let dwg = directory.path().join("edited.dwg");
    bridge
        .execute(
            &mut app,
            ask(
                "drawing_export",
                json!({"underlay_id": 1, "path": dwg.display().to_string()}),
            ),
            false,
        )
        .unwrap();
    bridge.execute(&mut app, import(&dwg), false).unwrap();
    let from_dwg = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"underlay_id": 3})),
            false,
        )
        .unwrap();
    assert_eq!(from_dwg["kinds"], edited["kinds"], "{from_dwg}\n{edited}");

    // Another drawing named as the source is refused, and nothing is written.
    let other = directory.path().join("other.dxf");
    std::fs::write(&other, DRAWING.replace("2630", "2640")).unwrap();
    let refused = directory.path().join("refused.dxf");
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask(
                    "drawing_export",
                    json!({"underlay_id": 1, "path": refused.display().to_string(), "source": other.display().to_string()})
                ),
                false
            )
            .unwrap_err(),
        "export_failed"
    );
    assert!(!refused.exists());
}

/// `drawing action=extrude`: the walls around a room are pulled up with the room cut out,
/// and the room's hatch by its id, each one Undo step written into the program bound to the
/// drawing; a point in nothing closed, both selectors or no height change nothing.
#[test]
fn a_client_pulls_closed_areas_of_a_drawing_up_into_solids() {
    let (mut app, mut bridge, offset) = imported(&super::underlay_objects::walled_room());
    let digest = app.canonical_digest();
    let undo = app.undo_step_count();
    let in_wall = [150.0 - offset[0], 3_000.0 - offset[1]];

    let walls = bridge
        .execute(
            &mut app,
            ask(
                "drawing_extrude",
                json!({"at_mm": in_wall, "height_mm": 2600}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(walls["name"], "AREA_1", "{walls}");
    assert_eq!(walls["bound_to_drawing"], true);
    assert_eq!(app.undo_step_count(), undo + 1);
    let hatch = bridge
        .execute(
            &mut app,
            ask("underlay_objects", json!({"object_kind": "HATCH"})),
            false,
        )
        .unwrap()["objects"][0]["id"]
        .clone();
    let floor = bridge
        .execute(
            &mut app,
            ask(
                "drawing_extrude",
                json!({"object_id": hatch, "height_mm": 30}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(floor["name"], "AREA_2", "{floor}");
    assert_eq!(app.undo_step_count(), undo + 2);
    let source = app.document.current_rule_program().unwrap().source.clone();
    for expected in [
        "AREA_1_SOLID = extrude(\"AREA_1\", profile = AREA_1, distance = 2600)",
        "AREA_2_SOLID = extrude(\"AREA_2\", profile = AREA_2, distance = 30)",
    ] {
        assert!(source.contains(expected), "{expected} in {source}");
    }
    let walls_data = &source[source.find("AREA_1_HOLES = [").unwrap()..];
    assert!(
        walls_data.starts_with("AREA_1_HOLES = [\n    [\n"),
        "the room is a hole: {source}"
    );

    for (arguments, field) in [
        (
            json!({"at_mm": [-50_000, -50_000], "height_mm": 100}),
            "at_mm",
        ),
        (
            json!({"at_mm": in_wall, "object_id": hatch, "height_mm": 100}),
            "at_mm",
        ),
        (json!({"at_mm": in_wall, "height_mm": 0}), "height_mm"),
    ] {
        assert_eq!(
            bridge
                .execute(&mut app, ask("drawing_extrude", arguments.clone()), false)
                .unwrap_err(),
            "invalid_params",
            "{arguments}"
        );
        let details = take_error_details().unwrap();
        assert_eq!(details["target"], field, "{details}");
    }
    assert_eq!(app.undo_step_count(), undo + 2);

    for _ in 0..2 {
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
    }
    assert_eq!(app.canonical_digest(), digest);
}
