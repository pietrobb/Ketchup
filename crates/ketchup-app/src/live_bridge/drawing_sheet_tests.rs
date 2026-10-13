//! `file action=export_drawings` with `sheet_id`: a drawing sheet of the
//! document is written as the DXF of its exact projection, the project sheet
//! is refused as DXF by name, and no export changes the document.

use super::drill_holes::saved;
use super::program_edit::evaluate_exact;
use super::*;
use ketchup_model::document::InstancePath;
use ketchup_model::drawing::{DrawingSheetId, project_orthographic_drawing};
use ketchup_model::drawing_export::export_drawing;

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
