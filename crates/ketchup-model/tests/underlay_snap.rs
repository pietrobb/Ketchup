//! Snapping to and measuring on underlay lines: the ends and middles of lines and arcs,
//! the nearest point on them, the length of one, and only on layers that are shown.
use ketchup_geometry::linalg::point_segment_distance2;
use ketchup_model::document::ProfileSegment;
use ketchup_model::underlay::snap::{UnderlaySnapIndex, UnderlaySnapKind};
use ketchup_model::underlay::{UnderlayLines, UnderlayLinesBuilder};

fn line(start_mm: [f64; 2], end_mm: [f64; 2]) -> ProfileSegment {
    ProfileSegment::Line { start_mm, end_mm }
}

fn arc(start_mm: [f64; 2], end_mm: [f64; 2], center_mm: [f64; 2]) -> ProfileSegment {
    ProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        clockwise: false,
    }
}

/// A line, a half circle of radius 500 bulging to x = 1500 and a line back, as one
/// polyline on "walls"; a loose full circle on "walls"; one line on "axes".
fn drawing() -> UnderlayLines {
    let mut builder = UnderlayLinesBuilder::default();
    builder.add(
        "walls",
        &[
            line([0.0, 0.0], [1000.0, 0.0]),
            arc([1000.0, 0.0], [1000.0, 1000.0], [1000.0, 500.0]),
            line([1000.0, 1000.0], [0.0, 1000.0]),
        ],
    );
    builder.add_loose("walls", &arc([3200.0, 0.0], [3200.0, 0.0], [3000.0, 0.0]));
    builder.add("axes", &[line([0.0, 500.0], [800.0, 500.0])]);
    builder.finish().1
}

fn close(a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= tolerance
}

#[test]
fn line_and_arc_ends_snap_and_the_chords_inside_an_arc_do_not() {
    let index = UnderlaySnapIndex::new(&drawing());
    let all = |_| true;
    for corner in [[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]] {
        let snaps = index.snaps([corner[0] + 3.0, corner[1] + 4.0], 10.0, all);
        let endpoint = snaps.endpoint.expect("a corner is an endpoint");
        assert_eq!(endpoint.point_mm, corner);
        assert_eq!(endpoint.distance_mm, 5.0);
    }
    // On the arc, between its ends: its chords meet there, but no line or arc ends.
    let on_arc = [
        1000.0 + 500.0 * 0.3_f64.cos(),
        500.0 + 500.0 * 0.3_f64.sin(),
    ];
    assert_eq!(index.snaps(on_arc, 20.0, all).endpoint, None);
    // The circle has no ends.
    assert_eq!(index.snaps([3200.0, 0.0], 20.0, all).endpoint, None);
}

#[test]
fn a_line_and_an_arc_snap_at_their_middle_a_circle_does_not() {
    let index = UnderlaySnapIndex::new(&drawing());
    let all = |_| true;
    let middle = index.snaps([500.0, 8.0], 10.0, all).midpoint.unwrap();
    assert_eq!(middle.point_mm, [500.0, 0.0]);
    assert_eq!(middle.kind, UnderlaySnapKind::Midpoint);
    // Halfway along the half circle, on its chords: within their 0.1 mm of the arc.
    let middle = index.snaps([1505.0, 500.0], 10.0, all).midpoint.unwrap();
    assert!(close(middle.point_mm, [1500.0, 500.0], 0.1), "{middle:?}");
    for at in [
        [3200.0, 0.0],
        [2800.0, 0.0],
        [3000.0, 200.0],
        [3000.0, -200.0],
    ] {
        assert_eq!(index.snaps(at, 20.0, all).midpoint, None);
    }
}

#[test]
fn the_nearest_point_lies_on_the_line_or_arc_within_the_distance_asked_for() {
    let index = UnderlaySnapIndex::new(&drawing());
    let all = |_| true;
    let nearest = index.snaps([300.0, 7.0], 10.0, all).nearest.unwrap();
    assert_eq!(nearest.point_mm, [300.0, 0.0]);
    assert_eq!(nearest.distance_mm, 7.0);
    // 100 mm outside the arc's crest.
    let nearest = index.snaps([1600.0, 500.0], 150.0, all).nearest.unwrap();
    assert!(close(nearest.point_mm, [1500.0, 500.0], 0.1), "{nearest:?}");
    assert!((nearest.distance_mm - 100.0).abs() <= 0.1);
    // Beyond the distance asked for, nothing.
    assert_eq!(index.snaps([1600.0, 500.0], 90.0, all).nearest, None);
    let circle = index.snaps([3000.0, 250.0], 60.0, all).nearest.unwrap();
    assert!(close(circle.point_mm, [3000.0, 200.0], 0.1), "{circle:?}");
}

#[test]
fn a_hidden_layer_gives_no_snap() {
    let index = UnderlaySnapIndex::new(&drawing());
    // The axis line at y = 500 is on layer 1; the walls are 200 mm away and more.
    let near_axis = [400.0, 502.0];
    let walls_only = index.snaps(near_axis, 10.0, |layer| layer == 0);
    assert_eq!(walls_only.iter().count(), 0);
    let shown = index.snaps(near_axis, 10.0, |_| true);
    let snap = shown.midpoint.unwrap();
    assert_eq!((snap.layer, snap.point_mm), (1, [400.0, 500.0]));
}

