//! Clicked in a headless window: a click on a line of an underlay picks the object that drew
//! it, Shift+click picks more or drops one, Delete takes them out of the drawing in one Undo
//! step, and the drawing no longer snaps where they were.
use super::underlay_snapping_tests::{imported_alone_with, imported_with};
use super::underlay_transform_tests::{click, double_click, press, shell, shell_of};
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

fn type_text(harness: &mut Harness<'_, KetchupApp>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    harness.step();
    harness.step();
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
