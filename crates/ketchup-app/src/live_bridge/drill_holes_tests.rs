//! `drill_holes` through apply_and_verify: a hole removes exactly its
//! cylinder in one Undo step, and a refused hole leaves the document, its
//! saved bytes and Undo untouched.

use super::program_edit::{evaluate_exact, exact_volume};
use super::*;
use eframe::egui::{Pos2, Rect, Vec2};
use ketchup_assistant::sidecar::AssistantHoleThread;
use ketchup_interaction::Vec3;
use std::f64::consts::PI;

const DIAMETER_MM: f64 = 6.0;

/// The newest face row of `surface_kind` (with `normal`, for planes).
fn face(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    surface_kind: &str,
    normal: [f64; 3],
) -> Value {
    let mut rows = Vec::new();
    let mut query = json!({"kind": "faces", "limit": 100});
    loop {
        let page = bridge
            .execute(
                app,
                Request::Query {
                    expected: None,
                    query: serde_json::from_value(query.clone()).unwrap(),
                },
                false,
            )
            .unwrap();
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        if page["next_cursor"].is_null() {
            break;
        }
        query["cursor"] = page["next_cursor"].clone();
    }
    rows.into_iter()
        .filter(|row| {
            row["geometry"]["surface_kind"] == surface_kind
                && (surface_kind != "plane"
                    || (0..3).all(|axis| {
                        row["geometry"]["unit_normal"][axis]
                            .as_f64()
                            .is_some_and(|n| (n - normal[axis]).abs() < 1e-9)
                    }))
        })
        .max_by_key(|row| row["producer_feature_id"].as_u64())
        .unwrap_or_else(|| panic!("no {surface_kind} face along {normal:?}"))
}

fn centroid(face: &Value) -> [f64; 3] {
    std::array::from_fn(|axis| face["geometry"]["centroid_mm"][axis].as_f64().unwrap())
}

fn drill(
    face: &Value,
    points_local_mm: Vec<[f64; 3]>,
    depth_mm: Option<f64>,
    through: bool,
) -> Request {
    tap(face, points_local_mm, DIAMETER_MM, depth_mm, through, None)
}

/// `drill` with any diameter and an optional (size, depth) thread.
fn tap(
    face: &Value,
    points_local_mm: Vec<[f64; 3]>,
    diameter_mm: f64,
    depth_mm: Option<f64>,
    through: bool,
    thread: Option<(&str, f64)>,
) -> Request {
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::DrillHoles {
            definition_id: face["definition_id"].as_u64().unwrap(),
            name: "holes".to_owned(),
            target_feature_id: face["producer_feature_id"].as_u64().unwrap(),
            face_reference_id: face["reference_id"].as_str().unwrap().to_owned(),
            points_local_mm,
            diameter_mm,
            depth_mm,
            through,
            thread: thread.map(|(size, depth_mm)| AssistantHoleThread {
                size: size.to_owned(),
                depth_mm,
            }),
        }],
    };
    serde_json::from_value(json!({"method": "apply_and_verify", "program": program})).unwrap()
}

fn report(app: &mut KetchupApp, bridge: &mut LiveBridge, section: ReportSection) -> Value {
    let expected = app.live_bridge_stamp();
    let page = bridge
        .execute(
            app,
            Request::ProgramReport {
                expected,
                section,
                offset: 0,
                limit: 100,
            },
            false,
        )
        .unwrap();
    assert!(page["next_offset"].is_null(), "{page}");
    page["rows"].clone()
}

fn saved(app: &KetchupApp) -> Vec<u8> {
    ketchup_model::persistence::save(&app.document.current())
}

