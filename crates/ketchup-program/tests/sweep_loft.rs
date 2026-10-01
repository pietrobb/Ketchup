//! `sweep()` along smooth paths and `loft()` through stacked sections.
use ketchup_program::model::{Part, ProgramPartBody, ProgramPathSegment};
use ketchup_program::path::{PolylineError, polyline};
use ketchup_program::run;
use std::collections::BTreeMap;

const RAIL: &str = "sweep(\"rail\", profile = [(-15, 0), (15, 0), (15, 20), (-15, 20)], \
                    path = [(0, 0, 0), (600, 0, 0), (600, 400, 0)], bend = 100)\n";

fn part(source: &str, name: &str) -> Part {
    let (model, _) = run("sweep.star", source, &BTreeMap::new()).unwrap();
    model.model.part(name).unwrap().clone()
}

fn error(source: &str) -> String {
    run("sweep.star", source, &BTreeMap::new())
        .unwrap_err()
        .message
}

fn path(part: &Part) -> &[ProgramPathSegment] {
    match &part.body {
        ProgramPartBody::Sweep { path, .. } => path,
        _ => panic!("not a sweep"),
    }
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|axis| (a[axis] - b[axis]).abs() < 1e-9)
}

#[test]
fn bend_rounds_a_path_corner_with_a_tangent_arc() {
    let rail = part(RAIL, "rail");
    let path = path(&rail);
    assert_eq!(path.len(), 3);
    assert!(close(path[0].end_mm, [500.0, 0.0, 0.0]), "{path:?}");
    let arc = path[1].arc.expect("the corner is an arc");
    assert!(close(arc.center_mm, [500.0, 100.0, 0.0]), "{arc:?}");
    assert!(close(arc.normal, [0.0, 0.0, 1.0]), "{arc:?}");
    assert!(close(path[2].start_mm, [600.0, 100.0, 0.0]), "{path:?}");
    assert!(close(path[2].end_mm, [600.0, 400.0, 0.0]), "{path:?}");
}

#[test]
fn the_swept_body_reaches_exactly_as_far_as_its_section_goes() {
    // Going +x the profile's u is -y and v is +z; after the quarter turn u is +x.
    let rail = part(RAIL, "rail");
    let (min, max) = rail.local_bounds();
    assert!(close(min, [0.0, -15.0, 0.0]), "{min:?}");
    assert!(close(max, [615.0, 400.0, 20.0]), "{max:?}");
    // Diagonally the outer edge of the bend (radius 115 about (500, 100)) leads.
    let diagonal = [
        std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
        0.0,
    ];
    let expected = 400.0 * std::f64::consts::FRAC_1_SQRT_2 + 115.0;
    assert!(
        (rail.reach(diagonal) - expected).abs() < 1e-6,
        "{}",
        rail.reach(diagonal)
    );
}

#[test]
fn a_path_rising_vertically_stands_the_profile_in_the_floor_plane() {
    // Along +z the profile's u is -x and v is +y.
    let post = part(
        "sweep(\"post\", profile = [(0, 0), (30, 0), (30, 10), (0, 10)], \
         path = [(0, 0, 0), (0, 0, 500)])",
        "post",
    );
    let (min, max) = post.local_bounds();
    assert!(close(min, [-30.0, 0.0, 0.0]), "{min:?}");
    assert!(close(max, [0.0, 10.0, 500.0]), "{max:?}");
}

#[test]
fn explicit_segments_join_a_tangent_arc_through_a_point() {
    let s = (0.5_f64).sqrt();
    let hook = part(
        &format!(
            "sweep(\"hook\", profile = [(-5, -5), (5, -5), (5, 5), (-5, 5)], path = [\
             [(0, 0, 0), (100, 0, 0)], \
             [(100, 0, 0), (200, 100, 0), {{\"through\": ({}, {}, 0)}}]])",
            100.0 + 100.0 * s,
            100.0 - 100.0 * s
        ),
        "hook",
    );
    let arc = path(&hook)[1].arc.unwrap();
    assert!(close(arc.center_mm, [100.0, 100.0, 0.0]), "{arc:?}");
    let (_, max) = hook.local_bounds();
    assert!((max[0] - 205.0).abs() < 1e-6, "{max:?}");
}

#[test]
fn a_path_corner_or_a_kinked_arc_is_refused_with_the_fix() {
    let corner = error(
        "sweep(\"r\", profile = [(0, 0), (10, 0), (10, 10)], \
         path = [(0, 0, 0), (100, 0, 0), (100, 100, 0)])",
    );
    assert!(corner.contains("bend=<radius>"), "{corner}");
    let kink = error(
        "sweep(\"r\", profile = [(0, 0), (10, 0), (10, 10)], path = [\
         [(0, 0, 0), (100, 0, 0)], \
         [(100, 0, 0), (200, 0, 0), {\"through\": (150, 30, 0)}]])",
    );
    assert!(kink.contains("corner") && kink.contains("smooth"), "{kink}");
    let tight = error(
        "sweep(\"r\", profile = [(0, 0), (10, 0), (10, 10)], \
         path = [(0, 0, 0), (100, 0, 0), (100, 100, 0)], bend = 150)",
    );
    assert!(tight.contains("does not fit"), "{tight}");
}

