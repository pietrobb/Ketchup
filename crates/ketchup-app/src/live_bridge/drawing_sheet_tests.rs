//! `file action=export_drawings` with `sheet_id`: a drawing sheet of the
//! document is written as the DXF of its exact projection, the project
//! drawings as a DXF per sheet, and no export changes the document. Exact
//! circles the views face are CIRCLE and ARC entities, not their lines. A
//! `.dwg` path gets that DXF as an AutoCAD 2018 DWG.

use super::drill_holes::{centroid, edit, face, saved};
use super::program_edit::evaluate_exact;
use super::*;
use ketchup_model::drawing::{DrawingSheetId, project_orthographic_drawing};
use ketchup_model::drawing_export::export_drawing;
use std::collections::BTreeMap;

fn export_drawings(path: &Path, extra: Value) -> Request {
    let mut request = json!({"method": "export_drawings", "path": path});
    request
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(request).unwrap()
}

fn summary(app: &mut KetchupApp, bridge: &mut LiveBridge) -> Value {
    let request = serde_json::from_value(json!({"method": "summary"})).unwrap();
    bridge.execute(app, request, false).unwrap()
}

/// The first part of the new document drawn on a sheet by MCP `create_drawing`,
/// found through the summary.
fn sheet_of_the_block(app: &mut KetchupApp, bridge: &mut LiveBridge) -> u64 {
    evaluate_exact(app);
    let root = app.document.current().occurrences().next().unwrap().id();
    let instance_paths = vec![AssistantInstancePath {
        root_occurrence_id: root.0,
        steps: Vec::new(),
    }];
    let name = "Block".to_owned();
    let created = edit(AssistantCadEditOperation::CreateDrawing {
        name,
        instance_paths,
    });
    let created = bridge.execute(app, created, false).unwrap();
    assert_eq!(created["published"], true, "{created}");
    evaluate_exact(app);
    let listed = summary(app, bridge);
    assert_eq!(listed["counts"]["drawing_sheets"], 1, "{listed}");
    assert_eq!(listed["drawing_sheets_complete"], true);
    let [sheet] = listed["drawing_sheets"].as_array().unwrap().as_slice() else {
        panic!("one drawing sheet: {listed}");
    };
    assert_eq!(sheet["name"]["text"], "Block");
    sheet["id"].as_u64().unwrap()
}

fn layer_lines(dxf: &str, layer: &str) -> usize {
    dxf.matches(&format!("0\nLINE\n8\n{layer}\n")).count()
}

#[test]
fn a_drawing_sheet_is_written_as_the_dxf_of_its_exact_projection() {
    let (mut app, mut bridge) = setup();
    assert_eq!(summary(&mut app, &mut bridge)["drawing_sheets"], json!([]));
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("block.dxf");

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");
    assert_eq!(exported["sheet_id"], id);
    assert_eq!(
        summary(&mut app, &mut bridge)["drawing_sheets"][0]["name"]["text"],
        exported["sheet"]
    );
    assert_eq!(exported["basis"], "drawing_sheet_exact_projection");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let expected = export_drawing(&snapshot, &drawing).unwrap();
    assert_eq!(
        written.as_bytes(),
        expected.dxf(),
        "the exact export, byte for byte"
    );
    assert_eq!(exported["bytes"], written.len());
    assert_eq!(
        exported["views"],
        json!(
            drawing
                .views
                .iter()
                .map(|view| view.kind.stable_name())
                .collect::<Vec<_>>()
        )
    );
    let visible = drawing
        .views
        .iter()
        .map(|view| view.visible_lines.len())
        .sum::<usize>();
    let hidden = drawing
        .views
        .iter()
        .map(|view| view.hidden_lines.len())
        .sum::<usize>();
    assert!(visible >= 12, "three views of a block: {visible}");
    assert_eq!(exported["visible_lines"], visible);
    assert_eq!(exported["hidden_lines"], hidden);
    assert_eq!(layer_lines(&written, "VISIBLE"), visible);
    assert_eq!(layer_lines(&written, "HIDDEN"), hidden);
    assert!(written.ends_with("0\nENDSEC\n0\nEOF\n"));

    assert_eq!(app.live_bridge_stamp(), stamp, "an export is no edit");
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(saved(&app), bytes);
}

