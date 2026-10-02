//! Polygon, Ellipse, Spline, Mirror and 2D Offset driven through the window the
//! way a person uses them: tool rail or shortcut, clicks, drags, typed values,
//! Escape and Undo — replayed offscreen, never with the real pointer.

use crate::harness::{Shell, ctrl};

use std::collections::BTreeMap;

use eframe::egui::Key;
use ketchup_app::AppCommand;
use ketchup_interaction::Vec3;
use ketchup_model::document::{ProfileSegment, Transform};

fn world(transform: &Transform, point: [f64; 2]) -> Vec3 {
    let [x, y, z] = transform.transform_point([point[0], point[1], 0.0]);
    Vec3::new(x, y, z)
}

/// Corners of the newest drawn profile in world coordinates, or `None` when one
/// of its segments is curved.
fn latest_corners(shell: &Shell) -> Option<Vec<Vec3>> {
    let (transform, segments) = shell.app().latest_profile()?;
    segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, .. } => Some(world(&transform, *start_mm)),
            _ => None,
        })
        .collect()
}

fn assert_regular(corners: &[Vec3], center: Vec3, radius_mm: f64, sides: usize, tolerance_mm: f64) {
    assert_eq!(corners.len(), sides, "corner count");
    for corner in corners {
        assert!(
            (corner.distance(center) - radius_mm).abs() < tolerance_mm,
            "corner {corner:?} is not {radius_mm} mm from {center:?}"
        );
    }
}

#[test]
fn polygon_takes_sides_and_radius_by_click_drag_or_typing_in_one_undo_step() {
    let mut shell = Shell::new();
    let tool_active = |shell: &Shell| {
        shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-polygon"))]),
        )
    };
    assert!(shell.offers(AppCommand::Polygon), "Polygon is in the rail");

    // The shortcut picks the tool; the value box offers the side count first.
    shell.press_key(Key::N);
    assert_eq!(shell.app().action_digest(), tool_active(&shell));
    assert_eq!(shell.app().value_input(), "6");
    shell.type_text("5");
    shell.press_key(Key::Enter);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    // Click the centre, move to a corner: the preview follows without editing.
    let center = Vec3::new(30.0, 20.0, 0.0);
    let corner = Vec3::new(50.0, 20.0, 0.0);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.move_pointer(shell.app().viewport_position(corner).unwrap());
    let preview = shell.app().closed_shape_preview_outline().unwrap();
    let preview_center =
        preview.iter().fold(Vec3::ZERO, |sum, point| sum + *point) * (1.0 / preview.len() as f64);
    // The pointer lands on screen pixels, so the preview is only near 20 mm.
    assert_regular(&preview, preview_center, 20.0, 5, 1.0e-3);
    assert_eq!(shell.app().document_revision(), before_revision);

    // A typed radius finishes it exactly, as one Undo step.
    shell.type_text("12.5");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    let first = latest_corners(&shell).expect("a polygon is straight lines");
    assert_regular(&first, preview_center, 12.5, 5, 1.0e-6);
    let first_digest = shell.app().canonical_digest();
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // Escape drops a polygon in progress without touching the document.
    shell.click_command(AppCommand::Polygon);
    shell.click_at(
        shell
            .app()
            .viewport_position(Vec3::new(-40.0, 0.0, 0.0))
            .unwrap(),
    );
    assert!(shell.app().closed_shape_preview_outline().is_some());
    shell.press_key(Key::Escape);
    assert!(shell.app().closed_shape_preview_outline().is_none());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // Dragging from a snapped corner of the first polygon draws the second one.
    shell.click_command(AppCommand::Polygon);
    let snapped = first[0];
    let second_corner = snapped + Vec3::new(0.0, 15.0, 0.0);
    shell.drag(
        shell.app().viewport_position(snapped).unwrap() + eframe::egui::Vec2::new(2.0, 0.0),
        shell.app().viewport_position(second_corner).unwrap(),
    );
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    let second = latest_corners(&shell).unwrap();
    let (transform, _) = shell.app().latest_profile().unwrap();
    let second_center = world(&transform, [0.0, 0.0]);
    assert!(
        second_center.distance(snapped) < 1.0e-6,
        "the centre snaps to the corner it was dragged from"
    );
    let radius = second[0].distance(second_center);
    assert!((radius - 15.0).abs() < 1.0e-3, "dragged radius {radius}");
    assert_regular(&second, second_center, radius, 5, 1.0e-6);
}
