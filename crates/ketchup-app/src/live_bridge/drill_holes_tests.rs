//! `drill_holes` through apply_and_verify: a hole removes exactly its
//! cylinder in one Undo step, and a refused hole leaves the document, its
//! saved bytes and Undo untouched.

use super::program_edit::{evaluate_exact, exact_volume};
use super::*;
use eframe::egui::{Pos2, Rect, Vec2};
use ketchup_assistant::sidecar::AssistantHoleThread;
use ketchup_interaction::Vec3;
use ketchup_model::document::CanonicalError;
use std::f64::consts::PI;

const DIAMETER_MM: f64 = 6.0;

/// The newest face row of `surface_kind` (with `normal`, for planes).
pub(super) fn face(
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

pub(super) fn centroid(face: &Value) -> [f64; 3] {
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
    edit(AssistantCadEditOperation::DrillHoles {
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
            modeled_thread: None,
        }),
    })
}

pub(super) fn edit(operation: AssistantCadEditOperation) -> Request {
    let program = AssistantCadEditProgram {
        operations: vec![operation],
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

pub(super) fn saved(app: &KetchupApp) -> Vec<u8> {
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

/// A thread names a standard size that a scaled hole no longer has (M8 grown
/// by 1.5 is not M8): Scale refuses a tapped part by name, and the document
/// refuses a Scale feature behind a thread however it arrives. A hole tapped
/// after a Scale is cut into the scaled solid at its true size and stays M8.
#[test]
fn scale_refuses_a_tapped_part_and_a_thread_tapped_after_scale_keeps_its_size() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let part = app.document.current().occurrences().next().unwrap().id();
    let tap_top = |app: &mut KetchupApp, bridge: &mut LiveBridge| {
        evaluate_exact(app);
        let top = face(app, bridge, "plane", [0.0, 0.0, 1.0]);
        let tapped = tap(
            &top,
            vec![centroid(&top)],
            6.8,
            Some(15.0),
            false,
            Some(("M8", 12.0)),
        );
        let report = bridge.execute(app, tapped, false).unwrap();
        assert_eq!(report["published"], true, "{report}");
    };
    let bytes = saved(&app);

    tap_top(&mut app, &mut bridge);
    let (undo, tapped) = (app.undo_step_count(), saved(&app));
    assert!(app.headless_select_occurrence(part));
    assert!(!app.scale_selected(1.5));
    assert_eq!(
        app.action_digest(),
        app.catalog.text("digest-scale-refused-thread")
    );
    assert_eq!(app.undo_step_count(), undo);
    assert!(saved(&app) == tapped);

    let snapshot = app.document.current();
    let definition_id = snapshot.occurrence(part).unwrap().definition_id();
    let target =
        *ketchup_model::exact_product::exact_body_terminal_features(&snapshot, definition_id)
            .unwrap()
            .values()
            .next()
            .unwrap();
    let id = FeatureId(
        snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap()
            + 1,
    );
    let error = app
        .document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: "Scale".to_owned(),
            kind: FeatureKind::Scale {
                target,
                origin_mm: [0.0; 3],
                factors: [1.5; 3],
            },
        }]))
        .err();
    assert!(
        matches!(error, Some(CanonicalError::ScaledHoleThread(_))),
        "{error:?}"
    );
    assert!(saved(&app) == tapped);

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(saved(&app) == bytes);
    assert!(app.headless_select_occurrence(part));
    assert!(app.scale_selected(1.5), "{}", app.action_digest());
    tap_top(&mut app, &mut bridge);
    assert_eq!(
        report(&mut app, &mut bridge, ReportSection::Hardware),
        json!([{"item": "M8×1.25 depth 12", "count": 1, "kind": "tapped_hole"}])
    );
}