#[test]
fn wrong_sheet_requests_are_refused_by_name() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);
    let existing = file("existing.dwg");
    std::fs::write(&existing, b"user file").unwrap();

    for (path, extra, code) in [
        (file("house.txt"), json!({}), "invalid_path"),
        (
            file("missing.dxf"),
            json!({"sheet_id": id + 1}),
            "drawing_sheet_not_found",
        ),
        (file("sheet.pdf"), json!({"sheet_id": id}), "invalid_path"),
        (file("sheet.txt"), json!({"sheet_id": id}), "invalid_path"),
        (
            file("sheet.dxf"),
            json!({"sheet_id": id, "format": "A1"}),
            "invalid_params",
        ),
        (
            file("sheet.dxf"),
            json!({"sheet_id": id, "title_block": {"author": "AI"}}),
            "invalid_params",
        ),
        (
            existing.clone(),
            json!({"sheet_id": id}),
            "drawings_target_exists",
        ),
    ] {
        let refused = bridge.execute(&mut app, export_drawings(&path, extra.clone()), false);
        assert_eq!(refused, Err(code), "{} {extra}", path.display());
        if path != existing {
            assert!(!path.exists(), "{} was written", path.display());
        }
    }
    assert_eq!(std::fs::read(&existing).unwrap(), b"user file");
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!(app.undo_step_count(), undo);
    assert_eq!(saved(&app), bytes);
}

/// The project drawings as DXF are a file per sheet beside the path,
/// numbered, each with the title block saying which sheet of how many it is;
/// as DWG the same sheets in AutoCAD 2018. A sheet file already there refuses
/// the set and leaves nothing of it behind.
#[test]
fn the_project_drawings_are_written_as_a_dxf_or_dwg_per_sheet() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let (stamp, undo) = (app.live_bridge_stamp(), app.undo_step_count());
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&file("house.dxf"), json!({})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["file_format"], "dxf", "{exported}");
    // A block is one storey: its plan, the sections and the elevations.
    assert_eq!(exported["sheet_count"], 3, "{exported}");
    for (index, shows) in ["plan", "sections", "elevations"].into_iter().enumerate() {
        let number = index + 1;
        let path = file(&format!("house-{number:02}.dxf"));
        let sheet = &exported["sheets"][index];
        assert_eq!(sheet["number"], number);
        assert_eq!(sheet["shows"]["kind"], shows, "{sheet}");
        assert_eq!(sheet["path"], path.display().to_string());
        assert_eq!(exported["files"][index], path.display().to_string());
        let dxf = std::fs::read_to_string(&path).unwrap();
        assert!(dxf.contains("$ACADVER\n1\nAC1009"));
        assert!(
            dxf.contains(&format!("\n1\n{number} / 3\n")),
            "sheet {number}"
        );
        assert!(
            dxf.contains("0\nLINE\n8\nPEN-"),
            "sheet {number} draws lines"
        );
    }
    assert!(!file("house.dxf").exists());

    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&file("house.dwg"), json!({})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(exported["sheet_count"], 3, "{exported}");
    for number in 1..=3 {
        let dwg = std::fs::read(file(&format!("house-{number:02}.dwg"))).unwrap();
        assert!(dwg.starts_with(b"AC1032"), "sheet {number}");
    }

    std::fs::write(file("taken-02.dxf"), b"user file").unwrap();
    assert_eq!(
        bridge.execute(
            &mut app,
            export_drawings(&file("taken.dxf"), json!({})),
            false
        ),
        Err("drawings_write_failed")
    );
    assert!(
        !file("taken-01.dxf").exists(),
        "the set is written whole or not at all"
    );
    assert!(!file("taken-03.dxf").exists());
    assert_eq!(std::fs::read(file("taken-02.dxf")).unwrap(), b"user file");
    assert_eq!(app.live_bridge_stamp(), stamp, "an export is no edit");
    assert_eq!(app.undo_step_count(), undo);
}

/// One DXF entity: its type, layer, numeric codes and KETCHUP id.
struct Entity {
    kind: String,
    layer: String,
    codes: BTreeMap<u16, f64>,
    id: String,
    text: String,
}

