//! `drawing action=view` and `action=solid_from_views`: a drawing of an L bracket, its
//! plan with a square hole and its front view on one sheet, imported twice; one copy stays
//! the plan, the other is aligned and stood up as the front view, and the AI asks for the
//! solid both show: one Undo step, the exact volume of the bracket, following the views.

use super::program_edit::{evaluate_exact, exact_volume};
use super::underlay_objects::{ask, imported, outline};
use super::*;
use egui_kittest::kittest::Queryable;

/// Plan: 100 × 60 with a 20 × 20 hole at (50..70, 20..40). Front view, drawn 200 below:
/// an L 100 wide, its foot 20 high, its upright 20 thick and 80 high.
fn bracket_sheet() -> String {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{}{}{}0\nENDSEC\n0\nEOF\n",
        outline(
            "part",
            &[[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]]
        ),
        outline(
            "part",
            &[[50.0, 20.0], [70.0, 20.0], [70.0, 40.0], [50.0, 40.0]]
        ),
        outline(
            "part",
            &[
                [0.0, -200.0],
                [100.0, -200.0],
                [100.0, -180.0],
                [20.0, -180.0],
                [20.0, -120.0],
                [0.0, -120.0],
            ]
        ),
    )
}

/// The foot and upright times the 60 deep plan, less the hole through the foot.
const BRACKET_MM3: f64 = (100.0 * 20.0 + 20.0 * 60.0) * 60.0 - 20.0 * 20.0 * 20.0;

fn call(app: &mut KetchupApp, bridge: &mut LiveBridge, method: &str, arguments: Value) -> Value {
    let mut arguments = arguments;
    arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    bridge
        .execute(app, ask(method, arguments), false)
        .unwrap_or_else(|code| panic!("{method}: {code}: {:?}", take_error_details()))
}

/// The sheet imported once as the plan (underlay 1) and again (underlay 2), the second
/// moved so the front view's foot lies on the plan's X axis at height 0.
fn plan_and_aligned() -> (KetchupApp, LiveBridge, [f64; 2]) {
    let sheet = bracket_sheet();
    let (mut app, mut bridge, offset) = imported(&sheet);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("front.dxf");
    std::fs::write(&path, &sheet).unwrap();
    let import = Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
    };
    bridge.execute(&mut app, import, false).unwrap();
    let ids = app
        .document
        .current()
        .underlays()
        .map(|underlay| underlay.id.0)
        .collect::<Vec<_>>();
    assert_eq!(ids, [1, 2]);
    let at = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];
    call(
        &mut app,
        &mut bridge,
        "fit_underlay",
        json!({"underlay_id": 2, "from_mm": [at([0.0, -200.0]), at([100.0, -200.0])], "to_mm": [[0.0, 0.0], [100.0, 0.0]], "scale": false}),
    );
    (app, bridge, offset)
}

/// [`plan_and_aligned`], the second drawing stood up as the front view by the AI.
fn plan_and_front() -> (KetchupApp, LiveBridge, [f64; 2]) {
    let (mut app, mut bridge, offset) = plan_and_aligned();
    let steps = app.undo_step_count();
    let stood = call(
        &mut app,
        &mut bridge,
        "drawing_view",
        json!({"underlay_id": 2, "plane": "front"}),
    );
    assert_eq!(stood["plane"], "front", "{stood}");
    assert_eq!(stood["drawing_y_world"], json!([0.0, 0.0, 1.0]));
    assert_eq!(app.undo_step_count(), steps + 1);
    (app, bridge, offset)
}

#[test]
fn a_front_view_stands_on_its_vertical_plane() {
    let (app, _bridge, _) = plan_and_front();
    let snapshot = app.document.current();
    let bounds = app.shown_underlay_bounds(&snapshot);
    let (origin, size) = bounds[1];
    // The whole second sheet stands in the XZ plane: the front view from the ground up,
    // the plan drawn on the sheet above it 200 to 260 high.
    assert!(origin.y.abs() < 1e-9 && size.y.abs() < 1e-9, "{bounds:?}");
    assert!(origin.z.abs() < 1e-6, "{bounds:?}");
    assert!((origin.z + size.z - 260.0).abs() < 1e-6, "{bounds:?}");
    // The plan still lies on the ground.
    assert!(bounds[0].1.z.abs() < 1e-9, "{bounds:?}");
    let front = snapshot.underlays().nth(1).unwrap();
    assert!(!front.lies_flat());
}