#[test]
fn a_line_or_an_arc_measures_from_corner_to_corner() {
    let index = UnderlaySnapIndex::new(&drawing());
    let all = |_| true;
    let on_line = index.snaps([700.0, 1.0], 5.0, all).nearest.unwrap();
    let piece = index.piece(on_line.segment).unwrap();
    assert_eq!(piece.points_mm, [[0.0, 0.0], [1000.0, 0.0]]);
    assert_eq!(piece.length_mm, 1000.0);
    let on_arc = index.snaps([1500.0, 500.0], 5.0, all).nearest.unwrap();
    let piece = index.piece(on_arc.segment).unwrap();
    assert_eq!(piece.points_mm.first(), Some(&[1000.0, 0.0]));
    assert_eq!(piece.points_mm.last(), Some(&[1000.0, 1000.0]));
    // Half the circumference of radius 500, along chords at most 0.1 mm off the arc.
    let half = std::f64::consts::PI * 500.0;
    assert!(
        piece.length_mm < half && half - piece.length_mm < 0.5,
        "{}",
        piece.length_mm
    );
    let circle = index.snaps([3000.0, 200.0], 5.0, all).nearest.unwrap();
    let circle = index.piece(circle.segment).unwrap();
    let whole = std::f64::consts::TAU * 200.0;
    assert!(
        (whole - circle.length_mm).abs() < 0.5,
        "{}",
        circle.length_mm
    );
}

#[test]
fn lines_and_arcs_snap_where_they_cross_or_one_ends_on_another() {
    let mut builder = UnderlayLinesBuilder::default();
    // A cross at (500, 500), on two layers.
    builder.add("walls", &[line([0.0, 500.0], [1000.0, 500.0])]);
    builder.add("axes", &[line([500.0, 0.0], [500.0, 1000.0])]);
    // A T: a line ending on the wall at (2000, 500).
    builder.add("walls", &[line([1500.0, 500.0], [2500.0, 500.0])]);
    builder.add("walls", &[line([2000.0, 500.0], [2000.0, 900.0])]);
    // Two separate lines meeting end to end at (4000, 0): an end, not a crossing.
    builder.add("walls", &[line([3000.0, 0.0], [4000.0, 0.0])]);
    builder.add("walls", &[line([4000.0, 0.0], [4000.0, 1000.0])]);
    // A circle of radius 200 about (6000, 0), crossed by a line along y = 0.
    builder.add_loose("walls", &arc([6200.0, 0.0], [6200.0, 0.0], [6000.0, 0.0]));
    builder.add("walls", &[line([5500.0, 0.0], [6500.0, 0.0])]);
    let index = UnderlaySnapIndex::new(&builder.finish().1);
    let all = |_| true;
    let cross = index.snaps([503.0, 504.0], 10.0, all).intersection.unwrap();
    assert_eq!(cross.kind, UnderlaySnapKind::Intersection);
    assert!(close(cross.point_mm, [500.0, 500.0], 1e-9), "{cross:?}");
    assert!((cross.distance_mm - 5.0).abs() < 1e-9);
    let tee = index
        .snaps([2002.0, 498.0], 10.0, all)
        .intersection
        .unwrap();
    assert!(close(tee.point_mm, [2000.0, 500.0], 1e-9), "{tee:?}");
    let corner = index.snaps([4001.0, 1.0], 10.0, all);
    assert_eq!(corner.intersection, None);
    assert_eq!(corner.endpoint.unwrap().point_mm, [4000.0, 0.0]);
    // The chords of the circle meet each other everywhere along it, but cross nothing.
    let on_circle = [6000.0 + 200.0 * 1.0_f64.cos(), 200.0 * 1.0_f64.sin()];
    assert_eq!(index.snaps(on_circle, 20.0, all).intersection, None);
    // Where the line crosses the circle, on its chords: the chords end on y = 0 there.
    for x in [5800.0, 6200.0] {
        let hit = index.snaps([x + 3.0, 4.0], 10.0, all).intersection.unwrap();
        assert!(close(hit.point_mm, [x, 0.0], 0.1), "{hit:?}");
    }
    // A crossing needs both lines shown.
    let walls_only = index.snaps([503.0, 504.0], 10.0, |layer| layer == 0);
    assert_eq!(walls_only.intersection, None);
    assert!(walls_only.nearest.is_some());
    // Nothing beyond the distance asked for.
    assert_eq!(index.snaps([520.0, 520.0], 10.0, all).intersection, None);
}

#[test]
fn the_grid_finds_what_looking_at_every_line_finds() {
    // 20,000 short lines scattered over 100 m, and a few long ones across all of it.
    let mut seed = 0x2545_f491_u64;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    let mut builder = UnderlayLinesBuilder::default();
    let mut segments = Vec::new();
    for index in 0..20_000 {
        let start = [next() * 100_000.0, next() * 100_000.0];
        let length = if index % 1000 == 0 { 100_000.0 } else { 500.0 };
        let angle = next() * std::f64::consts::TAU;
        let end = [
            start[0] + length * angle.cos(),
            start[1] + length * angle.sin(),
        ];
        builder.add("0", &[line(start, end)]);
        segments.push([start, end]);
    }
    let index = UnderlaySnapIndex::new(&builder.finish().1);
    assert_eq!(index.segment_count(), 20_000);
    for _ in 0..500 {
        let point = [next() * 100_000.0, next() * 100_000.0];
        let expected = segments
            .iter()
            .map(|[start, end]| point_segment_distance2(point, *start, *end))
            .fold(f64::INFINITY, f64::min);
        let found = index.snaps(point, 400.0, |_| true).nearest;
        // Far from the 400 mm edge, so that rounding cannot put a line on either side.
        if expected <= 399.0 {
            assert!((found.unwrap().distance_mm - expected).abs() < 1e-6);
        } else if expected > 401.0 {
            assert_eq!(found, None);
        }
    }
}
