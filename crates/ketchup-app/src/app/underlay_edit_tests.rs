//! Clicked in a headless window: a click on a line of an underlay picks the object that drew
//! it, Shift+click picks more or drops one, Delete takes them out of the drawing in one Undo
//! step, and the drawing no longer snaps where they were.
use super::underlay_snapping_tests::{imported_alone_with, imported_with};
use super::underlay_transform_tests::{click, double_click, point, press, shell, shell_of};
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

/// A room label and a solid hatch with no boundary object draw no line to click: a click
/// on the label's letters or inside the hatch picks them, and Delete takes them out.
#[test]
fn a_clicked_text_or_hatch_of_a_drawing_is_picked_and_deleted() {
    let mut harness = shell_of(imported_with(concat!(
        "0\nTEXT\n8\nlabels\n10\n120\n20\n60\n40\n10\n1\n1.02 WC\n",
        "0\nHATCH\n8\nfloor\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n3\n",
        "72\n0\n73\n1\n93\n4\n10\n100\n20\n180\n10\n180\n20\n180\n10\n180\n20\n240\n",
        "10\n100\n20\n240\n97\n0\n75\n0\n76\n1\n98\n0\n",
    )));
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    let label = Vec3::new(130.0, 64.0, 0.0);
    let floor = Vec3::new(140.0, 210.0, 0.0);

    click(&mut harness, label);
    assert_eq!(picked(&harness), 1, "{}", harness.state().digest);
    let app = harness.state();
    assert!(
        app.digest.contains("TEXT") && app.digest.contains("labels"),
        "{}",
        app.digest
    );
    click(&mut harness, floor);
    assert_eq!(picked(&harness), 1, "{}", harness.state().digest);
    let app = harness.state();
    assert!(
        app.digest.contains("HATCH") && app.digest.contains("floor"),
        "{}",
        app.digest
    );
    shift_click(&mut harness, label);
    assert_eq!(picked(&harness), 2, "{}", harness.state().digest);
    // Nothing there: the picks are dropped.
    click(&mut harness, Vec3::new(260.0, 250.0, 0.0));
    assert_eq!(picked(&harness), 0, "{}", harness.state().digest);

    click(&mut harness, floor);
    shift_click(&mut harness, label);
    press(&mut harness, egui::Key::Delete);
    assert_eq!(harness.state().undo_step_count(), undo + 1);
    for at in [label, floor] {
        click(&mut harness, at);
        assert_eq!(picked(&harness), 0, "deleted, {}", harness.state().digest);
    }

    harness.state_mut().undo();
    assert_eq!(harness.state().canonical_digest(), digest);
    click(&mut harness, label);
    assert_eq!(picked(&harness), 1, "back after Undo");
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

/// Review 2026-10-29 UI-63: moving objects picked in a drawing took the whole drawing out
/// of snapping, so a door could not be put on a wall of the same plan, and turning them
/// snapped to where they were. Only the lines of the picked objects are left out, once
/// they move or turn.
#[test]
fn picked_objects_moved_or_turned_snap_to_the_rest_of_their_drawing_only() {
    let mut harness = shell();
    let undo = harness.state().undo_step_count();
    let wall_end = Vec3::new(240.0, 20.0, 0.0);

    // The wall's end onto the top end of the line across, clicked a little off it: the
    // wall then runs from (-140, 150).
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, wall_end);
    click(&mut harness, Vec3::new(61.0, 149.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(
        end_at(&mut harness, [-140.0, 150.0]),
        "snapped to the line across"
    );
    harness.state_mut().undo();

    // A turn about the wall's end once its arm is set: the pointer by the wall's own other
    // end offers no snap where the wall was, by the line across it still does.
    harness.state_mut().dispatch_command(AppCommand::Select);
    harness.step();
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    assert_eq!(picked(&harness), 1);
    harness.state_mut().dispatch_command(AppCommand::Rotate);
    click(&mut harness, wall_end);
    click(&mut harness, Vec3::new(200.0, 120.0, 0.0));
    let snap_at = |harness: &mut Harness<'_, KetchupApp>, world: Vec3| {
        point(harness, world);
        harness
            .state()
            .hover
            .snap
            .as_ref()
            .map(|snap| snap.position_mm)
    };
    assert_ne!(
        snap_at(&mut harness, Vec3::new(41.0, 21.0, 0.0)),
        Some(Vec3::new(40.0, 20.0, 0.0)),
        "snapped to its old end"
    );
    assert_eq!(
        snap_at(&mut harness, Vec3::new(61.0, 149.0, 0.0)),
        Some(Vec3::new(60.0, 150.0, 0.0))
    );
    press(&mut harness, egui::Key::Escape);
    assert_eq!(harness.state().undo_step_count(), undo);
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
            AddedObject::Copy { .. } | AddedObject::Stretched { .. } => None,
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

/// UI-64: a drawing being drawn into stood up as a view, from Tools > Drawing view or by
/// the AI, is drawn into no more: drawing into it is off, said so, and the Line tool's
/// clicks on the ground add nothing to the drawing standing elsewhere. Before, the shapes
/// went into it and appeared on its vertical plane, away from where they were clicked.
#[test]
fn a_drawing_stood_up_as_a_view_is_drawn_into_no_more() {
    use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};
    let at = |[x, y]: [f64; 2]| Vec3::new(x, y, 0.0);
    for by_the_window in [true, false] {
        let mut harness = shell();
        click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
        harness
            .state_mut()
            .dispatch_command(AppCommand::DrawIntoUnderlay);
        let id = harness.state().underlay_draw_target().unwrap().underlay;
        if by_the_window {
            assert!(harness.state_mut().set_underlay_view(id, ViewPlane::Front));
            let app = harness.state();
            assert!(app.gesture.underlay_draw.is_none(), "{}", app.digest);
            assert!(app.digest.contains("Front view"), "{}", app.digest);
        } else {
            let snapshot = harness.state().document.current();
            let view = UnderlayView {
                plane: ViewPlane::Front,
                at_mm: 0.0,
            };
            let batch = plan_underlay_view(&snapshot, id, view).unwrap();
            harness
                .state_mut()
                .apply_batch_with_work_recovery(&batch)
                .unwrap();
        }
        assert_eq!(harness.state().underlay_draw_target(), None);
        harness.state_mut().dispatch_command(AppCommand::Line);
        click(&mut harness, at([240.0, 120.0]));
        click(&mut harness, at([240.0, 20.0]));
        assert!(drawn_shapes(&harness).is_empty(), "{by_the_window}");
    }
}

fn type_text(harness: &mut Harness<'_, KetchupApp>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    harness.step();
    harness.step();
}

/// `text` typed as winit sends it: a letter or digit key goes down before its text in the
/// same frame and comes up in the next.
pub(super) fn type_keys(harness: &mut Harness<'_, KetchupApp>, text: &str) {
    for character in text.chars() {
        let key = egui::Key::from_name(&character.to_ascii_uppercase().to_string());
        let modifiers = egui::Modifiers {
            shift: character.is_uppercase(),
            ..egui::Modifiers::NONE
        };
        let key_event = |pressed| egui::Event::Key {
            key: key.unwrap(),
            physical_key: key,
            pressed,
            repeat: false,
            modifiers,
        };
        let events = &mut harness.input_mut().events;
        if key.is_some() {
            events.push(key_event(true));
        }
        events.push(egui::Event::Text(character.to_string()));
        harness.step();
        if key.is_some() {
            harness.input_mut().events.push(key_event(false));
        }
        harness.step();
    }
}

/// Review 2026-10-29 UI-60: the key of a text's first letter went down before the text
/// and was left to the canvas, so "Spálňa" switched to Spline and "Chodba" to Circle. The
/// key of the letter that begins the text is the text's.
#[test]
fn a_text_beginning_with_a_tool_key_is_typed_not_its_tool() {
    let mut harness = shell();
    click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
    for word in ["Spálňa", "Obývačka", "Chodba", "Terasa", "Altán", "Lino"] {
        harness
            .state_mut()
            .dispatch_command(AppCommand::TextIntoUnderlay);
        type_keys(&mut harness, word);
        assert_eq!(
            harness.state().active_tool,
            ActiveTool::UnderlayText,
            "{word}"
        );
        assert_eq!(harness.state().value_box.input, word);
        press(&mut harness, egui::Key::Escape);
    }
}

/// 2D-2: Text into drawing, typed and clicked: the text typed (commas and all) goes on the
/// picked object's layer where clicked, as high as typed or as the second click is from
/// the first; one Undo step each, and a click on its letters picks it.
#[test]
fn the_text_tool_writes_a_typed_text_into_the_drawing() {
    let mut harness = shell();
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    let scale = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .scale();

    click(&mut harness, Vec3::new(100.0, 120.0, 0.0));
    harness
        .state_mut()
        .dispatch_command(AppCommand::TextIntoUnderlay);
    assert_eq!(harness.state().active_tool, ActiveTool::UnderlayText);
    // A click before any text places nothing.
    click(&mut harness, Vec3::new(140.0, 160.0, 0.0));
    assert_eq!(harness.state().undo_step_count(), undo);
    assert!(harness.state().gesture.underlay_text.is_none());

    type_text(&mut harness, "1.02 WC, bath");
    assert_eq!(harness.state().value_box.input, "1.02 WC, bath");
    click(&mut harness, Vec3::new(240.0, 120.0, 0.0));
    assert_eq!(harness.state().undo_step_count(), undo, "the start only");
    type_text(&mut harness, "20");
    press(&mut harness, egui::Key::Enter);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);

    // The second text's height is clicked: up the line at x = 60, 150 long.
    type_text(&mut harness, "A");
    click(&mut harness, Vec3::new(60.0, 0.0, 0.0));
    click(&mut harness, Vec3::new(60.0, 150.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 2, "{}", app.digest);

    let shapes = drawn_shapes(&harness);
    assert_eq!(shapes.len(), 2, "{shapes:?}");
    for ((layer, shape), (expected, at, height)) in shapes.iter().zip([
        ("1.02 WC, bath", [240.0, 120.0], 20.0),
        ("A", [60.0, 0.0], 150.0),
    ]) {
        assert_eq!(layer, "axes");
        match shape {
            DrawnShape::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => {
                assert_eq!(text, expected);
                assert!(near(*anchor_mm, at), "{anchor_mm:?}");
                assert!((height_mm * scale - height).abs() < 1e-6, "{height_mm}");
                assert_eq!(*rotation_deg, 0.0);
            }
            other => panic!("{other:?}"),
        }
    }

    // A click on its letters picks the new text.
    harness.state_mut().dispatch_command(AppCommand::Select);
    click(&mut harness, Vec3::new(265.0, 130.0, 0.0));
    assert_eq!(picked(&harness), 1, "{}", harness.state().digest);
    assert!(
        harness.state().digest.contains("TEXT"),
        "{}",
        harness.state().digest
    );

    for _ in 0..2 {
        harness.state_mut().undo();
    }
    assert_eq!(harness.state().canonical_digest(), digest);
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
            crate::app::underlay_edit::FindRegions::Here,
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

/// Tools > Export drawing to DXF, clicked in a headless window: the wall clicked and
/// deleted is left out of the DXF written, the axes line stays as the file had it, and the
/// file the drawing was imported from is unchanged.
#[test]
fn tools_export_drawing_writes_the_edited_drawing_as_dxf() {
    const PLAN: &str = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nLINE\n5\nA1\n8\nwalls\n10\n40\n20\n20\n11\n240\n21\n20\n",
        "0\nLINE\n5\nA2\n8\naxes\n10\n40\n20\n120\n11\n240\n21\n120\n",
        "0\nENDSEC\n0\nEOF\n",
    );
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dxf");
    let output = directory.path().join("edited.dxf");
    std::fs::write(&source, PLAN).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_export(&output);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let mut harness = shell_of(app);

    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    press(&mut harness, egui::Key::Delete);
    assert!(
        harness
            .state()
            .command_enabled(AppCommand::ExportUnderlayDxf)
    );
    let digest = harness.state().canonical_digest();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExportUnderlayDxf);

    let app = harness.state();
    assert!(app.digest.contains("edited.dxf"), "{}", app.digest);
    assert_eq!(app.canonical_digest(), digest, "exporting changes nothing");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), PLAN);
    let written = std::fs::read_to_string(&output).unwrap();
    assert!(!written.contains("5\nA1\n"), "{written}");
    assert!(written.contains("0\nLINE\n5\nA2\n8\naxes\n"), "{written}");
    let again = ketchup_model::import::read_dxf_underlay(
        written.as_bytes(),
        ketchup_model::import::DxfImportOptions::new(None),
    )
    .unwrap();
    assert_eq!(again.lines.segment_count(), 1);
}