#[test]
fn drilled_holes_remove_exactly_their_cylinders_in_one_undo_step() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let part = app
        .document
        .current()
        .occurrences()
        .next()
        .unwrap()
        .name()
        .to_owned();
    let solid = exact_volume(&app, &part);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let [x, y, z] = centroid(&top);
    let height = z - centroid(&bottom)[2];
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    let area = PI * (DIAMETER_MM * 0.5).powi(2);

    for (depth_mm, through, removed) in [(Some(8.0), false, 8.0), (None, true, height)] {
        let points = vec![[x - 20.0, y, z], [x + 20.0, y, z]];
        let report = bridge
            .execute(&mut app, drill(&top, points, depth_mm, through), false)
            .unwrap();
        assert_eq!(report["published"], true, "{report}");
        assert_eq!(app.undo_step_count(), undo + 1);
        evaluate_exact(&mut app);
        let expected = solid - 2.0 * area * removed;
        let volume = exact_volume(&app, &part);
        assert!(
            (volume - expected).abs() <= 1e-6 * solid,
            "through={through}: {volume} != {expected}"
        );

        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(
            app.live_bridge_stamp().canonical_digest,
            stamp.canonical_digest
        );
        assert!(saved(&app) == bytes, "Undo must restore the saved bytes");
    }
}

#[test]
fn refused_holes_name_their_case_and_leave_the_document_untouched() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    // A through hole gives the part a cylindrical wall to refuse.
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let [x, y, z] = centroid(&top);
    bridge
        .execute(&mut app, drill(&top, vec![[x, y, z]], None, true), false)
        .unwrap();
    evaluate_exact(&mut app);
    let wall = face(&mut app, &mut bridge, "cylinder", [0.0; 3]);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let before = (
        app.live_bridge_stamp(),
        app.undo_step_count(),
        app.redo_step_count(),
        saved(&app),
    );

    for (request, code, case) in [
        (
            drill(&wall, vec![[x + 3.0, y, z * 0.5]], Some(2.0), false),
            "planning_rejected",
            "planning.cad_hole_face_not_planar",
        ),
        (
            drill(&top, vec![[x * 3.0, y, z]], Some(2.0), false),
            "planning_rejected",
            "planning.cad_hole_point_off_face",
        ),
        (
            drill(&top, vec![[x + 20.0, y, z - 5.0]], Some(2.0), false),
            "planning_rejected",
            "planning.cad_hole_point_off_face",
        ),
        (
            drill(&top, vec![[x + 20.0, y, z]], Some(2.0), true),
            "invalid_program",
            "either depth_mm deep or through",
        ),
        (
            tap(
                &top,
                vec![[x + 20.0, y, z]],
                6.0,
                Some(8.0),
                false,
                Some(("M8", 6.0)),
            ),
            "planning_rejected",
            "planning.cad_hole_thread_diameter_mismatch",
        ),
        (
            tap(
                &top,
                vec![[x + 20.0, y, z]],
                6.0,
                Some(8.0),
                false,
                Some(("M7", 6.0)),
            ),
            "planning_rejected",
            "planning.cad_hole_thread_unknown",
        ),
    ] {
        assert_eq!(
            bridge.execute(&mut app, request, false),
            Err(code),
            "{case}"
        );
        let details = take_error_details().unwrap().to_string();
        assert!(details.contains(case), "{case}: {details}");
        let after = (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            saved(&app),
        );
        assert!(after == before, "{case}: the refusal changed the document");
    }
}

#[test]
fn a_tapped_hole_is_listed_for_machining_and_hardware_and_only_a_thread_is_saved() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let [x, y, z] = centroid(&top);
    let points = || vec![[x - 20.0, y, z], [x + 20.0, y, z]];
    let (undo, bytes) = (app.undo_step_count(), saved(&app));

    // Untapped: the saved document has no thread field at all.
    let plain = tap(&top, points(), 6.8, Some(15.0), false, None);
    assert_eq!(
        bridge.execute(&mut app, plain, false).unwrap()["published"],
        true
    );
    let untapped = saved(&app);
    assert!(!untapped.windows(12).any(|window| window == b"hole_threads"));
    assert_eq!(
        report(&mut app, &mut bridge, ReportSection::Hardware),
        json!([])
    );
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(saved(&app) == bytes);

    let tapped = tap(&top, points(), 6.8, Some(15.0), false, Some(("M8", 12.0)));
    let applied = bridge.execute(&mut app, tapped, false).unwrap();
    assert_eq!(applied["published"], true, "{applied}");
    assert_eq!(app.undo_step_count(), undo + 1);
    let machining = report(&mut app, &mut bridge, ReportSection::Machining);
    let labels: Vec<_> = machining
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["operation"]["label"].clone())
        .collect();
    assert_eq!(
        labels,
        [json!("M8×1.25 depth 12"), json!("M8×1.25 depth 12")]
    );
    assert_eq!(
        report(&mut app, &mut bridge, ReportSection::Hardware),
        json!([{"item": "M8×1.25 depth 12", "count": 2, "kind": "tapped_hole"}])
    );

    // The thread survives save and open, and Undo removes it with the holes.
    let threaded = saved(&app);
    let ketchup_model::persistence::LoadOutcome::Editable { document, .. } =
        ketchup_model::persistence::load(&threaded).unwrap()
    else {
        panic!("the saved document opens editable")
    };
    assert_eq!(document.current().hole_threads().count(), 2);
    assert!(ketchup_model::persistence::save(&document.current()) == threaded);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.undo_step_count(), undo);
    assert!(saved(&app) == bytes, "Undo must restore the saved bytes");
}

