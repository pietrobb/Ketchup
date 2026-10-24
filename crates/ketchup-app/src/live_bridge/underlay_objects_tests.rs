//! `inspect action=underlay_objects`: an imported drawing answers as objects, an insert
//! with its block, placement and attributes, a dimension with its measurement, a hatch
//! with its area, by kind, block, text and box, in pages, and changes nothing.

use super::*;

const DRAWING: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nBLOCKS\n",
    "0\nBLOCK\n8\n0\n2\nDOOR_80\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n5\nB2\n8\n0\n10\n0\n20\n0\n11\n800\n21\n0\n",
    "0\nENDBLK\n",
    "0\nBLOCK\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
    "0\nLINE\n5\nD1\n8\n0\n10\n0\n20\n500\n11\n2630\n21\n500\n",
    "0\nTEXT\n5\nD2\n8\n0\n10\n1300\n20\n510\n40\n100\n1\n2630\n",
    "0\nENDBLK\n",
    "0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nINSERT\n5\n1F\n8\ndoors\n2\nDOOR_80\n10\n1000\n20\n0\n50\n90\n66\n1\n",
    "0\nATTRIB\n8\ndoors\n2\nNAME\n1\nD-01\n10\n1000\n20\n0\n40\n50\n0\nSEQEND\n",
    "0\nINSERT\n5\n20\n8\ndoors\n2\nDOOR_80\n10\n5000\n20\n0\n",
    "0\nLINE\n5\n2A\n8\nwalls\n10\n0\n20\n0\n11\n500\n21\n0\n",
    "0\nDIMENSION\n5\n3B\n8\ndims\n2\n*D1\n10\n0\n20\n500\n13\n0\n23\n0\n",
    "14\n2630\n24\n0\n42\n2630\n70\n32\n",
    "0\nMTEXT\n5\n4D\n8\nrooms\n10\n200\n20\n200\n40\n100\n1\n1.01 Kitchen 12,5 m2\n",
    "0\nENDSEC\n0\nEOF\n",
);

fn objects(arguments: Value) -> Request {
    ask("underlay_objects", arguments)
}

fn ask(method: &str, arguments: Value) -> Request {
    let mut request = json!({ "method": method });
    request
        .as_object_mut()
        .unwrap()
        .extend(arguments.as_object().unwrap().clone());
    serde_json::from_value(request).unwrap()
}

/// `source` imported as an underlay, and how far the import moved it.
fn imported(source: &str) -> (KetchupApp, LiveBridge, [f64; 2]) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    std::fs::write(&path, source).unwrap();
    let (mut app, mut bridge) = setup();
    let import = Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
    };
    let answer = bridge
        .execute(&mut app, import, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let offset = serde_json::from_value(answer["underlay"]["offset_mm"].clone()).unwrap();
    (app, bridge, offset)
}

