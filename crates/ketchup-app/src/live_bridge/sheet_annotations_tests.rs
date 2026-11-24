use super::drawing_sheet::sheet_of_parts;
use super::*;
use ketchup_model::drawing::DrawingSheetId;

fn request(method: &str, fields: Value) -> Request {
    let mut value = fields;
    value["method"] = json!(method);
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_client_can_discover_sheet_edges_and_add_annotations_in_one_undo_step() {
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    let id = sheet_of_parts(&mut app, &mut bridge, 1);
    let geometry = bridge
        .execute(
            &mut app,
            request("drawing_sheet", json!({"sheet_id": id, "view": "front"})),
            false,
        )
        .unwrap();
    let lines = geometry["lines"].as_array().unwrap();
    assert!(lines.len() >= 4, "{geometry}");
    let pair = lines
        .iter()
        .enumerate()
        .find_map(|(i, a)| {
            lines.iter().skip(i + 1).find_map(|b| {
                let delta = |line: &Value| -> [f64; 2] {
                    std::array::from_fn(|axis| {
                        line["end_mm"][axis].as_f64().unwrap()
                            - line["start_mm"][axis].as_f64().unwrap()
                    })
                };
                (ketchup_geometry::linalg::dot2(delta(a), delta(b)).abs() < 1e-9)
                    .then(|| [a["id"].clone(), b["id"].clone()])
            })
        })
        .unwrap();
    let before = app
        .document
        .current()
        .drawing_sheet(DrawingSheetId(id))
        .unwrap()
        .clone();
    let undo = app.undo_step_count();
    let added = bridge.execute(&mut app, request("drawing_annotate", json!({"sheet_id": id, "annotations": [
        {"kind": "angular", "view": "front", "source_line_ids": pair, "arc_radius_page_mm": 8.0},
        {"kind": "datum", "view": "front", "source_line_id": pair[0], "label": "A", "offset_page_mm": [15.0, 15.0]},
        {"kind": "feature_control_frame", "view": "front", "source_line_id": pair[1], "characteristic": "Parallelism", "tolerance_mm": 0.1, "datum_labels": ["A"], "offset_page_mm": [20.0, 20.0]}
    ]})), false).unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    assert_eq!(added["added"], 3);
    assert_eq!(app.undo_step_count(), undo + 1);
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    assert_eq!(sheet.angular_dimensions().len(), 1);
    assert_eq!(sheet.datum_symbols()[0].label(), "A");
    assert_eq!(sheet.feature_control_frames()[0].tolerance_mm(), 0.1);
    assert_eq!(sheet.bom_balloons(), before.bom_balloons());
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(
        app.document
            .current()
            .drawing_sheet(DrawingSheetId(id))
            .unwrap(),
        &before
    );
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(
        app.document
            .current()
            .drawing_sheet(DrawingSheetId(id))
            .unwrap(),
        sheet
    );
    let saved = ketchup_model::persistence::save(&app.document.current());
    let reopened = ketchup_model::persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(
        reopened
            .current()
            .drawing_sheet(DrawingSheetId(id))
            .unwrap(),
        sheet
    );
}

#[test]
fn a_client_cannot_mutate_a_sheet_with_a_stale_stamp_or_bad_reference() {
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    let stale = app.live_bridge_stamp();
    let id = sheet_of_parts(&mut app, &mut bridge, 1);
    let before = ketchup_model::persistence::save(&app.document.current());
    let annotations = json!([{"kind": "datum", "view": "front", "source_line_id": format!("sheet-{id}/view-front/missing"), "label": "A", "offset_page_mm": [15.0, 15.0]}]);
    assert_eq!(
        bridge.execute(
            &mut app,
            request(
                "drawing_annotate",
                json!({"sheet_id": id, "expected": stale, "annotations": annotations})
            ),
            false
        ),
        Err("stale_document")
    );
    assert!(
        bridge
            .execute(
                &mut app,
                request(
                    "drawing_annotate",
                    json!({"sheet_id": id, "annotations": annotations})
                ),
                false
            )
            .is_err()
    );
    assert_eq!(
        ketchup_model::persistence::save(&app.document.current()),
        before
    );
    let missing = bridge.execute(
        &mut app,
        request("drawing_sheet", json!({"sheet_id": id + 999})),
        false,
    );
    assert!(missing.is_err());
}

#[test]
fn sheet_annotations_gui_generates_exact_bore_text_and_undoes() {
    use egui_kittest::kittest::Queryable as _;
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, _bridge, id) = drilled_sheet();
    app.dispatch_file_command(AppCommand::ExportProjectDrawings);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1800.0, 1600.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    settle(&mut harness);
    harness.get_by_label("Annotate sheet").click();
    settle(&mut harness);
    harness.get_by_label("Generate hole callouts").click();
    settle(&mut harness);
    let snapshot = harness.state().document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    assert_eq!(sheet.notes().len(), 1);
    assert_eq!(
        sheet.notes()[0].text_template(),
        "BORE Ø6 mm DEPTH 8 mm, FLAT BOTTOM"
    );
    harness.state_mut().dispatch_command(AppCommand::Undo);
    settle(&mut harness);
    assert!(
        harness
            .state()
            .document
            .current()
            .drawing_sheet(DrawingSheetId(id))
            .unwrap()
            .notes()
            .is_empty()
    );
}