#[test]
fn the_ai_builds_the_solid_a_plan_and_a_front_view_show_and_it_follows_the_view() {
    let (mut app, mut bridge, offset) = plan_and_front();
    let at = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];
    let steps = app.undo_step_count();
    let built = call(
        &mut app,
        &mut bridge,
        "drawing_solid_from_views",
        json!({"name": "BRACKET", "views": [
            {"underlay_id": 1, "at_mm": at([10.0, 10.0])},
            {"underlay_id": 2, "at_mm": [10.0, 10.0]},
        ]}),
    );
    assert_eq!(built["name"], "BRACKET", "{built}");
    assert_eq!(built["views"][1]["plane"], "front");
    assert_eq!(app.undo_step_count(), steps + 1);
    let source = app.document.current_rule_program().unwrap().source.clone();
    assert!(
        source.contains("BRACKET_V2_FRAME = ((0, 0, 0), (1, 0, 0), (0, 0, 1))"),
        "{source}"
    );
    evaluate_exact(&mut app);
    let volume = exact_volume(&app, "BRACKET");
    assert!(
        (volume - BRACKET_MM3).abs() < 1.0,
        "{volume} != {BRACKET_MM3}"
    );

    // The front view moved 10 to the right on its sheet: the bracket follows it, its foot
    // now 90 long within the plan, in the same Undo step as the move.
    let steps = app.undo_step_count();
    call(
        &mut app,
        &mut bridge,
        "fit_underlay",
        json!({"underlay_id": 2, "from_mm": [[0.0, 0.0], [100.0, 0.0]], "to_mm": [[10.0, 0.0], [110.0, 0.0]], "scale": false}),
    );
    assert_eq!(app.undo_step_count(), steps + 1);
    evaluate_exact(&mut app);
    let moved = (90.0 * 20.0 + 20.0 * 60.0) * 60.0 - 20.0 * 20.0 * 20.0;
    let volume = exact_volume(&app, "BRACKET");
    assert!((volume - moved).abs() < 1.0, "{volume} != {moved}");

    assert!(app.undo());
    evaluate_exact(&mut app);
    assert!((exact_volume(&app, "BRACKET") - BRACKET_MM3).abs() < 1.0);
    assert!(app.undo());
    assert!(
        app.document
            .current()
            .occurrences()
            .all(|occurrence| occurrence.name() != "BRACKET")
    );
}

/// Clicks Tools > Drawing view > the `plane` entry of the second drawing.
fn stand_second(harness: &mut egui_kittest::Harness<'_, KetchupApp>, plane: &str) {
    let text = |harness: &egui_kittest::Harness<'_, KetchupApp>, key: &str| {
        harness.state().catalog.text(key)
    };
    let tools = text(harness, "menu-tools");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, &tools)
        .click();
    harness.run();
    let submenu = text(harness, "menu-underlay-view");
    // A submenu's label ends in its arrow.
    harness.get_by_label_contains(&submenu).click();
    harness.run();
    let entry = text(harness, plane);
    let entries = harness.get_all_by_label(&entry).collect::<Vec<_>>();
    assert_eq!(entries.len(), 2, "one {entry} per drawing");
    entries[1].click();
    harness.run();
}

