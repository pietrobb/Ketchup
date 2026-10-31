//! A selected underlay moves and turns with the Move and Rotate tools, clicked in a headless
//! window: one Undo step each, Esc cancels, and a move out of the ground plane or a copy is
//! refused in the status line.
use super::underlay_snapping_tests::imported;
use crate::*;
use egui_kittest::Harness;
use ketchup_model::underlay::snap::UnderlaySnapKind;

pub(super) fn shell() -> Harness<'static, KetchupApp> {
    shell_of(imported())
}

pub(super) fn shell_of(app: KetchupApp) -> Harness<'static, KetchupApp> {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
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

pub(super) fn point(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> Pos2 {
    // A drawing's snaps are indexed in the background after each edit; a click snaps once
    // they are in.
    harness.state().wait_for_underlay_snaps();
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

pub(super) fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
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

/// A double-click on a line of the drawing: selects the whole drawing, so Move and Rotate
/// place it.
pub(super) fn double_click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    // Apart from an earlier click, so the two are not its second and third.
    for _ in 0..40 {
        harness.step();
    }
    click(harness, world);
    click(harness, world);
    assert!(
        harness.state().selection.underlay_objects().is_empty(),
        "{}",
        harness.state().digest
    );
}

pub(super) fn press(harness: &mut Harness<'_, KetchupApp>, key: egui::Key) {
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
    double_click(&mut harness, axes_end);
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
    double_click(&mut harness, axes_end);
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
    double_click(&mut harness, axes_end);
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
    double_click(&mut harness, axes_end);
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

/// Review 2026-10-29 UO-69: a drawing standing as a view, or a plan raised off the ground,
/// was refused by Move and Rotate in the window; it moves in the plane it stands on and
/// turns about the axis it is seen along, one Undo step each.
#[test]
fn a_standing_or_raised_drawing_moves_and_turns_in_its_own_plane() {
    use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};
    let views = [
        (ViewPlane::Front, 0.0, Axis::Y, Axis::Z),
        (ViewPlane::Right, 2500.0, Axis::X, Axis::Y),
        (ViewPlane::Top, 3000.0, Axis::Z, Axis::X),
    ];
    for (plane, at_mm, turn_axis, tilt_axis) in views {
        let mut harness = shell();
        let id = harness
            .state()
            .document
            .current()
            .underlays()
            .next()
            .unwrap()
            .id;
        let view = UnderlayView { plane, at_mm };
        let batch = plan_underlay_view(&harness.state().document.current(), id, view).unwrap();
        harness
            .state_mut()
            .apply_batch_with_work_recovery(&batch)
            .unwrap();
        let at = |app: &KetchupApp, point: [f64; 3]| {
            let underlay = app.document.current().underlay(id).unwrap().clone();
            underlay.world_transform().transform_point(point)
        };
        let before = at(harness.state(), [240.0, 120.0, 0.0]);
        harness.state_mut().select_whole_underlay(id);
        let undo = harness.state().undo_step_count();
        let rect = harness.state().camera.viewport_rect.unwrap();

        // A move picked off the plane keeps to it: the part across it is dropped.
        harness.state_mut().dispatch_command(AppCommand::Move);
        let app = harness.state_mut();
        assert!(
            app.begin_move_drag_at(rect.center(), rect, false),
            "{plane:?}: {}",
            app.digest
        );
        app.move_session_mut().unwrap().delta_mm = Vec3::new(100.0, 50.0, 30.0);
        let drag = app.move_session().unwrap().0.clone();
        assert!(app.commit_move_drag(&drag), "{plane:?}: {}", app.digest);
        let mut moved = [100.0, 50.0, 30.0];
        moved[plane.axes().2] = 0.0;
        let after = at(app, [240.0, 120.0, 0.0]);
        for axis in 0..3 {
            assert!(
                (after[axis] - before[axis] - moved[axis]).abs() < 1e-9,
                "{plane:?}: {after:?}"
            );
        }
        assert_eq!(app.undo_step_count(), undo + 1);
        assert_eq!(app.document.current().underlay(id).unwrap().view, view);
        app.undo();
        assert_eq!(at(harness.state(), [240.0, 120.0, 0.0]), before);

        // A quarter turn about the axis it is seen along keeps it on its plane.
        let app = harness.state_mut();
        app.dispatch_command(AppCommand::Rotate);
        let centre = at(app, [0.0, 0.0, 0.0]);
        assert!(
            app.begin_rotate_drag_at(rect.center(), rect, false),
            "{}",
            app.digest
        );
        app.set_rotate_axis_lock(Some(turn_axis));
        let session = app.rotate_session_mut().unwrap();
        session.centre_mm = Vec3::from(centre);
        session.angle_degrees = 90.0;
        let drag = app.rotate_session().unwrap().0.clone();
        assert!(app.commit_rotate_drag(&drag), "{plane:?}: {}", app.digest);
        let turned = at(app, [240.0, 120.0, 0.0]);
        assert!((turned[plane.axes().2] - before[plane.axes().2]).abs() < 1e-9);
        assert_ne!(turned, before);
        let underlay = app.document.current().underlay(id).unwrap().clone();
        assert_eq!(underlay.view, view);
        app.undo();

        // A turn about another axis would tilt it off its plane: refused by name.
        let digest = harness.state().canonical_digest();
        let app = harness.state_mut();
        app.dispatch_command(AppCommand::Rotate);
        assert!(app.begin_rotate_drag_at(rect.center(), rect, false));
        app.set_rotate_axis_lock(Some(tilt_axis));
        app.rotate_session_mut().unwrap().angle_degrees = 30.0;
        let drag = app.rotate_session().unwrap().0.clone();
        assert!(!app.commit_rotate_drag(&drag));
        assert!(app.digest.contains("plane it stands on"), "{}", app.digest);
        assert_eq!(app.canonical_digest(), digest);
    }
}

/// Review 2026-10-29 UO-69 (capability review): a drawing standing as a view is dragged
/// by the pointer in its own plane, up and down too, seen straight on (Front, Right) and
/// in the iso view, where the pointer only met a horizontal plane: seen straight on the
/// Move did not start off a snap, and in iso the drag never moved it up or down.
#[test]
fn a_standing_drawing_is_dragged_by_the_pointer_in_its_own_plane() {
    use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};
    let cases = [
        (ViewPlane::Front, 0.0, AppCommand::ViewFront),
        (ViewPlane::Right, 2500.0, AppCommand::ViewRight),
        (ViewPlane::Front, 0.0, AppCommand::ViewIso),
        (ViewPlane::Right, 2500.0, AppCommand::ViewIso),
    ];
    for (plane, at_mm, camera) in cases {
        let mut harness = shell();
        let id = harness
            .state()
            .document
            .current()
            .underlays()
            .next()
            .unwrap()
            .id;
        let view = UnderlayView { plane, at_mm };
        let batch = plan_underlay_view(&harness.state().document.current(), id, view).unwrap();
        harness
            .state_mut()
            .apply_batch_with_work_recovery(&batch)
            .unwrap();
        harness.state_mut().dispatch_command(camera);
        harness.step();
        let at = |app: &KetchupApp, point: [f64; 3]| {
            let underlay = app.document.current().underlay(id).unwrap().clone();
            Vec3::from(underlay.world_transform().transform_point(point))
        };
        let (from, to) = ([100.0, 60.0, 0.0], [100.0, 260.0, 0.0]);
        let shown = |app: &KetchupApp| {
            let rect = app.camera.viewport_rect.unwrap().shrink(20.0);
            [from, to].into_iter().all(|point| {
                app.viewport_position(at(app, point))
                    .is_some_and(|screen| rect.contains(screen))
            })
        };
        for _ in 0..20 {
            if shown(harness.state()) {
                break;
            }
            harness.state_mut().dispatch_command(AppCommand::ZoomOut);
            harness.step();
        }
        let app = harness.state_mut();
        assert!(shown(app), "{plane:?} {camera:?}: off screen");
        let rect = app.camera.viewport_rect.unwrap();
        let (start, end) = (at(app, from), at(app, to));
        let start_screen = app.viewport_position(start).unwrap();
        let end_screen = app.viewport_position(end).unwrap();
        app.select_whole_underlay(id);
        app.dispatch_command(AppCommand::Move);
        app.hover.snap = None;
        assert!(
            app.begin_move_drag_at(start_screen, rect, false),
            "{plane:?} {camera:?}: {}",
            app.digest
        );
        let mut drag = app.move_session().unwrap().0.clone();
        app.advance_move(&mut drag, end_screen, rect, true);
        let expected = end - start;
        assert!(
            length(drag.delta_mm - expected) < 1e-3,
            "{plane:?} {camera:?}: {:?} is not {expected:?}",
            drag.delta_mm
        );
        assert!(app.commit_move_drag(&drag), "{plane:?}: {}", app.digest);
        let moved = at(app, from);
        assert!(
            length(moved - end) < 1e-3,
            "{plane:?} {camera:?}: {moved:?}"
        );
        assert_eq!(app.document.current().underlay(id).unwrap().view, view);
    }
}

#[test]
fn an_underlay_move_out_of_the_ground_plane_or_a_copy_is_refused_by_name() {
    let mut harness = shell();
    double_click(&mut harness, Vec3::new(240.0, 120.0, 0.0));
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