/// Frames until the drawings window has found the floors on its worker, then
/// four more for the clicked view to lay out. Exact evaluation and the
/// drawings window keep asking for frames, which `Harness::run` takes for a
/// window that never settles.
fn settle(harness: &mut egui_kittest::Harness<'_, KetchupApp>) {
    crate::tests::shell::step_until(
        harness,
        std::time::Duration::from_secs(120),
        "the drawings window never found the floors",
        |app| app.found_zero(&app.document.current()).is_some(),
    );
    harness.run_steps(4);
}

fn drilled_sheet() -> (KetchupApp, LiveBridge, u64) {
    let (mut app, mut bridge) = setup();
    super::program_edit::evaluate_exact(&mut app);
    let top = super::drill_holes::face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let position = super::drill_holes::centroid(&top);
    let drill = super::drill_holes::edit(AssistantCadEditOperation::DrillHoles {
        definition_id: top["definition_id"].as_u64().unwrap(),
        name: "Bore".into(),
        target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
        face_reference_id: top["reference_id"].as_str().unwrap().into(),
        points_local_mm: vec![position],
        diameter_mm: 6.0,
        depth_mm: Some(8.0),
        through: false,
        thread: None,
    });
    bridge.execute(&mut app, drill, false).unwrap();
    let id = sheet_of_parts(&mut app, &mut bridge, 1);
    (app, bridge, id)
}

#[test]
fn r8_refused_hole_callouts_name_the_cut_and_why_in_the_window_and_to_clients() {
    use egui_kittest::kittest::Queryable as _;
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, mut bridge) = setup();
    super::program_edit::evaluate_exact(&mut app);
    let top = super::drill_holes::face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let position = super::drill_holes::centroid(&top);
    // Two Ø6 bores 3 mm apart: neither leaves a whole bore.
    let drill = super::drill_holes::edit(AssistantCadEditOperation::DrillHoles {
        definition_id: top["definition_id"].as_u64().unwrap(),
        name: "Overlapping bores".into(),
        target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
        face_reference_id: top["reference_id"].as_str().unwrap().into(),
        points_local_mm: vec![position, [position[0] + 3.0, position[1], position[2]]],
        diameter_mm: 6.0,
        depth_mm: Some(8.0),
        through: false,
        thread: None,
    });
    bridge
        .execute(&mut app, drill, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    let id = sheet_of_parts(&mut app, &mut bridge, 1);
    let refused = bridge.execute(
        &mut app,
        request(
            "drawing_annotate",
            json!({"sheet_id": id, "hole_callouts": true}),
        ),
        false,
    );
    assert!(refused.is_err());
    let details = take_error_details().unwrap().to_string();
    assert!(
        details.contains("Overlapping bores")
            && details.contains("no complete bore")
            && details.contains("one whole bore")
            && !details.contains("feature "),
        "{details}"
    );
    app.dispatch_file_command(AppCommand::ExportProjectDrawings);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1800.0, 1600.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    settle(&mut harness);
    harness.get_by_label("Annotate sheet").click();
    settle(&mut harness);
    harness.get_by_label("Generate hole callouts").click();
    settle(&mut harness);
    let shown = harness
        .query_all_by_label_contains(": The finished body has no complete bore")
        .map(|label| {
            let node = egui_kittest::kittest::NodeT::accesskit_node(&label);
            node.label().or_else(|| node.value()).unwrap_or_default()
        })
        .collect::<Vec<_>>();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(
        shown
            .iter()
            .all(|text| text.starts_with("Overlapping bores")),
        "{shown:?}"
    );
}

