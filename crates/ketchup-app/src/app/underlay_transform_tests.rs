//! A selected underlay moves and turns with the Move and Rotate tools, clicked in a headless
//! window: one Undo step each, Esc cancels, and a move out of the ground plane or a copy is
//! refused in the status line.
use super::underlay_snapping_tests::imported;
use crate::*;
use egui_kittest::Harness;
use ketchup_model::underlay::snap::UnderlaySnapKind;

fn shell() -> Harness<'static, KetchupApp> {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), imported());
    harness.step();
    harness.state_mut().dispatch_command(AppCommand::ViewTop);
    harness.step();
    // Out until every point the tests click is in the viewport.
    for _ in 0..20 {
        let app = harness.state();
        let rect = app.camera.viewport_rect.unwrap().shrink(20.0);
        let shown = [[0.0, 0.0, 0.0], [300.0, 300.0, 0.0], [240.0, -180.0, 0.0]]
            .into_iter()
            .all(|[x, y, z]| {
                app.viewport_position(Vec3::new(x, y, z))
                    .is_some_and(|screen| rect.contains(screen))
            });
        if shown {
            return harness;
        }
        harness.state_mut().dispatch_command(AppCommand::ZoomOut);
        harness.step();
    }
    panic!("the underlay never came into view");
}

fn point(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> Pos2 {
    let screen = harness.state().viewport_position(world).unwrap() + Vec2::new(2.0, 1.0);
    let rect = harness.state().camera.viewport_rect.unwrap();
    assert!(
        rect.contains(screen),
        "{world:?} at {screen:?} is off {rect:?}"
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(screen));
    harness.step();
    screen
}

fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    let screen = point(harness, world);
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: screen,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
}

fn press(harness: &mut Harness<'_, KetchupApp>, key: egui::Key) {
    harness.key_press(key);
    harness.step();
}

/// Whether an end of a line of the underlay lies within 1e-6 mm of `at`.
fn underlay_end_at(app: &KetchupApp, at: [f64; 2]) -> bool {
    let snapshot = app.document.current();
    app.underlay_snaps_near(&snapshot, at, 0.01)
        .0
        .iter()
        .any(|(_, snap)| {
            snap.kind == UnderlaySnapKind::Endpoint
                && (snap.point_mm[0] - at[0]).hypot(snap.point_mm[1] - at[1]) < 1e-6
        })
}