#[test]
fn a_client_reads_an_underlay_as_blocks_dimensions_and_texts_not_lines() {
    let (mut app, mut bridge, offset) = imported(DRAWING);
    let digest = app.canonical_digest();
    let to_document = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];

    let summary = bridge.execute(&mut app, objects(json!({})), false).unwrap();
    assert_eq!(summary["kinds"]["INSERT"], json!([2, 2]), "{summary}");
    // The wall, the line each door places, and the dimension's.
    assert_eq!(summary["kinds"]["LINE"], json!([1, 4]));
    assert_eq!(
        summary["blocks"],
        json!([{"block": "DOOR_80", "top_level": 2, "all": 2}])
    );
    assert_eq!(summary["layers"]["walls"], 1);
    // The top level by default: two doors, the wall, the dimension and the room text.
    assert_eq!(summary["matched"], 5);

    let doors = bridge
        .execute(&mut app, objects(json!({"block": "door"})), false)
        .unwrap();
    let rows = doors["objects"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["handle"], "1F");
    assert_eq!(
        rows[0]["attributes"],
        json!([{"tag": "NAME", "value": "D-01"}])
    );
    assert_eq!(rows[0]["children"], 1);
    // One page gives every placement of the block.
    assert_eq!(rows[1]["insertion_mm"], json!(to_document([5000.0, 0.0])));
    assert_eq!(rows[1]["rotation_deg"], 0.0);

    let door = bridge
        .execute(
            &mut app,
            objects(json!({"object_id": rows[0]["id"]})),
            false,
        )
        .unwrap();
    let door = &door["object"];
    assert_eq!(
        door["insertion_mm"],
        json!(to_document([1000.0, 0.0])),
        "{door}"
    );
    assert_eq!(door["drawing_insertion_mm"], json!([1000.0, 0.0]));
    assert!((door["rotation_deg"].as_f64().unwrap() - 90.0).abs() < 1e-9);
    assert_eq!(door["mirrored"], false);
    let leaf = bridge
        .execute(
            &mut app,
            objects(json!({"object_id": door["child_ids"][0]})),
            false,
        )
        .unwrap();
    assert_eq!(leaf["object"]["kind"], "LINE");
    assert!((leaf["object"]["length_mm"].as_f64().unwrap() - 800.0).abs() < 1e-6);

    let dimension = bridge
        .execute(
            &mut app,
            objects(json!({"object_kind": "dimension"})),
            false,
        )
        .unwrap();
    assert_eq!(
        dimension["objects"][0]["measurement"], 2630.0,
        "{dimension}"
    );
    assert_eq!(dimension["objects"][0]["text"], "2630");
    assert!(
        dimension.get("kinds").is_none(),
        "the counts come once: {dimension}"
    );
    // The dimension measures from the wall's end: each names the other among what it
    // touches; the doors stand apart.
    let wall = bridge
        .execute(&mut app, objects(json!({"layer": "walls"})), false)
        .unwrap()["objects"][0]["id"]
        .clone();
    let touching = |id: &Value, bridge: &mut LiveBridge, app: &mut KetchupApp| {
        bridge
            .execute(app, objects(json!({"object_id": id})), false)
            .unwrap()["object"]["touching"]
            .clone()
    };
    let dimension_id = dimension["objects"][0]["id"].clone();
    assert_eq!(
        touching(&dimension_id, &mut bridge, &mut app),
        json!([wall])
    );
    assert_eq!(
        touching(&wall, &mut bridge, &mut app),
        json!([dimension_id])
    );
    assert_eq!(touching(&rows[0]["id"], &mut bridge, &mut app), json!([]));

    let room = bridge
        .execute(&mut app, objects(json!({"text": "kitchen"})), false)
        .unwrap();
    assert_eq!(room["matched"], 1);
    assert_eq!(room["objects"][0]["kind"], "MTEXT");

    // Only what meets a box around the second door.
    let near = to_document([5000.0, 0.0]);
    let boxed = bridge
        .execute(
            &mut app,
            objects(json!({"within_mm": [[near[0] - 10.0, near[1] - 10.0], [near[0] + 10.0, near[1] + 10.0]]})),
            false,
        )
        .unwrap();
    assert_eq!(boxed["matched"], 1, "{boxed}");
    assert_eq!(boxed["objects"][0]["handle"], "20");

    let page = bridge
        .execute(&mut app, objects(json!({"limit": 2})), false)
        .unwrap();
    assert_eq!(page["objects"].as_array().unwrap().len(), 2);
    assert_eq!(page["next_offset"], 2);
    let last = bridge
        .execute(&mut app, objects(json!({"limit": 2, "offset": 4})), false)
        .unwrap();
    assert_eq!(last["next_offset"], Value::Null);

    assert_eq!(
        bridge
            .execute(&mut app, objects(json!({"object_id": 999})), false)
            .unwrap_err(),
        "entity_not_found"
    );
    assert_eq!(
        bridge
            .execute(&mut app, objects(json!({"limit": 0})), false)
            .unwrap_err(),
        "invalid_params"
    );
    assert_eq!(app.canonical_digest(), digest, "a question changes nothing");
}