/// Review 2026-10-29 SEC-60: in the window the drawing is written back on a thread while
/// the file activity indication shows, so the window keeps painting; stopped, it keeps
/// the file the user chose to replace. Given again, the export replaces it.
#[test]
fn the_window_writes_a_drawing_back_on_a_thread_that_cancelling_stops() {
    const PLAN: &str = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nLINE\n5\nA1\n8\nwalls\n10\n40\n20\n20\n11\n240\n21\n20\n",
        "0\nLINE\n5\nA2\n8\naxes\n10\n40\n20\n120\n11\n240\n21\n120\n",
        "0\nENDSEC\n0\nEOF\n",
    );
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dxf");
    let output = directory.path().join("edited.dxf");
    std::fs::write(&source, PLAN).unwrap();
    std::fs::write(&output, "kept").unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_export(&output)
        .queue_export(&output);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let mut harness = shell_of(app);
    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    press(&mut harness, egui::Key::Delete);

    // Given in a frame, as the menu gives it.
    let export = |harness: &mut Harness<'_, KetchupApp>| {
        let context = harness.ctx.clone();
        let app = harness.state_mut();
        app.file.activity.context = Some(context);
        app.dispatch_command(AppCommand::ExportUnderlayDxf);
        assert!(app.file.activity.underlay.exporting.is_some());
        assert!(app.file_activity_label().is_some());
    };
    // Closing the window stops it in the frame it is asked, before the drawing comes back.
    export(&mut harness);
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.step();
    let app = harness.state();
    assert!(app.file.activity.underlay.exporting.is_none());
    assert_eq!(app.digest, app.catalog.text("digest-cancelled"));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "kept");
    // The unsaved work keeps the window open.
    harness.state_mut().close_guard.pending = false;

    export(&mut harness);
    crate::tests::shell::step_until(
        &mut harness,
        Duration::from_secs(30),
        "the drawing was not written back",
        |app| app.file.activity.underlay.exporting.is_none(),
    );
    let app = harness.state();
    assert!(app.digest.contains("edited.dxf"), "{}", app.digest);
    let written = std::fs::read_to_string(&output).unwrap();
    assert!(!written.contains("5\nA1\n"), "{written}");
    assert!(written.contains("0\nLINE\n5\nA2\n8\naxes\n"), "{written}");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), PLAN);
}