/// The entities of the ENTITIES section, read pair by pair (a code may be
/// padded with spaces, as other writers do).
fn entities(dxf: &str) -> Vec<Entity> {
    let lines = dxf.lines().collect::<Vec<_>>();
    let pairs = lines
        .chunks(2)
        .filter_map(|pair| match pair {
            [code, value] => Some((code.trim(), value.trim())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let start = pairs
        .iter()
        .position(|pair| *pair == ("2", "ENTITIES"))
        .unwrap()
        + 1;
    let mut entities = Vec::<Entity>::new();
    for &(code, value) in &pairs[start..] {
        if (code, value) == ("0", "ENDSEC") {
            break;
        }
        match code {
            "0" => entities.push(Entity {
                kind: (*value).to_owned(),
                layer: String::new(),
                codes: BTreeMap::new(),
                id: String::new(),
                text: String::new(),
            }),
            "8" => entities.last_mut().unwrap().layer = (*value).to_owned(),
            "1000" => entities.last_mut().unwrap().id = (*value).to_owned(),
            "1" => entities.last_mut().unwrap().text = (*value).to_owned(),
            code => {
                if let (Ok(code), Ok(number)) = (code.parse::<u16>(), value.parse::<f64>()) {
                    entities.last_mut().unwrap().codes.insert(code, number);
                }
            }
        }
    }
    entities
}

fn edges_of_kind(app: &mut KetchupApp, bridge: &mut LiveBridge, kind: &str) -> Vec<Value> {
    let query = json!({"kind": "edges", "limit": 100, "search": kind});
    let page = bridge
        .execute(
            app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(query).unwrap(),
            },
            false,
        )
        .unwrap();
    assert!(page["next_cursor"].is_null(), "{page}");
    let rows = page["items"].as_array().unwrap().clone();
    let last = rows
        .iter()
        .filter_map(|row| row["producer_feature_id"].as_u64())
        .max();
    rows.into_iter()
        .filter(|row| row["producer_feature_id"].as_u64() == last)
        .collect()
}

#[test]
fn a_drilled_hole_and_a_rounded_corner_are_an_exact_circle_and_arc_in_the_dxf() {
    const HOLE_MM: f64 = 6.0;
    const ROUND_MM: f64 = 5.0;
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let [x, y, z] = centroid(&top);
    let drilled = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::DrillHoles {
                definition_id: top["definition_id"].as_u64().unwrap(),
                name: "hole".to_owned(),
                target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
                face_reference_id: top["reference_id"].as_str().unwrap().to_owned(),
                points_local_mm: vec![[x, y, z]],
                diameter_mm: HOLE_MM,
                depth_mm: None,
                through: true,
                thread: None,
            }),
            false,
        )
        .unwrap();
    assert_eq!(drilled["published"], true, "{drilled}");
    evaluate_exact(&mut app);
    // The vertical edge at the corner of largest x + y.
    let corner = edges_of_kind(&mut app, &mut bridge, "line")
        .into_iter()
        .filter(|edge| {
            let bounds = &edge["geometry"]["bounds_mm"];
            let span =
                |axis: usize| bounds[1][axis].as_f64().unwrap() - bounds[0][axis].as_f64().unwrap();
            span(0).abs() < 1e-3 && span(1).abs() < 1e-3 && span(2) > 1.0
        })
        .max_by(|left, right| {
            let sum = |edge: &Value| {
                edge["geometry"]["centroid_mm"][0].as_f64().unwrap()
                    + edge["geometry"]["centroid_mm"][1].as_f64().unwrap()
            };
            sum(left).total_cmp(&sum(right))
        })
        .expect("a vertical edge");
    let rounded = bridge
        .execute(
            &mut app,
            edit(AssistantCadEditOperation::FilletEdges {
                definition_id: corner["definition_id"].as_u64().unwrap(),
                name: "round".to_owned(),
                target_feature_id: corner["producer_feature_id"].as_u64().unwrap(),
                edge_reference_ids: vec![corner["reference_id"].as_str().unwrap().to_owned()],
                radius_mm: ROUND_MM,
            }),
            false,
        )
        .unwrap();
    assert_eq!(rounded["published"], true, "{rounded}");
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rounded.dxf");
    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let index = drawing
        .views
        .iter()
        .position(|view| view.kind.stable_name() == "top")
        .unwrap();
    let (view, placement) = (
        &drawing.views[index],
        &drawing.layout.view_placements[index],
    );
    let scale = (placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0])
        / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
    let model = |page_x: f64, page_y: f64| {
        [
            (page_x - placement.origin_mm[0]) / scale,
            (page_y - placement.origin_mm[1]) / scale,
        ]
    };
    let prefix = format!("{}/", view.stable_view_id);
    let entities = entities(&written);
    let top_view = entities
        .iter()
        .filter(|entity| entity.id.starts_with(&prefix))
        .collect::<Vec<_>>();
    // DXF numbers carry six decimals of page millimetres.
    let close = |left: f64, right: f64| (left - right).abs() <= 2e-6 / scale.min(1.0);

    // The hole is one CIRCLE at the centre of the top face, radius 3.
    let [lo, hi] = view.bounds_mm;
    let middle = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let hole = top_view
        .iter()
        .find(|entity| entity.kind == "CIRCLE" && close(entity.codes[&40] / scale, HOLE_MM * 0.5))
        .unwrap_or_else(|| panic!("no hole circle in the top view:\n{written}"));
    assert_eq!(hole.layer, "VISIBLE");
    let center = model(hole.codes[&10], hole.codes[&20]);
    assert!(
        close(center[0], middle[0]) && close(center[1], middle[1]),
        "{center:?} != {middle:?}"
    );
    // No line of the top view is a chord of the hole any more.
    let on_hole = |page_x: f64, page_y: f64| {
        let point = model(page_x, page_y);
        close(
            (point[0] - center[0]).hypot(point[1] - center[1]),
            HOLE_MM * 0.5,
        )
    };
    assert!(
        !top_view.iter().any(|entity| entity.kind == "LINE"
            && on_hole(entity.codes[&10], entity.codes[&20])
            && on_hole(entity.codes[&11], entity.codes[&21])),
        "the hole is still drawn as lines"
    );

    // The rounded corner is a quarter ARC of radius 5, its centre 5 inside
    // both sides it joins.
    let round = top_view
        .iter()
        .find(|entity| entity.kind == "ARC" && entity.layer == "VISIBLE")
        .unwrap_or_else(|| panic!("no arc in the top view:\n{written}"));
    assert!(close(round.codes[&40] / scale, ROUND_MM));
    let span = (round.codes[&51] - round.codes[&50]).rem_euclid(360.0);
    assert!((span - 90.0).abs() < 1e-4, "{span}");
    let center = model(round.codes[&10], round.codes[&20]);
    let inside = |axis: usize| {
        close(hi[axis] - center[axis], ROUND_MM) || close(center[axis] - lo[axis], ROUND_MM)
    };
    assert!(inside(0) && inside(1), "{center:?} in {lo:?}..{hi:?}");

    let curves = &exported["dxf_curves"];
    let count = |kind: &str| entities.iter().filter(|entity| entity.kind == kind).count();
    assert_eq!(curves["circles"], count("CIRCLE"), "{curves}");
    assert_eq!(curves["arcs"], count("ARC"), "{curves}");
    assert!(curves["replaced_lines"].as_u64().unwrap() > 0, "{curves}");
    let lines = drawing
        .views
        .iter()
        .map(|view| view.visible_lines.len() + view.hidden_lines.len())
        .sum::<usize>();
    assert_eq!(
        layer_lines(&written, "VISIBLE") + layer_lines(&written, "HIDDEN"),
        lines - curves["replaced_lines"].as_u64().unwrap() as usize,
        "every replaced line is gone, every other line stays"
    );
    // Front and right see the hole and the round edge-on, as straight lines;
    // no exact curve is left as lines.
    assert_eq!(curves["curves_as_lines"], json!([]), "{curves}");
    assert_eq!(curves["curves_as_lines_count"], 0);

    assert_the_dwg_carries_the_dxf(
        &mut app,
        &mut bridge,
        id,
        &written,
        &directory.path().join("rounded.dwg"),
    );
}

