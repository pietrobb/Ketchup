//! Clicked in a headless window: a click on a line of an underlay picks the object that drew
//! it, Shift+click picks more or drops one, Delete takes them out of the drawing in one Undo
//! step, and the drawing no longer snaps where they were.
use super::underlay_transform_tests::{click, double_click, press, shell};
use crate::*;
use egui_kittest::Harness;
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape};
use ketchup_model::underlay::snap::UnderlaySnapKind;

fn shift_click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    harness.input_mut().modifiers = egui::Modifiers::SHIFT;
    click(harness, world);
    harness.input_mut().modifiers = egui::Modifiers::NONE;
}

fn end_at(harness: &mut Harness<'_, KetchupApp>, at: [f64; 2]) -> bool {
    let app = harness.state();
    app.wait_for_underlay_snaps();
    let snapshot = app.document.current();
    app.underlay_snaps_near(&snapshot, at, 0.01)
        .0
        .iter()
        .any(|(_, snap)| {
            snap.kind == UnderlaySnapKind::Endpoint
                && (snap.point_mm[0] - at[0]).hypot(snap.point_mm[1] - at[1]) < 1e-6
        })
}

fn picked(harness: &Harness<'_, KetchupApp>) -> usize {
    harness.state().selection.underlay_objects().len()
}

#[test]
fn clicked_objects_of_a_drawing_are_picked_and_deleted_in_one_undo_step() {
    let mut harness = shell();
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();

    // The axes line: the underlay is selected with that one object picked.
    click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
    assert_eq!(harness.state().selected_underlay(), Some(id));
    assert_eq!(picked(&harness), 1);
    let app = harness.state();
    assert!(
        app.digest.contains("LINE") && app.digest.contains("axes"),
        "{}",
        app.digest
    );

    // Shift adds the wall line and the arc; Shift on the axes line again drops it.
    shift_click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    shift_click(&mut harness, Vec3::new(275.355_339, 34.644_661, 0.0));
    assert_eq!(picked(&harness), 3, "{}", harness.state().digest);
    for _ in 0..40 {
        harness.step();
    }
    shift_click(&mut harness, Vec3::new(180.0, 120.0, 0.0));
    assert_eq!(picked(&harness), 2, "{}", harness.state().digest);
    // A plain click picks one again.
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    assert_eq!(picked(&harness), 1);
    shift_click(&mut harness, Vec3::new(275.355_339, 34.644_661, 0.0));
    assert_eq!(picked(&harness), 2);
    assert_eq!(
        harness.state().canonical_digest(),
        digest,
        "picking changes nothing"
    );

    press(&mut harness, egui::Key::Delete);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert_eq!(app.selected_underlay(), Some(id));
    assert_eq!(picked(&harness), 0);
    assert!(!end_at(&mut harness, [40.0, 20.0]), "the wall is gone");
    assert!(!end_at(&mut harness, [290.0, 70.0]), "the arc is gone");
    assert!(end_at(&mut harness, [240.0, 120.0]), "the axes line stays");

    harness.state_mut().undo();
    assert_eq!(harness.state().canonical_digest(), digest);
    assert!(end_at(&mut harness, [40.0, 20.0]));
    assert!(end_at(&mut harness, [290.0, 70.0]));
}

