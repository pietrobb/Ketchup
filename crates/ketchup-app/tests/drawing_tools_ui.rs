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

/// Ends of the newest drawn ellipse's four quarters and its centre, in world
/// coordinates.
fn latest_ellipse(shell: &Shell) -> ([Vec3; 4], Vec3) {
    let (transform, segments) = shell.app().latest_profile().unwrap();
    assert_eq!(segments.len(), 4, "four quarter curves");
    let ends: Vec<Vec3> = segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::CubicBezier { start_mm, .. } => world(&transform, *start_mm),
            other => panic!("an ellipse quarter is a cubic, got {other:?}"),
        })
        .collect();
    (ends.try_into().unwrap(), world(&transform, [0.0, 0.0]))
}

#[test]
fn ellipse_takes_centre_and_both_half_axes_by_drag_click_or_typing_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.offers(AppCommand::Ellipse), "Ellipse is in the rail");
    shell.press_key(Key::E);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-ellipse"))]),
        )
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    // Drag from the centre to the end of the first half-axis, then follow the
    // pointer for the second one: the preview is that ellipse, nothing is saved.
    let center = Vec3::new(10.0, 10.0, 0.0);
    let major_end = Vec3::new(10.0, 30.0, 0.0);
    shell.drag(
        shell.app().viewport_position(center).unwrap(),
        shell.app().viewport_position(major_end).unwrap(),
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    shell.move_pointer(
        shell
            .app()
            .viewport_position(Vec3::new(16.0, 10.0, 0.0))
            .unwrap(),
    );
    let preview = shell.app().closed_shape_preview_outline().unwrap();
    let widest = preview
        .iter()
        .map(|point| point.distance(preview[0]))
        .fold(0.0, f64::max);
    assert_eq!(shell.app().document_revision(), before_revision);

    // The typed second half-axis finishes it exactly, as one Undo step.
    shell.type_text("8");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    let (ends, committed_center) = latest_ellipse(&shell);
    let radius_x = ends[0].distance(committed_center);
    assert!(
        (widest - 2.0 * radius_x).abs() < 1.0e-6,
        "the preview spanned the committed first axis: {widest} vs {radius_x}"
    );
    assert!((ends[2].distance(committed_center) - radius_x).abs() < 1.0e-9);
    assert!((ends[1].distance(committed_center) - 8.0).abs() < 1.0e-9);
    assert!((ends[3].distance(committed_center) - 8.0).abs() < 1.0e-9);
    let first_digest = shell.app().canonical_digest();
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // Escape drops an ellipse in progress.
    shell.click_command(AppCommand::Ellipse);
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

    // Click-move-click with both half-axes typed: the centre snaps to the end
    // of the first ellipse's axis and the sizes are exact.
    shell.click_command(AppCommand::Ellipse);
    let snapped = ends[0];
    shell.click_at(
        shell.app().viewport_position(snapped).unwrap() + eframe::egui::Vec2::new(2.0, 0.0),
    );
    shell.move_pointer(
        shell
            .app()
            .viewport_position(snapped + Vec3::new(25.0, 0.0, 0.0))
            .unwrap(),
    );
    shell.type_text("30");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    shell.type_text("10");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    let (ends, second_center) = latest_ellipse(&shell);
    assert!(
        second_center.distance(snapped) < 1.0e-6,
        "the centre snaps to the axis end it was clicked near"
    );
    assert!((ends[0].distance(second_center) - 30.0).abs() < 1.0e-9);
    assert!((ends[1].distance(second_center) - 10.0).abs() < 1.0e-9);
}

/// The points the newest drawn spline passes through, in world coordinates,
/// without the repeated closing point.
#[track_caller]
fn latest_spline(shell: &Shell) -> Vec<Vec3> {
    let (transform, segments) = shell.app().latest_profile().unwrap();
    let [ProfileSegment::Spline { points_mm }] = segments.as_slice() else {
        panic!("a drawn spline is one spline segment, got {segments:?}");
    };
    assert_eq!(
        points_mm.first(),
        points_mm.last(),
        "the spline closes on its first point"
    );
    points_mm[..points_mm.len() - 1]
        .iter()
        .map(|point| world(&transform, *point))
        .collect()
}