/// A wall moved with its floor hatch and its dimension, clicked in a headless window
/// (Shift picks the hatch and the dimension too) and exported: the DXF written draws all
/// three where they were moved to, the dimension's lines from a block of its own.
#[test]
fn a_wall_moved_with_its_hatch_and_dimension_exports_where_they_now_lie() {
    const PLAN: &str = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n5\n30\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n30\n0\n3\n*D1\n",
        "0\nLINE\n5\n31\n8\n0\n10\n40\n20\n100\n11\n240\n21\n100\n",
        "0\nLINE\n5\n32\n8\n0\n10\n40\n20\n30\n11\n40\n21\n110\n",
        "0\nLINE\n5\n33\n8\n0\n10\n240\n20\n30\n11\n240\n21\n110\n",
        "0\nENDBLK\n5\n34\n8\n0\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nLINE\n5\nA1\n8\nwalls\n10\n40\n20\n20\n11\n240\n21\n20\n",
        "0\nHATCH\n5\nA8\n8\nfloor\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n3\n",
        "72\n0\n73\n1\n93\n4\n10\n100\n20\n180\n10\n180\n20\n180\n10\n180\n20\n240\n",
        "10\n100\n20\n240\n97\n0\n75\n0\n76\n1\n98\n0\n",
        "0\nDIMENSION\n5\nA9\n8\ndims\n2\n*D1\n10\n240\n20\n100\n30\n0\n11\n140\n21\n110\n31\n0\n",
        "70\n33\n42\n200\n13\n40\n23\n30\n33\n0\n14\n240\n24\n30\n34\n0\n",
        "0\nENDSEC\n0\nEOF\n",
    );
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dxf");
    let output = directory.path().join("edited.dxf");
    std::fs::write(&source, PLAN).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_export(&output);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let mut harness = shell_of(app);
    let undo = harness.state().undo_step_count();

    click(&mut harness, Vec3::new(100.0, 20.0, 0.0));
    shift_click(&mut harness, Vec3::new(140.0, 210.0, 0.0));
    shift_click(&mut harness, Vec3::new(140.0, 100.0, 0.0));
    assert_eq!(picked(&harness), 3, "{}", harness.state().digest);
    // From the wall's end onto the box's top corner: in plan by (-140, 40).
    harness.state_mut().dispatch_command(AppCommand::Move);
    click(&mut harness, Vec3::new(240.0, 20.0, 0.0));
    click(&mut harness, Vec3::new(100.0, 60.0, 20.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(!end_at(&mut harness, [40.0, 20.0]), "the wall moved");
    assert!(end_at(&mut harness, [100.0, 140.0]), "the dimension moved");

    harness.state_mut().dispatch_command(AppCommand::Select);
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExportUnderlayDxf);
    let app = harness.state();
    assert!(app.digest.contains("edited.dxf"), "{}", app.digest);
    let written = std::fs::read_to_string(&output).unwrap();
    let again = ketchup_model::import::read_dxf_underlay(
        written.as_bytes(),
        ketchup_model::import::DxfImportOptions::new(None),
    )
    .unwrap();
    let layer = |name: &str| {
        let at = again.layers.iter().position(|layer| layer == name).unwrap();
        &again.lines.layers()[at]
    };
    let ends = |name: &str| {
        let mut ends = layer(name)
            .polylines()
            .flat_map(|polyline| [polyline[0], polyline[polyline.len() - 1]])
            .map(|[x, y]| [x.round(), y.round()])
            .collect::<Vec<_>>();
        ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ends
    };
    assert_eq!(ends("walls"), [[-100.0, 60.0], [100.0, 60.0]], "{written}");
    assert_eq!(
        ends("dims"),
        [
            [-100.0, 70.0],
            [-100.0, 140.0],
            [-100.0, 150.0],
            [100.0, 70.0],
            [100.0, 140.0],
            [100.0, 150.0]
        ],
        "{written}"
    );
    let floor = layer("floor").fills();
    assert_eq!(floor.len(), 1);
    assert!(floor[0].contains([0.0, 250.0]) && !floor[0].contains([140.0, 210.0]));
}