#[test]
fn move_and_rotate_place_a_clicked_underlay_in_one_undo_step_each() {
    let mut harness = shell();
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    let axes_end = Vec3::new(240.0, 120.0, 0.0);
    let wall_end = Vec3::new(240.0, 20.0, 0.0);

    // A click on a line of the underlay, with no part under the pointer, selects it.
    click(&mut harness, axes_end);
    assert_eq!(harness.state().selected_underlay(), Some(id));
    assert!(harness.state().selected_instance_paths().is_empty());
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();

    // Move: from the axes line's end onto the top corner of the box; the underlay moves in
    // plan, so that end lands at the corner's X and Y.
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, axes_end);
    point(&mut harness, Vec3::new(300.0, 300.0, 0.0));
    press(&mut harness, egui::Key::Escape);
    assert!(!harness.state().transform_gesture_active());
    assert_eq!(harness.state().canonical_digest(), digest);

    click(&mut harness, axes_end);
    click(&mut harness, Vec3::new(100.0, 60.0, 20.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(underlay_end_at(app, [100.0, 60.0]), "{}", app.digest);
    assert!(!underlay_end_at(app, [240.0, 120.0]));
    assert_eq!(app.selected_underlay(), Some(id));

    harness.state_mut().undo();
    assert!(underlay_end_at(harness.state(), [240.0, 120.0]));
    assert_eq!(harness.state().canonical_digest(), digest);

    // Rotate: about the wall's end, from the axes line's end to the wall's middle, a
    // quarter turn: that end lands on the wall's old middle.
    harness.state_mut().dispatch_command(AppCommand::Rotate);
    click(&mut harness, wall_end);
    click(&mut harness, axes_end);
    click(&mut harness, Vec3::new(140.0, 20.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(underlay_end_at(app, [140.0, 20.0]), "{}", app.digest);
    assert!(underlay_end_at(app, [240.0, -180.0]));
    let placement = app.document.current().underlay(id).unwrap().placement;
    let matrix = placement.matrix();
    assert_eq!(
        [
            matrix[2], matrix[6], matrix[8], matrix[9], matrix[10], matrix[11]
        ],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
    );
}

/// Review 2026-10-21 P3: a move onto a model point above the ground (z > 0) lands the
/// underlay where that point lies in plan, and says how far it moved there.
#[test]
fn an_underlay_moved_onto_a_point_above_the_ground_lands_in_plan() {
    let mut harness = shell();
    let axes_end = Vec3::new(240.0, 120.0, 0.0);
    click(&mut harness, axes_end);
    assert!(harness.state().selected_underlay().is_some());
    let undo = harness.state().undo_step_count();
    harness.state_mut().dispatch_command(AppCommand::ViewIso);
    harness.step();
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, axes_end);
    let corner = Vec3::new(100.0, 60.0, 20.0);
    point(&mut harness, corner);
    let app = harness.state();
    let snapped = app.hover.snap.as_ref().map(|snap| snap.position_mm);
    assert_eq!(snapped, Some(corner), "the box's top corner is snapped");
    click(&mut harness, corner);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(underlay_end_at(app, [100.0, 60.0]), "{}", app.digest);
    // The status line names how far it moved in plan, not toward the point above it.
    let in_plan = format_height(140.0_f64.hypot(60.0));
    assert!(app.digest.contains(&in_plan), "{in_plan} in {}", app.digest);
}

/// Review 2026-10-24 UI-24: a move its placement refuses, such as one past the model's
/// range, found no plan and said the preview was stale; the status line names why.
#[test]
fn an_underlay_move_past_the_models_range_is_refused_by_its_reason() {
    let mut harness = shell();
    let axes_end = Vec3::new(240.0, 120.0, 0.0);
    click(&mut harness, axes_end);
    assert!(harness.state().selected_underlay().is_some());
    let digest = harness.state().canonical_digest();
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, axes_end);
    let app = harness.state_mut();
    app.move_session_mut().unwrap().delta_mm = Vec3::new(1_500_000.0, 0.0, 0.0);
    let drag = app.move_session().unwrap().0.clone();
    assert!(!app.commit_move_drag(&drag));
    assert!(
        app.digest.contains("its origin stays within"),
        "{}",
        app.digest
    );
    assert_eq!(app.canonical_digest(), digest);
}

/// Review 2026-10-24 UI-25: an underlay being moved is drawn where the pointer takes it,
/// yet the pointer snapped to its lines where they were before the move, where nothing is
/// drawn.
#[test]
fn an_underlay_being_moved_does_not_snap_to_where_it_was() {
    let mut harness = shell();
    let axes_end = Vec3::new(240.0, 120.0, 0.0);
    let wall_end = Vec3::new(240.0, 20.0, 0.0);
    click(&mut harness, axes_end);
    point(&mut harness, wall_end);
    let snapped = |app: &KetchupApp| app.hover.snap.as_ref().map(|snap| snap.position_mm);
    assert_eq!(
        snapped(harness.state()),
        Some(wall_end),
        "snaps before a move"
    );

    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, axes_end);
    point(&mut harness, wall_end);
    let app = harness.state();
    assert!(app.transform_gesture_active());
    assert_ne!(snapped(app), Some(wall_end));
}

#[test]
fn an_underlay_move_out_of_the_ground_plane_or_a_copy_is_refused_by_name() {
    let mut harness = shell();
    click(&mut harness, Vec3::new(240.0, 120.0, 0.0));
    assert!(harness.state().selected_underlay().is_some());
    let digest = harness.state().canonical_digest();

    harness.state_mut().dispatch_command(AppCommand::Rotate);
    click(&mut harness, Vec3::new(240.0, 20.0, 0.0));
    harness.state_mut().set_rotate_axis_lock(Some(Axis::X));
    harness
        .state_mut()
        .rotate_session_mut()
        .unwrap()
        .angle_degrees = 30.0;
    let drag = harness.state().rotate_session().unwrap().0.clone();
    assert!(!harness.state_mut().commit_rotate_drag(&drag));
    let app = harness.state();
    assert!(app.digest.contains("ground plane"), "{}", app.digest);
    assert_eq!(app.canonical_digest(), digest);

    let rect = harness.state().camera.viewport_rect.unwrap();
    let app = harness.state_mut();
    app.cancel_transform_session();
    assert!(!app.begin_move_drag_at(rect.center(), rect, true));
    assert_eq!(
        app.digest,
        app.catalog.format(
            "digest-underlay-edit-failed",
            &BTreeMap::from([("reason", app.catalog.text("error-underlay-copy"))]),
        )
    );
    assert!(!app.transform_gesture_active());

    // Esc in Select drops the underlay from the selection.
    app.dispatch_command(AppCommand::Select);
    press(&mut harness, egui::Key::Escape);
    assert_eq!(harness.state().selected_underlay(), None);
    assert_eq!(harness.state().canonical_digest(), digest);
}