#[test]
fn the_hole_tool_drills_where_the_face_is_clicked_in_one_undo_step() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap();
    let part = occurrence.name().to_owned();
    let world = snapshot
        .world_transform_for_occurrence(occurrence.id())
        .unwrap();
    let solid = exact_volume(&app, &part);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let [x, y, z] = centroid(&top);
    let height = z - centroid(&bottom)[2];
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0));
    let [wx, wy, wz] = world.transform_point([x + 10.0, y, z]);
    let click = app.project(Vec3::new(wx, wy, wz), rect);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));
    app.dispatch_command(AppCommand::Hole);
    assert_eq!(app.active_tool, ActiveTool::Hole);

    // "6" drills through, "6 8" 8 deep: the part loses exactly the cylinder.
    for (input, removed) in [("6", height), ("6 8", 8.0)] {
        app.value_box.input = input.to_owned();
        assert!(
            app.drill_hole_at_screen(click, rect),
            "{input}: {}",
            app.digest
        );
        app.finish_window_edit();
        assert_eq!(app.digest, app.catalog.text("digest-hole-created"));
        assert_eq!(app.undo_step_count(), undo + 1);
        evaluate_exact(&mut app);
        let expected = solid - PI * 9.0 * removed;
        let volume = exact_volume(&app, &part);
        assert!(
            (volume - expected).abs() <= 1e-6 * solid,
            "{input}: {volume} != {expected}"
        );
        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
        assert_eq!(
            app.live_bridge_stamp().canonical_digest,
            stamp.canonical_digest
        );
        assert!(
            saved(&app) == bytes,
            "{input}: Undo must restore the saved bytes"
        );
    }

    // "M8 12" drills the library's tap drill and lists the thread.
    app.value_box.input = "M8 12".to_owned();
    assert!(app.drill_hole_at_screen(click, rect));
    app.finish_window_edit();
    assert_eq!(app.digest, app.catalog.text("digest-hole-created"));
    let threads: Vec<_> = app
        .document
        .current()
        .hole_threads()
        .map(|(_, thread)| thread.label())
        .collect();
    assert_eq!(threads, ["M8×1.25 depth 12"]);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(saved(&app) == bytes);

    // A hole wider than the part is refused by name; an unknown size never
    // reaches the bridge. Neither changes the document or Undo.
    let unchanged = |app: &KetchupApp| {
        app.live_bridge_stamp().canonical_digest == stamp.canonical_digest
            && app.undo_step_count() == undo
            && saved(app) == bytes
    };
    app.value_box.input = "500".to_owned();
    assert!(app.drill_hole_at_screen(click, rect));
    app.finish_window_edit();
    assert!(
        app.digest
            .starts_with("planning_rejected: Hole 1: its wall would leave the part"),
        "{}",
        app.digest
    );
    assert!(unchanged(&app));
    app.value_box.input = "M7 12".to_owned();
    assert!(!app.drill_hole_at_screen(click, rect));
    assert_eq!(app.digest, app.catalog.text("digest-hole-invalid-size"));
    assert!(!app.live.edits.is_running());
    assert!(unchanged(&app));
}
