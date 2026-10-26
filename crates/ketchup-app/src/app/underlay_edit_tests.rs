//! Clicked in a headless window: a click on a line of an underlay picks the object that drew
//! it, Shift+click picks more or drops one, Delete takes them out of the drawing in one Undo
//! step, and the drawing no longer snaps where they were.
use super::underlay_transform_tests::{click, press, shell};
use crate::*;
use egui_kittest::Harness;
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