#[test]
fn a_loft_tapers_between_its_sections_and_orders_them_upwards() {
    let leg = part(
        "loft(\"leg\", sections = [([(0, 0), (40, 0), (40, 40), (0, 40)], 0), \
         ([(8, 8), (32, 8), (32, 32), (8, 32)], 700)], at = (100, 0, 0))",
        "leg",
    );
    assert_eq!(leg.size_mm, [40.0, 40.0, 700.0]);
    let (min, max) = leg.world_bounds();
    assert!(close(min, [100.0, 0.0, 0.0]) && close(max, [140.0, 40.0, 700.0]));
    let unordered = error(
        "loft(\"leg\", sections = [([(0, 0), (40, 0), (40, 40)], 10), \
         ([(0, 0), (40, 0), (40, 40)], 10)])",
    );
    assert!(unordered.contains("above the previous"), "{unordered}");
    let cut = error(
        "leg = loft(\"leg\", sections = [([(0, 0), (40, 0), (40, 40)], 0), \
         ([(0, 0), (40, 0), (40, 40)], 50)])\n\
         cut(leg, profile = [(0, 0), (5, 0), (5, 5)], depth = 2, name = \"c\")",
    );
    assert!(cut.contains("subtract()"), "{cut}");
}

#[test]
fn a_point_path_refusal_names_its_kind_and_the_offending_point() {
    let flat = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0]];
    assert_eq!(polyline(&flat[..1], 0.0), Err(PolylineError::TooFewPoints));
    assert_eq!(
        polyline(&flat, 0.0),
        Err(PolylineError::SharpCorners { corners: 1 })
    );
    let back = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [50.0, 0.0, 0.0]];
    assert!(matches!(
        polyline(&back, 10.0),
        Err(PolylineError::Reverses { point: 2, at }) if at == [100.0, 0.0, 0.0]
    ));
    let Err(PolylineError::BendDoesNotFit {
        from_point: 1,
        needed_mm,
        side_mm,
        ..
    }) = polyline(&flat, 200.0)
    else {
        panic!("a 200 mm bend cannot fit a 100 mm side");
    };
    assert!(needed_mm > side_mm, "{needed_mm} <= {side_mm}");
}

const SPRING: &str = "sweep(\"spring\", profile = [(-1, -1), (1, -1), (1, 1), (-1, 1)], \
                      path = helix(radius = 20, pitch = 10, turns = 1))\n";

#[test]
fn helix_is_a_path_of_tangent_cubic_quarter_turns() {
    let spring = part(SPRING, "spring");
    let path = path(&spring);
    assert_eq!(path.len(), 4);
    assert!(
        path.iter()
            .all(|segment| segment.bezier.is_some() && segment.arc.is_none())
    );
    assert!(close(path[0].start_mm, [20.0, 0.0, 0.0]), "{path:?}");
    assert!(close(path[1].start_mm, [0.0, 20.0, 2.5]), "{path:?}");
    assert!(close(path[3].end_mm, [20.0, 0.0, 10.0]), "{path:?}");
    for pair in path.windows(2) {
        assert_eq!(pair[0].end_mm, pair[1].start_mm);
    }
    // One turn of a 20 mm helix rising 10 mm is sqrt((2 pi 20)^2 + 10^2) long.
    let expected = (std::f64::consts::TAU * 20.0).hypot(10.0);
    let total: f64 = path.iter().map(ProgramPathSegment::length).sum();
    assert!((total - expected).abs() < 0.1, "{total} vs {expected}");
}

#[test]
fn a_profile_swept_along_a_helix_reaches_just_past_its_turns() {
    // The 2 mm square stays within its half diagonal of the helix.
    let (min, max) = part(SPRING, "spring").local_bounds();
    for (value, low, high) in [
        (max[0], 21.0, 21.5),
        (max[1], 21.0, 21.5),
        (min[0], -21.5, -21.0),
        (min[1], -21.5, -21.0),
        (min[2], -1.5, -0.9),
        (max[2], 10.9, 11.5),
    ] {
        assert!((low..=high).contains(&value), "{min:?} {max:?}");
    }
}

#[test]
fn helix_refuses_more_quarter_turns_than_a_path_holds() {
    let message = error("p = helix(radius = 5, pitch = 1, turns = 17)\n");
    assert!(message.contains("68 quarter turns"), "{message}");
    assert!(error("p = helix(radius = 0, pitch = 1, turns = 1)\n").contains("must be positive"));
}