#[test]
fn sheet_annotations_new_bore_invalidates_quantity_and_removed_bores_can_be_cleared() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, mut bridge, id) = drilled_sheet();
    let sheet_id = DrawingSheetId(id);
    app.annotate_drawing_sheet(sheet_id, &[], true).unwrap();
    let original = app
        .document
        .current()
        .drawing_sheet(sheet_id)
        .unwrap()
        .clone();
    let top = super::drill_holes::face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let mut position = super::drill_holes::centroid(&top);
    position[0] += 10.0;
    let drill = super::drill_holes::edit(AssistantCadEditOperation::DrillHoles {
        definition_id: top["definition_id"].as_u64().unwrap(),
        name: "Second bore".into(),
        target_feature_id: top["producer_feature_id"].as_u64().unwrap(),
        face_reference_id: top["reference_id"].as_str().unwrap().into(),
        points_local_mm: vec![position],
        diameter_mm: 6.0,
        depth_mm: Some(8.0),
        through: false,
        thread: None,
    });
    bridge
        .execute(&mut app, drill, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    assert!(
        app.drawing_sheet_geometry(sheet_id).is_err(),
        "an extra bore must invalidate the old quantity"
    );
    app.annotate_drawing_sheet(sheet_id, &[], true).unwrap();
    let regenerated = app
        .document
        .current()
        .drawing_sheet(sheet_id)
        .unwrap()
        .clone();
    assert_eq!(
        regenerated.notes()[0].text_template(),
        "2× BORE Ø6 mm DEPTH 8 mm, FLAT BOTTOM"
    );
    let sources = regenerated.notes()[0].hole_callout_sources().unwrap();
    for source in sources.iter().rev() {
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::DeleteFeature {
                id: source.feature_id,
            }]))
            .unwrap();
    }
    super::program_edit::evaluate_exact(&mut app);
    assert!(app.drawing_sheet_geometry(sheet_id).is_err());
    let answer = bridge
        .execute(
            &mut app,
            request(
                "drawing_annotate",
                json!({"sheet_id": id, "hole_callouts": true}),
            ),
            false,
        )
        .unwrap();
    assert_eq!(answer["removed_callout_sources"], 2);
    assert!(app.digest.contains("Removed callouts"));
    assert!(
        app.document
            .current()
            .drawing_sheet(sheet_id)
            .unwrap()
            .notes()
            .is_empty()
    );
    assert!(app.drawing_sheet_geometry(sheet_id).is_ok());
    app.dispatch_command(AppCommand::Undo);
    assert_eq!(
        app.document.current().drawing_sheet(sheet_id).unwrap(),
        &regenerated
    );
    assert_eq!(original.notes().len(), 1);
}

#[test]
fn sheet_annotations_client_generates_exact_bore_text_in_one_undo_step() {
    let _turn = crate::live_bridge::measurement::tests::integration_support::file_turn();
    let (mut app, mut bridge, id) = drilled_sheet();
    let undo = app.undo_step_count();
    bridge
        .execute(
            &mut app,
            request(
                "drawing_annotate",
                json!({
                    "sheet_id": id, "hole_callouts": true
                }),
            ),
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    let snapshot = app.document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(id)).unwrap();
    assert_eq!(sheet.notes().len(), 1);
    assert_eq!(
        sheet.notes()[0].text_template(),
        "BORE Ø6 mm DEPTH 8 mm, FLAT BOTTOM"
    );
    let directory = tempfile::tempdir().unwrap();
    for extension in ["svg", "pdf", "dxf"] {
        let path = directory.path().join(format!("bores.{extension}"));
        let answer = bridge
            .execute(
                &mut app,
                request(
                    "export_drawings",
                    json!({
                        "sheet_id": id, "path": path
                    }),
                ),
                false,
            )
            .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
        assert_eq!(answer["exported"], true);
        let bytes = std::fs::read(&path).unwrap();
        if extension == "pdf" {
            assert!(bytes.starts_with(b"%PDF-"));
        } else {
            assert!(String::from_utf8_lossy(&bytes).contains("DEPTH 8 mm, FLAT BOTTOM"));
        }
    }
    assert_eq!(app.undo_step_count(), undo + 1);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(
        app.document
            .current()
            .drawing_sheet(DrawingSheetId(id))
            .unwrap()
            .notes()
            .is_empty()
    );
}