/// A closed polyline on `layer` through `corners`.
fn outline(layer: &str, corners: &[[f64; 2]]) -> String {
    let points: String = corners
        .iter()
        .map(|[x, y]| format!("10\n{x}\n20\n{y}\n"))
        .collect();
    format!(
        "0\nLWPOLYLINE\n8\n{layer}\n90\n{}\n70\n1\n{points}",
        corners.len()
    )
}

/// A room 9.4 × 5.4 m inside walls 300 mm thick, drawn as two outlines, with its name in
/// it, a hatch over it and a loose furniture line.
fn walled_room() -> String {
    let rectangle = |low: f64, high_x: f64, high_y: f64| {
        [[low, low], [high_x, low], [high_x, high_y], [low, high_y]]
    };
    let hatch = concat!(
        "0\nHATCH\n8\nwalls\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n",
        "92\n2\n72\n0\n73\n1\n93\n4\n10\n300\n20\n300\n10\n9700\n20\n300\n",
        "10\n9700\n20\n5700\n10\n300\n20\n5700\n97\n0\n75\n0\n76\n1\n98\n0\n",
    );
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
         {}{}{hatch}\
         0\nLINE\n8\nwalls\n10\n1000\n20\n1000\n11\n1500\n21\n1200\n\
         0\nMTEXT\n8\nrooms\n10\n2000\n20\n2000\n40\n100\n1\nKitchen\n\
         0\nENDSEC\n0\nEOF\n",
        outline("walls", &rectangle(0.0, 10_000.0, 6_000.0)),
        outline("walls", &rectangle(300.0, 9_700.0, 5_700.0)),
    )
}

#[test]
fn a_client_finds_the_rooms_walls_enclose_with_their_names_and_wall_thickness() {
    let (mut app, mut bridge, offset) = imported(&walled_room());
    let digest = app.canonical_digest();
    let regions = bridge
        .execute(&mut app, ask("underlay_regions", json!({})), false)
        .unwrap();
    let rows = regions["regions"].as_array().unwrap();
    let areas = rows
        .iter()
        .map(|row| (row["area_m2"].as_f64().unwrap() * 100.0).round() / 100.0)
        .collect::<Vec<_>>();
    // The room, and the walls around it with the room as their hole.
    assert_eq!(areas, [50.76, 9.24], "{regions}");
    assert_eq!(rows[1]["holes"], 1);
    assert_eq!(rows[0]["texts"], json!(["Kitchen"]));
    assert!((rows[0]["perimeter_mm"].as_f64().unwrap() - 29_600.0).abs() < 1e-6);

    let inside = [2_000.0 - offset[0], 2_000.0 - offset[1]];
    let at = bridge
        .execute(
            &mut app,
            ask("underlay_regions", json!({"point_mm": inside})),
            false,
        )
        .unwrap();
    assert_eq!(at["matched"], 1, "{at}");
    let corners = at["regions"][0]["outline_mm"].as_array().unwrap();
    assert_eq!(corners.len(), 4);
    assert!(corners.contains(&json!([300.0 - offset[0], 300.0 - offset[1]])));

    let walls = bridge
        .execute(
            &mut app,
            ask(
                "underlay_parallel",
                json!({"layers": ["walls"], "max_gap_mm": 500}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(walls["gaps"][0]["gap_mm"], 300.0, "{walls}");
    assert_eq!(walls["gaps"][0]["pairs"], 4);
    // Each side of a pair names the object that draws it: the two outlines.
    let pair = &walls["pairs"][0];
    assert_ne!(pair["a"]["object_id"], pair["b"]["object_id"], "{pair}");
    assert!(pair["a"]["object_id"].is_u64());
    assert_eq!(
        bridge
            .execute(
                &mut app,
                ask("underlay_regions", json!({"layers": ["nothing"]})),
                false
            )
            .unwrap_err(),
        "entity_not_found"
    );
    assert_eq!(app.canonical_digest(), digest, "a question changes nothing");
}