/// 2D-5 in the window: Tools > Drawing view stands the aligned second drawing up as the
/// front view, one Undo step; the AI builds the solid from the plan and that view; laid
/// back into the plan from the same menu, the solid follows in that one step: the plan's
/// area and the L's, both seen from above, now cross only where they overlap, 2 high.
#[test]
fn tools_drawing_view_stands_a_drawing_up_and_the_solid_follows_it() {
    let (app, mut bridge, offset) = plan_and_aligned();
    let at = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let steps = harness.state().undo_step_count();
    stand_second(&mut harness, "underlay-view-front");
    let app = harness.state();
    assert_eq!(app.undo_step_count(), steps + 1, "{}", app.digest);
    assert!(app.digest.contains("Front view"), "{}", app.digest);
    let snapshot = app.document.current();
    let (origin, size) = app.shown_underlay_bounds(&snapshot)[1];
    assert!(origin.y.abs() < 1e-9 && size.y.abs() < 1e-9 && origin.z.abs() < 1e-6);

    call(
        harness.state_mut(),
        &mut bridge,
        "drawing_solid_from_views",
        json!({"name": "BRACKET", "views": [
            {"underlay_id": 1, "at_mm": at([10.0, 10.0])},
            {"underlay_id": 2, "at_mm": [10.0, 10.0]},
        ]}),
    );
    evaluate_exact(harness.state_mut());
    let volume = exact_volume(harness.state(), "BRACKET");
    assert!(
        (volume - BRACKET_MM3).abs() < 1.0,
        "{volume} != {BRACKET_MM3}"
    );

    let steps = harness.state().undo_step_count();
    stand_second(&mut harness, "underlay-view-top");
    assert_eq!(harness.state().undo_step_count(), steps + 1);
    assert!(
        harness
            .state()
            .document
            .current_rule_program()
            .unwrap()
            .source
            .contains("BRACKET_V2_FRAME = ((0, 0, 0), (1, 0, 0), (0, 1, 0))")
    );
    evaluate_exact(harness.state_mut());
    let crossed = (100.0 * 20.0 + 20.0 * 40.0) * 2.0;
    let volume = exact_volume(harness.state(), "BRACKET");
    assert!((volume - crossed).abs() < 1.0, "{volume} != {crossed}");

    assert!(harness.state_mut().undo());
    evaluate_exact(harness.state_mut());
    assert!((exact_volume(harness.state(), "BRACKET") - BRACKET_MM3).abs() < 1.0);
}

/// A real sheet: the DXF Kečup writes for a drawing of an L piece 100 × 80, 60 thick, a
/// 20 × 10 slot through its foot: its front, top and right views drawn at the sheet's
/// scale on VISIBLE, the slot's hidden edges on HIDDEN; the right view split inside by
/// the edge where the foot meets the upright. Returns the sheet and the piece's volume.
fn piece_sheet() -> (String, f64) {
    let (mut app, mut bridge) = setup();
    let source = "P = extrude(\"PIECE\", profile = [(0, 0), (100, 0), (100, 20), (20, 20), (20, 80), (0, 80)], distance = 60)\ncut(P, profile = [(50, 5), (70, 5), (70, 15), (50, 15)], through = True, name = \"PIECE/slot\")\n";
    let applied = Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: Some(BTreeMap::new()),
        file_name: Some("piece.star".to_owned()),
        replace_document: true,
    };
    bridge
        .execute(&mut app, applied, false)
        .unwrap_or_else(|code| panic!("{code}: {:?}", take_error_details()));
    evaluate_exact(&mut app);
    let volume = exact_volume(&app, "PIECE");
    let id = super::drawing_sheet::sheet_of_parts(&mut app, &mut bridge, 1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("piece.dxf");
    let exported = bridge
        .execute(
            &mut app,
            ask("export_drawings", json!({"path": path, "sheet_id": id})),
            false,
        )
        .unwrap();
    assert_eq!(exported["exported"], true, "{exported}");
    (std::fs::read_to_string(&path).unwrap(), volume)
}

/// `sheet` imported `copies` times into an empty window.
fn imported_copies(sheet: &str, copies: usize) -> (KetchupApp, LiveBridge) {
    let (mut app, mut bridge, _) = imported(sheet);
    let directory = tempfile::tempdir().unwrap();
    for copy in 1..copies {
        let path = directory.path().join(format!("copy{copy}.dxf"));
        std::fs::write(&path, sheet).unwrap();
        let import = Request::Import {
            expected: None,
            path: path.display().to_string(),
            unit: None,
            as_underlay: true,
        };
        bridge.execute(&mut app, import, false).unwrap();
    }
    (app, bridge)
}