/// A room with a column in it (two closed polylines) and a solid hatch with no boundary.
const ROOM_AND_FLOOR: &str = concat!(
    "0\nLWPOLYLINE\n8\nrooms\n90\n4\n70\n1\n",
    "10\n200\n20\n160\n10\n290\n20\n160\n10\n290\n20\n280\n10\n200\n20\n280\n",
    "0\nLWPOLYLINE\n8\nrooms\n90\n4\n70\n1\n",
    "10\n230\n20\n200\n10\n250\n20\n200\n10\n250\n20\n220\n10\n230\n20\n220\n",
    "0\nHATCH\n8\nfloor\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n3\n",
    "72\n0\n73\n1\n93\n4\n10\n100\n20\n180\n10\n180\n20\n180\n10\n180\n20\n240\n",
    "10\n100\n20\n240\n97\n0\n75\n0\n76\n1\n98\n0\n",
);
const IN_ROOM: Vec3 = Vec3::new(215.0, 260.0, 0.0);
const IN_FLOOR: Vec3 = Vec3::new(140.0, 210.0, 0.0);

fn pull_up(harness: &mut Harness<'_, KetchupApp>, height: &str, at: Vec3) {
    harness.state_mut().value_box.input = height.to_owned();
    click(harness, at);
    // A click before the regions are found pulls the area up on a later frame.
    crate::tests::shell::step_until(
        harness,
        std::time::Duration::from_secs(60),
        "the regions of the click were never found",
        |app| app.solid_tools.area_click_awaits_regions.is_none(),
    );
}

