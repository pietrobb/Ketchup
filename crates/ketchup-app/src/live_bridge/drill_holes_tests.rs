//! `drill_holes` through apply_and_verify: a hole removes exactly its
//! cylinder in one Undo step, and a refused hole leaves the document, its
//! saved bytes and Undo untouched.

use super::program_edit::{evaluate_exact, exact_volume};
use super::*;
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
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::DrillHoles {
            definition_id: face["definition_id"].as_u64().unwrap(),
            name: "holes".to_owned(),
            target_feature_id: face["producer_feature_id"].as_u64().unwrap(),
            face_reference_id: face["reference_id"].as_str().unwrap().to_owned(),
            points_local_mm,
            diameter_mm: DIAMETER_MM,
            depth_mm,
            through,
        }],
    };
    serde_json::from_value(json!({"method": "apply_and_verify", "program": program})).unwrap()
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