/// The views of drawing `id` as the AI finds them: the whole figures of its VISIBLE
/// lines, each `[[x, y], [x, y]]` in document mm.
fn view_bounds(app: &mut KetchupApp, bridge: &mut LiveBridge, id: u64) -> Vec<[[f64; 2]; 2]> {
    let found = call(
        app,
        bridge,
        "underlay_regions",
        json!({"underlay_id": id, "layers": ["VISIBLE"], "whole": true, "min_area_m2": 0.0}),
    );
    found["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| serde_json::from_value(row["bounds_mm"].clone()).unwrap())
        .collect()
}

/// 2D-5 on a real sheet: three copies of it, one per view; the AI finds each view as a
/// whole figure (the right one has an edge inside it), aligns it to the part's size and
/// stands it on its plane, then builds the solid all three show: the piece, slot and all.
#[test]
fn the_ai_rebuilds_a_part_from_the_three_views_of_its_drawing_sheet() {
    let (sheet, volume) = piece_sheet();
    assert!((volume - 180_000.0).abs() < 1.0, "{volume}");
    let (mut app, mut bridge) = imported_copies(&sheet, 3);
    let figures = view_bounds(&mut app, &mut bridge, 1);
    // The views are the figures no other figure holds; the slot is a figure in the top one.
    let inside = |[a, b]: [[f64; 2]; 2], [c, d]: [[f64; 2]; 2]| {
        a != c && a[0] >= c[0] && a[1] >= c[1] && b[0] <= d[0] && b[1] <= d[1]
    };
    let views = figures
        .iter()
        .copied()
        .filter(|figure| !figures.iter().any(|other| inside(*figure, *other)))
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 3, "front, top and right: {figures:?}");
    let size = |[low, high]: [[f64; 2]; 2]| [high[0] - low[0], high[1] - low[1]];
    // Told apart by their proportions: X × Z, X × Y and Y × Z of 100 × 80 × 60.
    let find = |ratio: f64| {
        *views
            .iter()
            .find(|view| {
                let [w, h] = size(**view);
                (w / h - ratio).abs() < 0.02
            })
            .unwrap_or_else(|| panic!("a view {ratio}: {views:?}"))
    };
    let (front, top, right) = (find(100.0 / 60.0), find(100.0 / 80.0), find(80.0 / 60.0));
    let regions = call(
        &mut app,
        &mut bridge,
        "underlay_regions",
        json!({"underlay_id": 1, "layers": ["VISIBLE"], "min_area_m2": 0.0}),
    );
    assert!(
        regions["regions"].as_array().unwrap().len() > 3,
        "the right view's inside edge splits it into faces: {regions}"
    );
    // Each copy aligned: the bottom edge of its view on the part's edge, at full size.
    for (id, view, length, plane) in [
        (1, top, 100.0, "top"),
        (2, front, 100.0, "front"),
        (3, right, 80.0, "right"),
    ] {
        let [low, high] = view;
        call(
            &mut app,
            &mut bridge,
            "fit_underlay",
            json!({"underlay_id": id, "from_mm": [low, [high[0], low[1]]], "to_mm": [[0.0, 0.0], [length, 0.0]], "scale": true}),
        );
        if plane != "top" {
            call(
                &mut app,
                &mut bridge,
                "drawing_view",
                json!({"underlay_id": id, "plane": plane}),
            );
        }
    }
    let steps = app.undo_step_count();
    call(
        &mut app,
        &mut bridge,
        "drawing_solid_from_views",
        json!({"name": "REBUILT", "views": [
            {"underlay_id": 1, "at_mm": [10.0, 40.0], "layers": ["VISIBLE"]},
            {"underlay_id": 2, "at_mm": [50.0, 30.0], "layers": ["VISIBLE"]},
            {"underlay_id": 3, "at_mm": [40.0, 30.0], "layers": ["VISIBLE"]},
        ]}),
    );
    assert_eq!(app.undo_step_count(), steps + 1);
    evaluate_exact(&mut app);
    let rebuilt = exact_volume(&app, "REBUILT");
    assert!((rebuilt - volume).abs() < 1.0, "{rebuilt} != {volume}");
}

#[test]
fn views_seen_along_one_axis_make_no_solid() {
    let (mut app, mut bridge, offset) = imported(&bracket_sheet());
    let at = |[x, y]: [f64; 2]| [x - offset[0], y - offset[1]];
    let digest = app.canonical_digest();
    let mut arguments = json!({"views": [
        {"underlay_id": 1, "at_mm": at([10.0, 10.0])},
        {"underlay_id": 1, "at_mm": at([10.0, -190.0])},
    ]});
    arguments["expected"] = serde_json::to_value(app.live_bridge_stamp()).unwrap();
    let refused = bridge.execute(&mut app, ask("drawing_solid_from_views", arguments), false);
    assert_eq!(refused, Err("invalid_params"));
    let details = take_error_details().unwrap();
    assert_eq!(details["target"], "views", "{details}");
    assert_eq!(app.canonical_digest(), digest);
}