/// Steps until a change of a drawing waiting for the regions of the areas that follow it,
/// pushed out of those kept by other tests, is made.
fn awaited(harness: &mut Harness<'_, KetchupApp>) {
    crate::tests::shell::step_until(
        harness,
        std::time::Duration::from_secs(60),
        "the regions of the change were never found",
        |app| app.solid_tools.drawing_edit_awaits_regions.is_none(),
    );
}

fn program_source(harness: &Harness<'_, KetchupApp>) -> String {
    harness
        .state()
        .document
        .current_rule_program()
        .map(|program| program.source.clone())
        .unwrap_or_default()
}

/// 2D-4: Tools > Pull drawing area up next to a part of the window's own: a click inside
/// the room pulls it up the typed height as a part with the column cut through, a click
/// in the hatch pulls the hatch up; one Undo step each, and a click on nothing closed, or
/// with no height, changes nothing.
#[test]
fn a_clicked_closed_area_of_a_drawing_is_pulled_up_into_a_part() {
    let mut harness = shell_of(imported_with(ROOM_AND_FLOOR));
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    assert_eq!(harness.state().active_tool, ActiveTool::ExtrudeUnderlayArea);

    pull_up(&mut harness, "300", IN_ROOM);
    pull_up(&mut harness, "50", IN_FLOOR);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 2, "{}", app.digest);
    assert!(app.digest.contains("AREA_2"), "{}", app.digest);
    let snapshot = app.document.current();
    let features = |name: &str| {
        let definition = snapshot
            .definitions()
            .find(|definition| definition.name() == name)
            .unwrap_or_else(|| panic!("no part {name}"));
        assert_eq!(
            snapshot
                .occurrences()
                .filter(|occurrence| occurrence.definition_id() == definition.id())
                .count(),
            1
        );
        snapshot
            .features()
            .filter(|feature| feature.definition_id() == definition.id())
            .map(|feature| (definition.id(), feature.id(), feature.kind().clone()))
            .collect::<Vec<_>>()
    };
    let room = features("AREA_1");
    let heights = |features: &[(DefinitionId, FeatureId, FeatureKind)]| {
        features
            .iter()
            .filter_map(|(_, _, kind)| match kind {
                FeatureKind::Pad(PadSpec {
                    extent: FeatureExtent::Blind(height),
                    ..
                }) => Some(height.millimetres()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        heights(&room),
        [300.0, 302.0],
        "the room, the column's tool"
    );
    let (definition, cut, _) = room
        .iter()
        .find(|(_, _, kind)| matches!(kind, FeatureKind::Boolean { .. }))
        .cloned()
        .expect("the column is cut out");
    assert!(
        ExactBRepGraph::from_snapshot(&snapshot, definition, cut).is_ok(),
        "the room with the column cut out builds"
    );
    assert_eq!(heights(&features("AREA_2")), [50.0]);
    assert!(program_source(&harness).is_empty(), "no program was made");

    pull_up(&mut harness, "300", Vec3::new(20.0, -100.0, 0.0));
    assert_eq!(harness.state().undo_step_count(), undo + 2);
    assert!(
        harness.state().digest.contains("closed area"),
        "{}",
        harness.state().digest
    );
    pull_up(&mut harness, "", IN_ROOM);
    assert_eq!(harness.state().undo_step_count(), undo + 2);

    harness.state_mut().undo();
    harness.state_mut().undo();
    assert_eq!(harness.state().canonical_digest(), digest);
}

/// Review 2026-10-29 UO-69: a plan raised off the ground (a floor above the first) was no
/// plan to the window: its lines gave no snap and Pull area found nothing in it. Seen at an
/// angle, the pointer snaps to its corner where it lies and a click inside its room pulls
/// the room up, the click met in the plan's own plane.
#[test]
fn a_plan_raised_off_the_ground_snaps_and_is_pulled_up_where_it_lies() {
    use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};
    let height = 150.0;
    let mut harness = shell_of(imported_with(ROOM_AND_FLOOR));
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    let view = UnderlayView {
        plane: ViewPlane::Top,
        at_mm: height,
    };
    let batch = plan_underlay_view(&harness.state().document.current(), id, view).unwrap();
    harness
        .state_mut()
        .apply_batch_with_work_recovery(&batch)
        .unwrap();
    harness.state_mut().dispatch_command(AppCommand::ViewIso);
    harness.step();

    let corner = Vec3::new(290.0, 280.0, height);
    point(&mut harness, corner);
    let snapped = harness
        .state()
        .hover
        .snap
        .as_ref()
        .map(|snap| snap.position_mm);
    assert_eq!(snapped, Some(corner), "the raised room's corner is snapped");

    let undo = harness.state().undo_step_count();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", Vec3::new(IN_ROOM.x, IN_ROOM.y, height));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(app.digest.contains("AREA_1"), "{}", app.digest);

    // The drawing tools draw in the ground plane: they draw into it once it lies there.
    let app = harness.state_mut();
    app.select_whole_underlay(id);
    app.dispatch_command(AppCommand::DrawIntoUnderlay);
    assert_eq!(app.underlay_draw_target(), None);
    assert!(app.digest.contains("ground plane"), "{}", app.digest);
}

/// 2D-4 in a document with only the drawing: the pulled areas are written into the program
/// as `program action=from_underlay` writes them, bound to the drawing.
#[test]
fn a_pulled_area_of_a_drawing_alone_is_written_into_the_program() {
    let mut harness = shell_of(imported_alone_with(ROOM_AND_FLOOR));
    let undo = harness.state().undo_step_count();
    let digest = harness.state().canonical_digest();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", IN_ROOM);
    pull_up(&mut harness, "50", IN_FLOOR);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 2, "{}", app.digest);
    assert!(app.digest.contains("follows the drawing"), "{}", app.digest);
    let written = program_source(&harness);
    for expected in [
        "# underlay binding ",
        "AREA_1_SOLID = extrude(\"AREA_1\", profile = AREA_1, distance = 300)",
        "(200, 160)",
        "(230, 200)",
        "AREA_2_SOLID = extrude(\"AREA_2\", profile = AREA_2, distance = 50)",
        "(100, 180)",
    ] {
        assert!(written.contains(expected), "{expected} in {written}");
    }
    assert!(harness.state().document.current().occurrences().count() >= 2);

    harness.state_mut().undo();
    harness.state_mut().undo();
    assert_eq!(harness.state().canonical_digest(), digest);
}