/// 2D-2: Move and Rotate on picked objects of a drawing move those objects, not the
/// drawing, one Undo step each; with Ctrl the turn adds a copy and keeps the original.
#[test]
fn move_and_rotate_on_picked_objects_of_a_drawing_edit_only_them() {
    let mut harness = shell();
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    let wall_end = Vec3::new(240.0, 20.0, 0.0);

    // The wall line alone, its end moved onto the box's top corner: in plan by (-140, 40).
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    assert_eq!(picked(&harness), 1);
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, wall_end);
    click(&mut harness, Vec3::new(100.0, 60.0, 20.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert_eq!(app.selected_underlay(), Some(id));
    assert_eq!(picked(&harness), 1, "the moved wall stays picked");
    assert!(end_at(&mut harness, [100.0, 60.0]), "the wall moved");
    assert!(end_at(&mut harness, [-100.0, 60.0]));
    assert!(!end_at(&mut harness, [40.0, 20.0]));
    assert!(
        end_at(&mut harness, [240.0, 120.0]),
        "the axes line stays where it was"
    );
    let placement = harness
        .state()
        .document
        .current()
        .underlay(id)
        .unwrap()
        .placement;
    assert_eq!(*placement.matrix(), *Transform::identity().matrix());

    harness.state_mut().undo();
    assert_eq!(harness.state().canonical_digest(), digest);
    assert!(end_at(&mut harness, [40.0, 20.0]));

    // Ctrl+Rotate about the wall's end, from its middle a quarter turn: a turned copy
    // reaches down to (240, -180) and the wall stays.
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    harness.state_mut().dispatch_command(AppCommand::Rotate);
    harness.input_mut().modifiers = egui::Modifiers::COMMAND;
    click(&mut harness, wall_end);
    harness.input_mut().modifiers = egui::Modifiers::NONE;
    click(&mut harness, Vec3::new(140.0, 20.0, 0.0));
    click(&mut harness, Vec3::new(240.0, -80.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(end_at(&mut harness, [240.0, -180.0]), "the turned copy");
    assert!(end_at(&mut harness, [40.0, 20.0]), "the wall stays");

    // A double-click selects the whole drawing again: no object is picked.
    harness.state_mut().dispatch_command(AppCommand::Select);
    harness.step();
    double_click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
    assert_eq!(harness.state().selected_underlay(), Some(id));
}

fn drawn_shapes(harness: &Harness<'_, KetchupApp>) -> Vec<(String, DrawnShape)> {
    let snapshot = harness.state().document.current();
    let underlay = snapshot.underlays().next().unwrap();
    underlay
        .edits
        .added
        .iter()
        .filter_map(|added| match added {
            AddedObject::Drawn { layer, shape } => Some((layer.clone(), shape.clone())),
            AddedObject::Copy { .. } => None,
        })
        .collect()
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6
}

/// 2D-2: with Draw into drawing on, Line, Circle, Rectangle and Arc draw into the drawing,
/// on the layer of the object picked in it, one Undo step each and nothing in the model;
/// turned off, they draw into the model again.
#[test]
fn the_drawing_tools_draw_into_the_drawing_on_the_picked_objects_layer() {
    let mut harness = shell();
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    let definitions = harness.state().document.current().definitions().count();
    let [axes_end, wall_end, wall_start] = [[240.0, 120.0], [240.0, 20.0], [40.0, 20.0]];
    let at = |[x, y]: [f64; 2]| Vec3::new(x, y, 0.0);

    // The axes line picked: drawing goes on its layer, with the Line tool.
    click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
    harness
        .state_mut()
        .dispatch_command(AppCommand::DrawIntoUnderlay);
    assert_eq!(harness.state().active_tool, ActiveTool::Line);
    assert!(
        harness.state().digest.contains("axes"),
        "{}",
        harness.state().digest
    );

    click(&mut harness, at(axes_end));
    click(&mut harness, at(wall_end));
    press(&mut harness, egui::Key::Escape);
    harness.state_mut().dispatch_command(AppCommand::Circle);
    click(&mut harness, at(wall_end));
    click(&mut harness, at(wall_start));
    harness.state_mut().dispatch_command(AppCommand::Rectangle);
    click(&mut harness, at(wall_start));
    click(&mut harness, at(axes_end));
    harness.state_mut().dispatch_command(AppCommand::Arc);
    click(&mut harness, at(wall_start));
    click(&mut harness, at(wall_end));
    click(&mut harness, at(axes_end));

    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 4, "{}", app.digest);
    assert_eq!(app.document.current().definitions().count(), definitions);
    let shapes = drawn_shapes(&harness);
    assert_eq!(shapes.len(), 4, "{shapes:?}");
    assert!(
        shapes.iter().all(|(layer, _)| layer == "axes"),
        "{shapes:?}"
    );
    match &shapes[0].1 {
        DrawnShape::Polyline { points_mm, closed } => {
            assert!(!closed && points_mm.len() == 2);
            assert!(near(points_mm[0], axes_end) && near(points_mm[1], wall_end));
        }
        other => panic!("{other:?}"),
    }
    match &shapes[1].1 {
        DrawnShape::Circle {
            center_mm,
            radius_mm,
        } => assert!(near(*center_mm, wall_end) && (radius_mm - 200.0).abs() < 1e-6),
        other => panic!("{other:?}"),
    }
    match &shapes[2].1 {
        DrawnShape::Polyline { points_mm, closed } => {
            assert!(*closed);
            let corners = [wall_start, [240.0, 20.0], axes_end, [40.0, 120.0]];
            assert!(points_mm.iter().zip(corners).all(|(a, b)| near(*a, b)));
        }
        other => panic!("{other:?}"),
    }
    match &shapes[3].1 {
        // Through (40, 20), (240, 120) and (240, 20): about (140, 70), counterclockwise
        // from the end at (240, 20) over the top to (40, 20).
        DrawnShape::Arc {
            center_mm,
            radius_mm,
            start_deg,
            end_deg,
        } => {
            assert!(near(*center_mm, [140.0, 70.0]));
            assert!((radius_mm - 100.0_f64.hypot(50.0)).abs() < 1e-6);
            assert!((start_deg - (-50.0_f64).atan2(100.0).to_degrees()).abs() < 1e-6);
            assert!((end_deg - (-50.0_f64).atan2(-100.0).to_degrees()).abs() < 1e-6);
        }
        other => panic!("{other:?}"),
    }
    assert!(end_at(&mut harness, [40.0, 120.0]), "the rectangle snaps");

    harness.state_mut().dispatch_command(AppCommand::Select);
    for _ in 0..4 {
        harness.state_mut().undo();
    }
    assert_eq!(harness.state().canonical_digest(), digest);

    // Off again: the Line tool draws into the model.
    harness
        .state_mut()
        .dispatch_command(AppCommand::DrawIntoUnderlay);
    assert!(harness.state().gesture.underlay_draw.is_none());
    harness.state_mut().dispatch_command(AppCommand::Line);
    click(&mut harness, at(axes_end));
    click(&mut harness, at(wall_end));
    assert_eq!(
        harness.state().document.current().definitions().count(),
        definitions + 1
    );
    assert!(drawn_shapes(&harness).is_empty());
}

/// 2D-2: Trim, Extend and Offset drawing line, clicked: Trim cuts the wall back to the
/// line crossing it, Extend takes a short line up to the axes line, Offset draws a
/// parallel line a typed distance away on the side clicked, and Mirror flips a picked
/// line across a line of two clicks; one Undo step each.
#[test]
fn trim_extend_offset_and_mirror_edit_lines_of_a_drawing() {
    let mut harness = shell();
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    let digest = harness.state().canonical_digest();
    let line = |points: [[f64; 2]; 2]| DrawnShape::Polyline {
        points_mm: points.to_vec(),
        closed: false,
    };
    // Across the wall and the axes line at x = 140, and a short line at x = 180 between.
    harness
        .state_mut()
        .draw_into_underlay(
            id,
            "axes",
            vec![
                line([[140.0, 0.0], [140.0, 140.0]]),
                line([[180.0, 40.0], [180.0, 100.0]]),
            ],
        )
        .unwrap();
    let undo = harness.state().undo_step_count();

    harness
        .state_mut()
        .dispatch_command(AppCommand::TrimUnderlayLine);
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    // The drawing's line at x = 60 crosses the wall too: the piece between goes.
    assert!(
        end_at(&mut harness, [60.0, 20.0]),
        "the wall is cut at x = 60"
    );
    assert!(
        end_at(&mut harness, [140.0, 20.0]),
        "the wall ends at x = 140"
    );
    assert!(end_at(&mut harness, [240.0, 20.0]));
    assert!(end_at(&mut harness, [40.0, 20.0]));

    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtendUnderlayLine);
    click(&mut harness, Vec3::new(180.0, 90.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 2, "{}", app.digest);
    assert!(end_at(&mut harness, [180.0, 120.0]), "up to the axes line");
    assert!(end_at(&mut harness, [180.0, 40.0]), "the other end stays");

    harness
        .state_mut()
        .dispatch_command(AppCommand::OffsetUnderlayLine);
    click(&mut harness, Vec3::new(200.0, 20.0, 0.0));
    assert_eq!(
        harness.state().undo_step_count(),
        undo + 2,
        "a pick changes nothing"
    );
    harness.state_mut().value_box.input = "30".to_owned();
    click(&mut harness, Vec3::new(210.0, 70.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 3, "{}", app.digest);
    assert!(end_at(&mut harness, [140.0, 50.0]), "30 above the wall");
    assert!(end_at(&mut harness, [240.0, 50.0]));
    assert!(end_at(&mut harness, [140.0, 20.0]), "the wall stays");

    // A click off every line of the drawing changes nothing.
    harness
        .state_mut()
        .dispatch_command(AppCommand::TrimUnderlayLine);
    click(&mut harness, Vec3::new(60.0, 260.0, 0.0));
    assert_eq!(harness.state().undo_step_count(), undo + 3);

    // Mirror with the short line picked: across x = 240, through the ends of the wall and
    // its offset, it goes to x = 300.
    harness.state_mut().dispatch_command(AppCommand::Select);
    click(&mut harness, Vec3::new(180.0, 60.0, 0.0));
    assert_eq!(picked(&harness), 1, "{}", harness.state().digest);
    harness.state_mut().dispatch_command(AppCommand::Mirror);
    assert_eq!(harness.state().active_tool, ActiveTool::Mirror);
    click(&mut harness, Vec3::new(240.0, 20.0, 0.0));
    click(&mut harness, Vec3::new(240.0, 50.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 4, "{}", app.digest);
    assert!(end_at(&mut harness, [300.0, 120.0]), "mirrored");
    assert!(end_at(&mut harness, [300.0, 40.0]));
    assert!(!end_at(&mut harness, [180.0, 40.0]), "moved, not copied");

    harness.state_mut().dispatch_command(AppCommand::Select);
    for _ in 0..5 {
        harness.state_mut().undo();
    }
    assert_eq!(harness.state().canonical_digest(), digest);
}