/// A round bar with a flat (review DR-1): Ø100 with a 3 mm flat is, seen along
/// its axis, an ARC of 320.1 degrees and the LINE of the flat. The flat's ends
/// lie on the exact circle and it spans 39.9 degrees, like one step of a
/// tessellated circle, but it is no part of the circular edge.
#[test]
fn a_round_bar_with_a_flat_is_an_arc_and_the_line_of_the_flat_in_the_dxf() {
    const RADIUS_MM: f64 = 50.0;
    const FLAT_MM: f64 = 3.0;
    let (mut app, mut bridge) = setup();
    app.new_document();
    let source = format!(
        "bar = revolve('bar', profile = [(0, 0), ({RADIUS_MM}, 0), ({RADIUS_MM}, 20), (0, 20)], \
         axis = [(0, 0), (0, 1)])\n\
         rotate(bar, axis = (1, 0, 0), angle = 90)\n\
         trim(bar, ({}, 0, 0), (1, 0, 0))\n",
        RADIUS_MM - FLAT_MM
    );
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            variants: Default::default(),
            file_name: "flat.star".into(),
            source,
            overrides: BTreeMap::new(),
        },
        false,
    )
    .unwrap();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("flat.dxf");
    let exported = bridge
        .execute(
            &mut app,
            export_drawings(&path, json!({"sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");

    let written = std::fs::read_to_string(&path).unwrap();
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    // The view along the bar's axis: the one that faces its rims.
    let index = drawing
        .views
        .iter()
        .position(|view| !view.circles.is_empty() && view.kind.stable_name() != "back")
        .unwrap();
    let (view, placement) = (
        &drawing.views[index],
        &drawing.layout.view_placements[index],
    );
    let scale = (placement.page_bounds_mm[1][0] - placement.page_bounds_mm[0][0])
        / (view.bounds_mm[1][0] - view.bounds_mm[0][0]);
    let prefix = format!("{}/", view.stable_view_id);
    let entities = entities(&written);
    let axis_view = entities
        .iter()
        .filter(|entity| entity.id.starts_with(&prefix) && entity.layer == "VISIBLE")
        .collect::<Vec<_>>();
    let close = |left: f64, right: f64| (left - right).abs() <= 1e-4;

    assert!(
        !axis_view.iter().any(|entity| entity.kind == "CIRCLE"),
        "the bar with a flat is drawn as a full circle:\n{written}"
    );
    let half_gap = ((RADIUS_MM - FLAT_MM) / RADIUS_MM).acos().to_degrees();
    let rim = axis_view
        .iter()
        .find(|entity| entity.kind == "ARC" && close(entity.codes[&40] / scale, RADIUS_MM))
        .unwrap_or_else(|| {
            panic!(
                "no rim arc in the view along the axis: {} {:?} {:?}",
                exported["dxf_curves"],
                view.circles,
                axis_view
                    .iter()
                    .map(|entity| (
                        entity.kind.as_str(),
                        entity.codes.get(&40).map(|r| r / scale)
                    ))
                    .collect::<Vec<_>>()
            )
        });
    let span = (rim.codes[&51] - rim.codes[&50]).rem_euclid(360.0);
    assert!((span - (360.0 - 2.0 * half_gap)).abs() < 1e-3, "{span}");
    let chord = 2.0 * RADIUS_MM * half_gap.to_radians().sin();
    assert!(
        axis_view.iter().any(|entity| {
            entity.kind == "LINE"
                && close(
                    (entity.codes[&11] - entity.codes[&10])
                        .hypot(entity.codes[&21] - entity.codes[&20])
                        / scale,
                    chord,
                )
        }),
        "the flat of {chord} mm is no line of the view along the axis:\n{written}"
    );
}

/// The geometry of every LINE, CIRCLE and ARC: type, layer, KETCHUP id and
/// its numbers to five decimals of page millimetres, sorted.
fn drawn(dxf: &str) -> Vec<String> {
    let mut drawn = entities(dxf)
        .into_iter()
        .filter(|entity| matches!(entity.kind.as_str(), "LINE" | "CIRCLE" | "ARC"))
        .map(|entity| {
            let numbers = [10, 20, 11, 21, 40, 50, 51]
                .iter()
                .filter_map(|code| {
                    entity
                        .codes
                        .get(code)
                        .map(|value| format!("{code}={value:.5}"))
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {} {} {numbers}", entity.kind, entity.layer, entity.id)
        })
        .collect::<Vec<_>>();
    drawn.sort();
    drawn
}

/// The `\U+XXXX` escapes of an R12 string as the characters they stand for.
fn decode_dxf_unicode(value: &str) -> String {
    let mut units = Vec::new();
    let mut rest = value;
    while let Some(character) = rest.chars().next() {
        if let Some(hex) = rest.strip_prefix("\\U+").and_then(|tail| tail.get(..4)) {
            units.push(u16::from_str_radix(hex, 16).unwrap());
            rest = &rest[7..];
        } else {
            let mut buffer = [0; 2];
            units.extend_from_slice(character.encode_utf16(&mut buffer));
            rest = &rest[character.len_utf8()..];
        }
    }
    String::from_utf16(&units).unwrap()
}

/// The same sheet as DWG: an AutoCAD 2018 file that reads back with every
/// line, circle and arc of the DXF on its layer, at its place, with its id.
fn assert_the_dwg_carries_the_dxf(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    id: u64,
    dxf: &str,
    path: &Path,
) {
    let exported = bridge
        .execute(app, export_drawings(path, json!({"sheet_id": id})), false)
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");
    assert_eq!(exported["file_format"], "dwg");
    let dwg = std::fs::read(path).unwrap();
    assert_eq!(exported["bytes"], dwg.len());
    assert!(dwg.starts_with(ketchup_dwg::WRITTEN_VERSION.as_bytes()));
    let read = ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap();
    assert_eq!(read.version, ketchup_dwg::WRITTEN_VERSION);
    assert!(read.skipped.is_empty(), "{:?}", read.skipped);
    let back = String::from_utf8(read.dxf).unwrap();
    let expected = drawn(dxf);
    assert!(expected.iter().any(|entity| entity.starts_with("CIRCLE ")));
    assert!(expected.iter().any(|entity| entity.starts_with("ARC ")));
    assert_eq!(drawn(&back), expected);
    // The R12 sheet writes a character outside ASCII as its \U+XXXX escape;
    // the DWG holds the character itself.
    let texts = |dxf: &str| {
        let mut texts = entities(dxf)
            .into_iter()
            .filter(|entity| entity.kind == "TEXT")
            .map(|entity| (entity.id, decode_dxf_unicode(&entity.text)))
            .collect::<Vec<_>>();
        texts.sort();
        texts
    };
    assert!(!texts(dxf).is_empty());
    assert_eq!(texts(&back), texts(dxf));
}

/// A sheet's top view draws the plan's shown annotations where they lie: the
/// texts at their points, each dimension with the value Kečup measured centred
/// above its line, and the leader. A dimension straight up has no length in
/// plan and one text off the sheet is named; hidden, they leave the sheet.
#[test]
fn a_drawing_sheet_draws_the_plan_annotations_in_its_top_view() {
    let (mut app, mut bridge) = setup();
    let program = Request::ApplyProgram {
        expected: None,
        source: "box('slab', (6000, 3000, 200), at=(0, 0, -200))\n\
                 points = [(0, 0), (3000, 0), (6000, 0)]\n\
                 for i, p in enumerate(points):\n    \
                 text('P' + str(i + 1), 'P' + str(i + 1), at=p, height_mm=3.5, tags='plan')\n\
                 chain_dim('south', points, offset_mm=500, tags='plan')\n\
                 leader('anchor', 'M24', point=(1000, 1000), at=(2000, 2000), tags='plan')\n\
                 dim('up', (0, 0, 0), (0, 0, 2000), offset_mm=300, tags='plan')\n\
                 text('far', 'far', at=(900000, 0), tags='plan')\n"
            .into(),
        overrides: None,
        file_name: Some("plan.star".into()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, program, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let directory = tempfile::tempdir().unwrap();
    let export = |app: &mut KetchupApp, bridge: &mut LiveBridge, name: &str| {
        let path = directory.path().join(name);
        let exported = bridge
            .execute(app, export_drawings(&path, json!({"sheet_id": id})), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        (exported, std::fs::read_to_string(&path).unwrap())
    };

    let (exported, written) = export(&mut app, &mut bridge, "plan.dxf");
    assert_eq!(
        exported["annotations"],
        json!({"texts": 3, "dimensions": 2, "leaders": 1, "lines": 0,
               "without_plan_view": 0, "outside_plan": 1,
               "off_sheet": ["far"], "off_sheet_count": 1}),
        "{exported}"
    );
    let sheet = entities(&written);
    let of = |kind: &str, id: &str| {
        let found = sheet
            .iter()
            .filter(|entity| entity.kind == kind && entity.id == format!("annotation-{id}"))
            .collect::<Vec<_>>();
        assert!(!found.is_empty(), "no {kind} of {id}");
        assert!(
            found.iter().all(|entity| entity.layer == "plan"),
            "{id} is on the tag's layer"
        );
        found
    };
    let at = |entity: &Entity| [entity.codes[&10], entity.codes[&20]];
    let points = ["P1", "P2", "P3"].map(|name| {
        let [label] = of("TEXT", name).try_into().ok().unwrap();
        assert_eq!(label.text, name);
        at(label)
    });
    // The points lie 3000 mm apart along the sheet's x at the view's scale.
    let scale = (points[1][0] - points[0][0]) / 3000.0;
    assert!(scale > 0.0);
    assert!((points[2][0] - points[1][0] - 3000.0 * scale).abs() < 0.01);
    assert!((points[1][1] - points[0][1]).abs() < 0.01);
    // Each dimension is a DIMENSION through its two points, carrying the
    // measured value, which its block writes centred above the line.
    for (name, pair) in ["south/1", "south/2"].iter().zip(points.windows(2)) {
        let [dimension] = of("DIMENSION", name).try_into().ok().unwrap();
        assert_eq!(dimension.text, "3000");
        assert_eq!(dimension.codes[&70], 1.0, "aligned");
        for (code, point) in [(13, pair[0]), (14, pair[1])] {
            assert!((dimension.codes[&code] - point[0]).abs() < 0.01);
            assert!((dimension.codes[&(code + 10)] - point[1]).abs() < 0.01);
        }
        let middle = [dimension.codes[&11], dimension.codes[&21]];
        assert!((middle[0] - f64::midpoint(pair[0][0], pair[1][0])).abs() < 0.01);
        // 500 mm into the plan, and 1 mm on paper above the line.
        assert!((middle[1] - (pair[0][1] + 500.0 * scale + 1.0)).abs() < 0.01);
    }
    let block_values = written.matches("\n1\n3000\n").count();
    assert_eq!(
        block_values, 4,
        "two DIMENSION values and the two TEXTs of their blocks"
    );
    let [leader] = of("TEXT", "anchor").try_into().ok().unwrap();
    assert_eq!(leader.text, "M24");
    assert!(text_free(&sheet, "up") && text_free(&sheet, "far"));

    // As DWG the sheet keeps both DIMENSION entities and the texts on the tag's layer.
    let dwg = directory.path().join("plan.dwg");
    bridge
        .execute(
            &mut app,
            export_drawings(&dwg, json!({"sheet_id": id})),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let back = ketchup_dwg::dwg_to_dxf(&std::fs::read(&dwg).unwrap(), 64 << 20).unwrap();
    let back = entities(&String::from_utf8(back.dxf).unwrap());
    let on_plan = |kind: &str| {
        back.iter()
            .filter(|entity| entity.kind == kind && entity.layer == "plan")
            .collect::<Vec<_>>()
    };
    let dimensions = on_plan("DIMENSION");
    assert_eq!(dimensions.len(), 2);
    assert!(dimensions.iter().all(|dimension| dimension.text == "3000"));
    let mut labels = on_plan("TEXT")
        .iter()
        .map(|text| text.text.clone())
        .collect::<Vec<_>>();
    labels.sort();
    assert_eq!(labels, ["M24", "P1", "P2", "P3"]);

    // The PDF of the same sheet writes both values centred by a text matrix.
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    let drawing = project_orthographic_drawing(&snapshot, &app.exact.results, sheet).unwrap();
    let bundle = export_drawing(&snapshot, &drawing).unwrap();
    let pdf = String::from_utf8_lossy(bundle.pdf());
    assert_eq!(pdf.matches("Tm <33303030> Tj").count(), 2, "{pdf}");
    drop(snapshot);

    // Hidden, the annotations leave the sheet.
    let hide = Request::TagVisibility {
        expected: None,
        name: "plan".into(),
        visible: false,
    };
    bridge
        .execute(&mut app, hide, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let (exported, written) = export(&mut app, &mut bridge, "hidden.dxf");
    assert_eq!(exported["annotations"]["texts"], 0, "{exported}");
    assert_eq!(exported["annotations"]["dimensions"], 0);
    assert!(!written.contains("annotation-"));
}

fn text_free(entities: &[Entity], id: &str) -> bool {
    let id = format!("annotation-{id}");
    !entities.iter().any(|entity| entity.id.starts_with(&id))
}