/// UO-67: an area pulled up on a drawing that is then stood up as the front view stands
/// with it, its height along the view's line of sight and the room upright in the XZ
/// plane; laid back into the plan it lies on the ground again. Before, it stayed on the
/// ground whatever the drawing did.
#[test]
fn a_pulled_area_stands_up_with_its_drawing() {
    use ketchup_model::underlay::view::ViewPlane;
    let mut harness = shell_of(imported_alone_with(ROOM_AND_FLOOR));
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", IN_ROOM);
    let size = |harness: &Harness<'_, KetchupApp>| {
        let app = harness.state();
        let snapshot = app.document.current();
        let occurrence = snapshot
            .occurrences()
            .find(|occurrence| occurrence.name() == "AREA_1")
            .expect("the room is pulled up")
            .id();
        let root = InstancePath::root(occurrence);
        let [low, high] = app.rotation_bounds_for(&|path| *path == root).unwrap();
        (low, high - low)
    };
    let (_, lying) = size(&harness);
    assert!((lying.z - 300.0).abs() < 1e-6, "{lying:?}");
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;

    assert!(
        harness.state_mut().set_underlay_view(id, ViewPlane::Front),
        "{}",
        harness.state().digest
    );
    awaited(&mut harness);
    let (low, standing) = size(&harness);
    assert!(
        (standing.y - 300.0).abs() < 1e-6 && (standing.z - lying.y).abs() < 1e-6,
        "{standing:?}, {}",
        program_source(&harness)
    );
    // The drawing's +Z faces the viewer in front: the room is pulled towards −Y.
    assert!((low.y + 300.0).abs() < 1e-6, "{low:?}");
    assert!((low.z - 160.0).abs() < 1e-6, "{low:?}");
    let stood = "place(profile_part(\"AREA_1/outline\"";
    assert!(
        program_source(&harness).contains(stood),
        "the sketch stands too"
    );

    assert!(
        harness.state_mut().set_underlay_view(id, ViewPlane::Top),
        "{}",
        harness.state().digest
    );
    awaited(&mut harness);
    let (_, again) = size(&harness);
    assert!((again - lying).length() < 1e-6, "{again:?}");
    let source = program_source(&harness);
    assert!(!source.contains("place("), "{source}");
}

/// A closed outline below the plan whose lower edge is a saw of `teeth` teeth, 10 m wide.
fn saw(teeth: u32) -> String {
    let mut entity = format!("0\nLWPOLYLINE\n8\nwalls\n90\n{}\n70\n1\n", 2 * teeth + 3);
    let step = 10_000.0 / f64::from(teeth);
    for tooth in 0..teeth {
        let x = -5_000.0 + step * f64::from(tooth);
        entity.push_str(&format!(
            "10\n{x}\n20\n-400\n10\n{}\n20\n-390\n",
            x + step / 2.0
        ));
    }
    entity.push_str("10\n5000\n20\n-400\n10\n5000\n20\n-100\n10\n-5000\n20\n-100\n");
    entity
}

/// Review 2026-10-29 UO-63: an area refused for its size said "No closed area is there"
/// or "cannot be written (invalid_params)". The refusal is said: an outline of more
/// corners than one piece of program data holds, and a drawing of more straight pieces
/// than one region search takes.
#[test]
fn a_pulled_area_refused_for_its_size_says_why() {
    let in_saw = Vec3::new(20.0, -250.0, 0.0);
    let mut harness = shell_of(imported_alone_with(&saw(2_000)));
    let undo = harness.state().undo_step_count();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", in_saw);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo, "{}", app.digest);
    assert!(
        app.digest.starts_with("Nothing was pulled up")
            && app.digest.contains("corners, more than the 4000"),
        "{}",
        app.digest
    );

    // An open line of more pieces than one search takes, snaking above the plan.
    let pieces = ketchup_model::underlay::regions::MAX_REGION_SEGMENTS + 1;
    let mut snake = format!("0\nLWPOLYLINE\n8\nwalls\n90\n{}\n70\n0\n", pieces + 1);
    for vertex in 0..=pieces {
        let (row, column) = (vertex / 1_000, vertex % 1_000);
        let column = if row % 2 == 0 { column } else { 999 - column };
        snake.push_str(&format!("10\n{}\n20\n{}\n", 10 * column, 1_000 + 5 * row));
    }
    let mut harness = shell_of(imported_with(&format!("{}{snake}", saw(10))));
    let undo = harness.state().undo_step_count();
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", in_saw);
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo, "{}", app.digest);
    assert!(
        app.digest
            .contains("straight pieces are more than the 200000"),
        "{}",
        app.digest
    );
}