/// A component works like SketchUp's: a hole in the shared definition drills
/// every copy; after Make Unique a hole drills only that copy.
#[test]
fn a_hole_drills_every_copy_of_a_component_until_one_is_made_unique() {
    let (mut app, mut bridge) = setup();
    let source = app.document.current().occurrences().next().unwrap().id();
    let copied = edit(AssistantCadEditOperation::Copy {
        selector: ketchup_assistant::sidecar::AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![source.0],
        },
        translation_mm: [5000.0, 0.0, 0.0],
    });
    assert_eq!(
        bridge.execute(&mut app, copied, false).unwrap()["published"],
        true
    );
    let snapshot = app.document.current();
    let [original, copy] = [0, 1].map(|index| {
        let occurrence = snapshot.occurrences().nth(index).unwrap();
        (occurrence.id(), occurrence.name().to_owned())
    });
    let shared = snapshot.occurrence(source).unwrap().definition_id();
    assert_eq!(snapshot.occurrence(copy.0).unwrap().definition_id(), shared);
    evaluate_exact(&mut app);
    let solid = exact_volume(&app, &original.1);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let [x, y, z] = centroid(&top);
    let area = PI * (DIAMETER_MM * 0.5).powi(2);
    let through = area * (z - centroid(&bottom)[2]);
    let hole = area * 8.0;
    let volumes = |app: &KetchupApp| [exact_volume(app, &original.1), exact_volume(app, &copy.1)];
    let close = |volume: f64, expected: f64| (volume - expected).abs() <= 1e-6 * solid;

    // Through, so the top stays the newest upward face (no hole floor).
    let shared_hole = drill(&top, vec![[x, y, z]], None, true);
    let undo = app.undo_step_count();
    assert_eq!(
        bridge.execute(&mut app, shared_hole, false).unwrap()["published"],
        true
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    evaluate_exact(&mut app);
    let [a, b] = volumes(&app);
    assert!(
        close(a, solid - through) && close(b, solid - through),
        "{a} {b}"
    );

    let unique = edit(AssistantCadEditOperation::MakeOccurrenceUnique {
        occurrence_id: copy.0.0,
    });
    let undo = app.undo_step_count();
    assert_eq!(
        bridge.execute(&mut app, unique, false).unwrap()["published"],
        true
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    let own = app
        .document
        .current()
        .occurrence(copy.0)
        .unwrap()
        .definition_id();
    assert_ne!(own, shared);
    evaluate_exact(&mut app);
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    assert_eq!(top["definition_id"].as_u64(), Some(own.0));
    let shared_definition = |app: &KetchupApp| {
        let snapshot = app.document.current();
        format!("{:?}", snapshot.definition(shared).unwrap())
    };
    let before = shared_definition(&app);
    let own_hole = drill(&top, vec![[x + 20.0, y, z]], Some(8.0), false);
    let undo = app.undo_step_count();
    let drilled = bridge.execute(&mut app, own_hole, false);
    assert!(drilled.is_ok(), "{:?}", take_error_details());
    assert_eq!(drilled.unwrap()["published"], true);
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    evaluate_exact(&mut app);
    let [a, b] = volumes(&app);
    assert!(close(a, solid - through), "the original changed: {a}");
    assert!(close(b, solid - through - hole), "the unique copy: {b}");
    assert_eq!(
        shared_definition(&app),
        before,
        "the shared definition changed"
    );
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

/// Review 2026-10-14 (GX-13): with a section open, Hole, Draft and Mirror
/// reached the face the cut hides; they now take the first face it keeps.
#[test]
fn the_hole_and_mirror_tools_never_reach_the_face_a_section_hides() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let snapshot = app.document.current();
    let world = snapshot
        .world_transform_for_occurrence(snapshot.occurrences().next().unwrap().id())
        .unwrap();
    let top = face(&mut app, &mut bridge, "plane", [0.0, 0.0, 1.0]);
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let [x, y, z] = centroid(&top);
    let low = world.transform_point(centroid(&bottom))[2];
    let high = world.transform_point([x, y, z])[2];
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0));
    let [wx, wy, wz] = world.transform_point([x + 10.0, y, z]);
    let click = app.project(Vec3::new(wx, wy, wz), rect);
    app.set_section(crate::app::plane_at([0.0, 0.0, 1.0], (low + high) / 2.0));

    // Mirror's plane: the kept bottom face, seen from inside the cut part.
    let (hit, _) = app.surface_hit_at_screen(click, rect).unwrap();
    assert!(
        (hit.z - low).abs() < 1e-6,
        "{hit:?} is not on the kept face"
    );

    // Hole (and Draft, which picks its face the same way): an 8 mm blind
    // hole drilled from the bottom face, not from the hidden top.
    app.dispatch_command(AppCommand::Hole);
    app.value_box.input = "6 8".to_owned();
    assert!(app.drill_hole_at_screen(click, rect), "{}", app.digest);
    app.finish_window_edit();
    assert_eq!(app.digest, app.catalog.text("digest-hole-created"));
    evaluate_exact(&mut app);
    // The hole's end faces down into it; the hidden top would have put it
    // 8 mm below the top, facing up.
    let end = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    let end_z = world.transform_point(centroid(&end))[2];
    assert!(
        (end_z - (low + 8.0)).abs() < 1e-6,
        "the hole ends at {end_z}, not 8 mm above the bottom {low}"
    );
}
