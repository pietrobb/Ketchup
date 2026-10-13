//! `file action=export_drawings` with `sheet_id`: a drawing sheet of the
//! document is written as the DXF of its exact projection, the project sheet
//! is refused as DXF by name, and no export changes the document. Exact
//! circles the views face are CIRCLE and ARC entities, not their lines.

use super::drill_holes::{centroid, edit, face, saved};
use super::program_edit::evaluate_exact;
use super::*;
use ketchup_model::document::InstancePath;
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

/// The first part of the new document drawn on a sheet from the selection
/// (MCP `create_drawing` draws only rigid assemblies), found through the summary.
fn sheet_of_the_block(app: &mut KetchupApp, bridge: &mut LiveBridge) -> u64 {
    evaluate_exact(app);
    let root = app.document.current().occurrences().next().unwrap().id();
    app.selection.occurrences.insert(InstancePath::root(root));
    assert!(app.preview_selection_drawing(), "{}", app.action_digest());
    assert!(app.confirm_assembly_preview());
    evaluate_exact(app);
    let name = app
        .document
        .current()
        .drawing_sheets()
        .next()
        .unwrap()
        .name()
        .to_owned();
    let listed = summary(app, bridge);
    assert_eq!(listed["counts"]["drawing_sheets"], 1, "{listed}");
    assert_eq!(listed["drawing_sheets_complete"], true);
    let [sheet] = listed["drawing_sheets"].as_array().unwrap().as_slice() else {
        panic!("one drawing sheet: {listed}");
    };
    assert_eq!(sheet["name"]["text"], name.as_str());
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
fn the_project_sheet_as_dxf_and_wrong_sheet_requests_are_refused_by_name() {
    let (mut app, mut bridge) = setup();
    let id = sheet_of_the_block(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let directory = tempfile::tempdir().unwrap();
    let file = |name: &str| directory.path().join(name);
    let existing = file("existing.dxf");
    std::fs::write(&existing, b"user file").unwrap();

    for (path, extra, code) in [
        (file("house.dxf"), json!({}), "project_sheet_pdf_only"),
        (file("house.dwg"), json!({}), "project_sheet_pdf_only"),
        (
            file("missing.dxf"),
            json!({"sheet_id": id + 1}),
            "drawing_sheet_not_found",
        ),
        (file("sheet.pdf"), json!({"sheet_id": id}), "invalid_path"),
        (file("sheet.dwg"), json!({"sheet_id": id}), "invalid_path"),
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

/// One DXF entity: its type, layer, numeric codes and KETCHUP id.
struct Entity {
    kind: String,
    layer: String,
    codes: BTreeMap<u16, f64>,
    id: String,
}

fn entities(dxf: &str) -> Vec<Entity> {
    let body = dxf.split_once("2\nENTITIES\n").unwrap().1;
    let lines = body.lines().collect::<Vec<_>>();
    let mut entities = Vec::<Entity>::new();
    for pair in lines.chunks(2) {
        let [code, value] = pair else { break };
        match code.trim() {
            "0" => entities.push(Entity {
                kind: (*value).to_owned(),
                layer: String::new(),
                codes: BTreeMap::new(),
                id: String::new(),
            }),
            "8" => entities.last_mut().unwrap().layer = (*value).to_owned(),
            "1000" => entities.last_mut().unwrap().id = (*value).to_owned(),
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
}