/// Review 2026-10-26 UO-40: the first click of Pull area found the drawing's regions on
/// the UI thread, a second and more on a large plan. Choosing the tool starts the search
/// off it, and the click takes what it found.
#[test]
fn choosing_pull_area_finds_the_regions_off_the_ui_thread() {
    let mut harness = shell_of(imported_alone_with(&saw(37)));
    let found = |app: &KetchupApp| {
        let snapshot = app.document.current();
        let underlay = snapshot.underlays().next().unwrap();
        let lines = app.cached_underlay_lines(underlay).unwrap();
        crate::live_bridge::underlay_regions::found_ahead(&snapshot, underlay, &lines)
    };
    assert!(!found(harness.state()), "nothing was searched yet");
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the regions were not found ahead of a click",
        found,
    );
    let undo = harness.state().undo_step_count();
    pull_up(&mut harness, "300", Vec3::new(20.0, -250.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
}

/// Review 2026-10-26 UO-40, after the search ahead: a click of Pull area before the search
/// ahead was done found the regions again on the UI thread. It waits for that search and
/// pulls the area up on the frame its regions are found.
#[test]
fn a_pull_area_click_before_the_regions_are_found_waits_for_them_off_the_ui_thread() {
    let hold = crate::live_bridge::underlay_regions::HoldAhead::hold();
    let held = hold.clone();
    let guard =
        crate::tests::shell::hang_guard("the search of the regions ahead", move || held.let_go());
    // A drawing of its own: the regions another test found are kept for this one too.
    let mut harness = shell_of(imported_alone_with(&saw(41)));
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    let undo = harness.state().undo_step_count();
    harness.state_mut().value_box.input = "300".to_owned();
    click(&mut harness, Vec3::new(20.0, -250.0, 0.0));
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo, "{}", app.digest);
    assert!(
        app.digest.starts_with("Finding the areas"),
        "{}",
        app.digest
    );
    hold.let_go();
    drop(guard);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the waiting click never pulled the area up",
        |app| app.undo_step_count() > undo,
    );
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(
        app.digest.starts_with("Pulled the area up"),
        "{}",
        app.digest
    );
}

/// Review 2026-10-29 UO-40: a line deleted from a drawing an area follows found the area's
/// region again on the UI thread, in the commit, once for every area, a second and more on
/// a large plan. The deletion waits for the regions found off it and is made on the frame
/// they are.
#[test]
fn a_line_deleted_from_a_drawing_an_area_follows_waits_for_its_regions_off_the_ui_thread() {
    // Lines of the room's layer of its own, one left after the deletion: the regions
    // another test found are not these, before it or after.
    let mut harness = shell_of(imported_alone_with(&format!(
        "{ROOM_AND_FLOOR}0\nLINE\n8\nrooms\n10\n13\n20\n400\n11\n113\n21\n400\n0\nLINE\n8\nrooms\n10\n13\n20\n431\n11\n113\n21\n431\n"
    )));
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", IN_ROOM);
    harness.state_mut().dispatch_command(AppCommand::Select);
    let undo = harness.state().undo_step_count();
    click(&mut harness, Vec3::new(60.0, 400.0, 0.0));
    assert_eq!(picked(&harness), 1, "{}", harness.state().digest);

    let hold = crate::live_bridge::underlay_regions::HoldAhead::hold();
    let held = hold.clone();
    let guard = crate::tests::shell::hang_guard("the search of the followed regions", move || {
        held.let_go();
    });
    let searched = crate::live_bridge::underlay_regions::searches_on_this_thread();
    press(&mut harness, egui::Key::Delete);
    let app = harness.state();
    assert_eq!(
        crate::live_bridge::underlay_regions::searches_on_this_thread(),
        searched,
        "the UI thread found regions"
    );
    assert_eq!(app.undo_step_count(), undo, "{}", app.digest);
    assert!(
        app.digest.starts_with("Finding the areas"),
        "{}",
        app.digest
    );
    hold.let_go();
    drop(guard);
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(60),
        "the waiting deletion was never made",
        |app| app.undo_step_count() > undo,
    );
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(
        app.digest.contains("taken out of the drawing"),
        "{}",
        app.digest
    );
    assert_eq!(
        crate::live_bridge::underlay_regions::searches_on_this_thread(),
        searched,
        "the UI thread found regions"
    );
}