#[test]
fn spline_takes_points_by_drag_click_or_typed_distance_and_closes_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.offers(AppCommand::Spline), "Spline is in the rail");
    shell.press_key(Key::S);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-spline"))]),
        )
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let at = |shell: &Shell, x: f64, y: f64| {
        shell.app().viewport_position(Vec3::new(x, y, 0.0)).unwrap()
    };

    // A drag from the first point places the second, a click the third; the
    // preview is a closed curve from the first point through them and the
    // pointer, and nothing is saved yet.
    shell.drag(at(&shell, 10.0, 10.0), at(&shell, 50.0, 10.0));
    shell.click_at(at(&shell, 50.0, 40.0));
    shell.move_pointer(at(&shell, 10.0, 40.0));
    let preview = shell.app().closed_shape_preview_outline().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision);

    // A typed distance places the fourth point that far toward the pointer.
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), before_revision);

    // Clicking next to the first point closes the spline as one Undo step.
    shell.click_at(
        shell.app().viewport_position(preview[0]).unwrap() + eframe::egui::Vec2::new(3.0, 0.0),
    );
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert!(shell.app().closed_shape_preview_outline().is_none());
    let points = latest_spline(&shell);
    assert_eq!(points.len(), 4);
    for placed in &points[..3] {
        assert!(
            preview.iter().any(|point| point.distance(*placed) < 1.0e-6),
            "the preview passed through {placed:?}"
        );
    }
    assert!(
        points[0].distance(preview[0]) < 1.0e-9,
        "closed on the first point"
    );
    let typed = points[3].distance(points[2]);
    assert!((typed - 25.0).abs() < 1.0e-9, "typed distance {typed}");
    let first_digest = shell.app().canonical_digest();
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // Escape drops a spline with too few points to close.
    shell.click_command(AppCommand::Spline);
    shell.click_at(at(&shell, 20.0, 20.0));
    shell.click_at(at(&shell, 35.0, 20.0));
    assert!(shell.app().closed_shape_preview_outline().is_some());
    shell.press_key(Key::Escape);
    assert!(shell.app().closed_shape_preview_outline().is_none());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // With enough points, Escape and Enter both close it.
    for (finish, revision, y) in [(Key::Escape, 2, 15.0), (Key::Enter, 3, 25.0)] {
        shell.click_command(AppCommand::Spline);
        for [x, dy] in [[15.0, 0.0], [30.0, 0.0], [30.0, 10.0], [15.0, 8.0]] {
            shell.click_at(at(&shell, x, y + dy));
        }
        shell.press_key(finish);
        assert_eq!(
            shell.app().document_revision(),
            before_revision + revision,
            "{finish:?} closes the spline"
        );
        assert_eq!(latest_spline(&shell).len(), 4);
    }
}

/// World box of a root occurrence: its lowest corner and its size.
fn occurrence_box(shell: &Shell, id: u64) -> (Vec3, Vec3) {
    shell
        .app()
        .occurrence_box_geometry(id)
        .unwrap_or_else(|| panic!("occurrence {id} has a box"))
}

fn assert_near(actual: Vec3, expected: Vec3, what: &str) {
    assert!(
        actual.distance(expected) < 1.0e-6,
        "{what}: {actual:?} is not {expected:?}"
    );
}

#[test]
fn mirror_copies_the_selection_across_the_clicked_face_or_a_typed_offset_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.offers(AppCommand::Mirror), "Mirror is in the rail");
    assert_eq!(shell.app().occurrence_count(), 1);
    let (origin, size) = occurrence_box(&shell, 1);
    let top_centre = origin + Vec3::new(size.x / 2.0, size.y / 2.0, size.z);

    // The shortcut picks the tool once something is selected.
    shell.key(Key::A, ctrl());
    shell.press_key(Key::I);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-mirror"))]),
        )
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    // Hovering the top face previews the copy above it without editing.
    let top = shell.app().viewport_position(top_centre).unwrap();
    shell.move_pointer(top);
    let preview = shell.app().mirror_preview_corners();
    assert_eq!(preview.len(), 1, "one mirrored box is previewed");
    let lowest = preview[0].iter().fold(
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        |low, corner| {
            Vec3::new(
                low.x.min(corner.x),
                low.y.min(corner.y),
                low.z.min(corner.z),
            )
        },
    );
    assert_near(
        lowest,
        origin + Vec3::new(0.0, 0.0, size.z),
        "preview corner",
    );
    assert_eq!(shell.app().document_revision(), before_revision);

    // A click adds the mirrored copy as one Undo step.
    shell.click_at(top);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().occurrence_count(), 2);
    let (copy_origin, copy_size) = occurrence_box(&shell, 2);
    assert_near(
        copy_origin,
        origin + Vec3::new(0.0, 0.0, size.z),
        "copy origin",
    );
    assert_near(copy_size, size, "copy size");
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(ketchup_model::document::OccurrenceId(2)),
        shell
            .app()
            .occurrence_definition_id(ketchup_model::document::OccurrenceId(1)),
        "the copy uses the same definition"
    );
    let first_digest = shell.app().canonical_digest();
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);

    // A typed offset moves the plane off the face before Enter mirrors.
    shell.key(Key::Z, ctrl());
    shell.move_pointer(top);
    shell.type_text("10");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().occurrence_count(), 2);
    let (offset_origin, _) = occurrence_box(&shell, 2);
    assert_near(
        offset_origin,
        origin + Vec3::new(0.0, 0.0, size.z + 20.0),
        "offset copy origin",
    );

    // Escape leaves the tool without touching the document.
    let second_digest = shell.app().canonical_digest();
    shell.move_pointer(top);
    assert!(!shell.app().mirror_preview_corners().is_empty());
    shell.press_key(Key::Escape);
    assert!(shell.app().mirror_preview_corners().is_empty());
    assert_eq!(shell.app().canonical_digest(), second_digest);
}
