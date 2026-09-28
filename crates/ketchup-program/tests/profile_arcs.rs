//! Circular arcs in extrude/revolve profiles and `round_corners`.
use ketchup_program::model::{Part, ProgramPartBody, ProgramProfileSegment};
use ketchup_program::run;
use std::collections::BTreeMap;

fn part(source: &str, name: &str) -> Part {
    let (model, _) = run("arcs.star", source, &BTreeMap::new()).unwrap();
    model.model.part(name).unwrap().clone()
}

fn error(source: &str) -> String {
    run("arcs.star", source, &BTreeMap::new())
        .unwrap_err()
        .message
}

fn segments(part: &Part) -> &[ProgramProfileSegment] {
    match &part.body {
        ProgramPartBody::Extrusion { segments, .. } | ProgramPartBody::Revolve { segments, .. } => {
            segments
        }
        ProgramPartBody::Panel => panic!("not a profile part"),
    }
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

#[test]
fn round_corners_rounds_every_corner_with_a_tangent_arc() {
    let top = part(
        "extrude(\"top\", distance = 18, \
         profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40))",
        "top",
    );
    let segments = segments(&top);
    let names: Vec<_> = segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "segment1", "corner2", "segment2", "corner3", "segment3", "corner4", "segment4",
            "corner1"
        ]
    );
    let centres: Vec<_> = segments
        .iter()
        .filter_map(|s| s.arc.map(|arc| (arc.center_mm, arc.clockwise)))
        .collect();
    for ((centre, clockwise), expected) in
        centres
            .iter()
            .zip([[760.0, 40.0], [760.0, 460.0], [40.0, 460.0], [40.0, 40.0]])
    {
        assert!(close(*centre, expected), "{centre:?} vs {expected:?}");
        assert!(!clockwise);
    }
    assert_eq!(segments[0].start_mm, [40.0, 0.0]);
    assert_eq!(segments[0].end_mm, [760.0, 0.0]);
    // The loop is closed: every segment starts where the previous one ends.
    for pair in segments.windows(2) {
        assert!(close(pair[0].end_mm, pair[1].start_mm), "{pair:?}");
    }
    assert!(close(segments[7].end_mm, segments[0].start_mm));
    let (min, max) = top.local_bounds();
    assert!(min.iter().all(|v| v.abs() < 1e-9) && max == [800.0, 500.0, 18.0]);
}

#[test]
fn round_corners_keeps_zero_radius_corners_sharp_and_names_sides() {
    let top = part(
        "extrude(\"top\", distance = 18, profile = round_corners(\
         [[0, 0], [800, 0], [800, 500], [0, 500]], [0, 100, 0, 0], \
         names = [\"front\", \"right\", \"back\", \"left\"]))",
        "top",
    );
    let names: Vec<_> = segments(&top).iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["front", "corner2", "right", "back", "left"]);
    assert!(
        error("round_corners([[0, 0], [100, 0], [100, 50], [0, 50]], 30)")
            .contains("side \"segment2\" is 50.0 mm")
    );
}

#[test]
fn an_arc_through_a_point_by_radius_and_by_centre_is_the_same_arc() {
    // Through (300, 60) from (0, 0) to (600, 0): centre (300, -720).
    let arc = |form: &str| {
        let source = format!(
            "extrude(\"a\", distance = 20, profile = [[\"top\", [600, 120], [0, 120]], \
             [\"left\", [0, 120], [0, 0]], [\"arch\", [0, 0], [600, 0], {form}], \
             [\"right\", [600, 0], [600, 120]]])"
        );
        segments(&part(&source, "a"))[2].arc.unwrap()
    };
    let through = arc("{\"through\": (300, 60)}");
    assert!(close(through.center_mm, [300.0, -720.0]));
    assert!(through.clockwise);
    let radius = arc("{\"radius\": 780, \"clockwise\": True}");
    assert!(close(radius.center_mm, through.center_mm), "{radius:?}");
    assert_eq!(radius.clockwise, through.clockwise);
    let large = arc("{\"radius\": 780, \"clockwise\": True, \"large\": True}");
    assert!(close(large.center_mm, [300.0, 720.0]), "{large:?}");
    let centre = arc("{\"center\": (300, -720), \"clockwise\": True}");
    assert_eq!(centre, through);
}

#[test]
fn arcs_that_bulge_out_extend_the_bounds() {
    let part = part(
        "extrude(\"a\", distance = 20, profile = [[\"top\", [600, 120], [0, 120]], \
         [\"left\", [0, 120], [0, 0]], [\"belly\", [0, 0], [600, 0], {\"through\": (300, -60)}], \
         [\"right\", [600, 0], [600, 120]]])",
        "a",
    );
    let (min, max) = part.local_bounds();
    assert!((min[1] + 60.0).abs() < 1e-6, "{min:?}");
    assert_eq!(max, [600.0, 120.0, 20.0]);
    assert!((part.size_mm[1] - 180.0).abs() < 1e-6);
}

#[test]
fn a_revolved_half_disc_is_a_ball_of_its_radius() {
    let ball = part(
        "revolve(\"ball\", axis = [[0, -50], [0, 50]], profile = [\
         [\"axis\", [0, -50], [0, 50]], \
         [\"skin\", [0, 50], [0, -50], {\"center\": (0, 0), \"clockwise\": True}]])",
        "ball",
    );
    assert_eq!(ball.size_mm, [100.0, 100.0, 100.0]);
    let (min, max) = ball.local_bounds();
    assert_eq!((min, max), ([-50.0; 3], [50.0; 3]));
}

#[test]
fn impossible_arcs_are_rejected_with_the_reason() {
    let with = |arc: &str| {
        error(&format!(
            "extrude(\"a\", distance = 5, profile = [[\"line\", [0, 0], [100, 0]], \
             [\"arc\", [100, 0], [0, 0], {arc}]])"
        ))
    };
    assert!(with("{\"radius\": 40}").contains("smaller than half"));
    assert!(with("{\"through\": (50, 0)}").contains("use a line"));
    assert!(with("{\"center\": (10, 10)}").contains("equally far"));
    assert!(with("{\"bulge\": 3}").contains("unknown arc key"));
    assert!(with("{\"through\": (50, 20), \"clockwise\": True}").contains("takes no clockwise"));
}