/// Review 2026-10-29 UO-40: with the regions of an area that follows a drawing not found
/// yet, as after opening a document, moving the drawing with the Move tool, fitting it and
/// standing it up as a view found them on the UI thread in the commit. Each waits for them
/// found off it and is made on the frame they are.
#[test]
fn moving_fitting_or_standing_up_a_drawing_an_area_follows_waits_for_its_regions_off_the_ui_thread()
{
    use crate::transform_operation::TransformPlan;
    use ketchup_model::underlay::view::ViewPlane;
    let mut harness = shell_of(imported_alone_with(&format!(
        "{ROOM_AND_FLOOR}0\nLINE\n8\nrooms\n10\n13\n20\n470\n11\n113\n21\n470\n"
    )));
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    pull_up(&mut harness, "300", IN_ROOM);
    let id = harness
        .state()
        .document
        .current()
        .underlays()
        .next()
        .unwrap()
        .id;
    type Change = Box<dyn Fn(&mut KetchupApp)>;
    let changes: [(&str, Change, &str); 3] = [
        (
            "move",
            Box::new(move |app| {
                let edit = Transform::from_translation(70.0, 30.0, 0.0).unwrap();
                let plan = TransformPlan::underlay(&app.document.current(), id, edit).unwrap();
                let selection = super::underlay_snapping::underlay_reference(id, 0);
                assert!(app.commit_move_transform(plan, &selection, Vec3::new(70.0, 30.0, 0.0)));
            }),
            "Moved",
        ),
        (
            "fit",
            Box::new(move |app| {
                app.fit_underlay_in_window(
                    id,
                    [[0.0, 0.0], [1000.0, 0.0]],
                    [[-40.0, 20.0], [960.0, 20.0]],
                    false,
                );
            }),
            "Underlay placed",
        ),
        (
            "view",
            Box::new(move |app| {
                assert!(app.set_underlay_view(id, ViewPlane::Front));
            }),
            "",
        ),
    ];
    for (change, make, made) in changes {
        harness.state().forget_followed_regions();
        let hold = crate::live_bridge::underlay_regions::HoldAhead::hold();
        let held = hold.clone();
        let guard =
            crate::tests::shell::hang_guard("the search of the followed regions", move || {
                held.let_go();
            });
        let undo = harness.state().undo_step_count();
        let searched = crate::live_bridge::underlay_regions::searches_on_this_thread();
        make(harness.state_mut());
        let app = harness.state();
        assert_eq!(
            crate::live_bridge::underlay_regions::searches_on_this_thread(),
            searched,
            "{change}: the UI thread found regions"
        );
        assert_eq!(app.undo_step_count(), undo, "{change}: {}", app.digest);
        assert!(
            app.digest.starts_with("Finding the areas"),
            "{change}: {}",
            app.digest
        );
        hold.let_go();
        drop(guard);
        crate::tests::shell::step_until(
            &mut harness,
            std::time::Duration::from_secs(60),
            "the waiting change was never made",
            |app| app.undo_step_count() > undo,
        );
        let app = harness.state();
        assert_eq!(app.undo_step_count(), undo + 1, "{change}: {}", app.digest);
        assert!(app.digest.starts_with(made), "{change}: {}", app.digest);
        assert_eq!(
            crate::live_bridge::underlay_regions::searches_on_this_thread(),
            searched,
            "{change}: the UI thread found regions"
        );
    }
}

/// Review 2026-10-26 UO-40, a waiting click in a building of a plan per floor: the click
/// waited until the regions of every flat drawing were found at once, more drawings than
/// the found searches kept, so each search found pushed out another and the click waited
/// for ever.
#[test]
fn a_waiting_pull_area_click_is_pulled_up_among_more_flat_drawings_than_searches_kept() {
    let directory = tempfile::tempdir().unwrap();
    let mut dialogs = dialogs::ScriptedFileDialogs::new();
    // Drawings of their own: the regions another test found are kept for this one too.
    for teeth in 43..49 {
        let path = directory.path().join(format!("floor{teeth}.dxf"));
        std::fs::write(
            &path,
            format!(
                "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
                 0\nSECTION\n2\nENTITIES\n{}0\nENDSEC\n0\nEOF\n",
                saw(teeth)
            ),
        )
        .unwrap();
        dialogs = dialogs.queue_import(ImportFormat::Dxf, &path);
    }
    let mut app = KetchupApp::with_catalog_and_initial_box(LocaleCatalog::english(), false)
        .with_dialogs(Box::new(dialogs));
    for _ in 43..49 {
        app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    }
    let flat = app
        .document
        .current()
        .underlays()
        .filter(|underlay| underlay.lies_flat())
        .count();
    assert_eq!(flat, 6, "{}", app.action_digest());
    let mut harness = shell_of(app);
    harness
        .state_mut()
        .dispatch_command(AppCommand::ExtrudeUnderlayArea);
    let undo = harness.state().undo_step_count();
    harness.state_mut().value_box.input = "300".to_owned();
    click(&mut harness, Vec3::new(20.0, -250.0, 0.0));
    crate::tests::shell::step_until(
        &mut harness,
        std::time::Duration::from_secs(20),
        "the waiting click never pulled the area up",
        |app| app.solid_tools.area_click_awaits_regions.is_none(),
    );
    let app = harness.state();
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(
        app.digest.starts_with("Pulled the area up"),
        "{}",
        app.digest
    );
}
