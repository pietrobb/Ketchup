//! PG-12 (review 2026-10-18): member() without across= keeps a sloping bar's
//! height in the vertical plane at every slope.
use ketchup_program::run;
use std::collections::BTreeMap;

/// A 60x200 rafter 4 m long, rising along x at `slope` degrees.
const RAFTER: &str = "\
SLOPE = param(\"slope\", 25, min = 0, max = 90)
a = SLOPE * math.pi / 180
member(\"rafter\", (0, 0, 0), (4000 * math.cos(a), 0, 4000 * math.sin(a)), (60, 200))
";

/// The rafter's extent across the roof, along world y.
fn width_across(slope: f64) -> f64 {
    let overrides = BTreeMap::from([("slope".to_owned(), slope)]);
    let evaluated = run("rafter.star", RAFTER, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .0;
    let (min, max) = evaluated.model.part("rafter").unwrap().world_bounds();
    max[1] - min[1]
}

/// How far local x of a 4 m bar at `plan` degrees from world x in plan,
/// rising at `slope` degrees, points along (-sin plan, cos plan, 0), the
/// horizontal square to it to the left.
fn across_left(plan: f64, slope: f64) -> f64 {
    let source = "\
P = param(\"plan\", 0, min = -180, max = 180)
S = param(\"slope\", 0, min = 0, max = 89)
p = P * math.pi / 180
s = S * math.pi / 180
member(\"bar\", (0, 0, 0), (4000 * math.cos(s) * math.cos(p), 4000 * math.cos(s) * math.sin(p), 4000 * math.sin(s)), (60, 200))
";
    let overrides = BTreeMap::from([("plan".to_owned(), plan), ("slope".to_owned(), slope)]);
    let evaluated = run("bar.star", source, &overrides)
        .unwrap_or_else(|error| panic!("{error}"))
        .0;
    let bar = evaluated.model.part("bar").unwrap();
    let origin = bar.to_local([0.0, 0.0, 0.0])[0];
    let along = |world: [f64; 3]| bar.to_local(world)[0] - origin;
    let p = plan.to_radians();
    -p.sin() * along([1.0, 0.0, 0.0]) + p.cos() * along([0.0, 1.0, 0.0])
}

/// PG-27 (review 2026-10-24): local x turned over at about 25.8 degrees in
/// plan, and for a sloping bar at a plan angle that depended on its slope:
/// a rafter 5 degrees off world x rising at 30 degrees had it the other way
/// round from one along world x. As before for level bars, local x points
/// towards +y for a bar more along world x than y in plan and towards +x for
/// one more along y, at any slope: it turns over only at 45 degrees in plan.
#[test]
fn local_x_turns_over_only_where_a_bar_turns_from_along_x_to_along_y_in_plan() {
    for slope in [0.0, 30.0, 60.0] {
        for plan in [0.0, 5.0, -5.0, 30.0, 40.0, 44.9, -60.0, 225.1, 270.0] {
            let left = across_left(plan, slope);
            assert!(
                (left - 1.0).abs() < 1e-9,
                "plan {plan}, slope {slope}: local x {left} to the left"
            );
        }
        for plan in [45.1, 60.0, 90.0, 120.0, 140.0, 180.0, 210.0] {
            let left = across_left(plan, slope);
            assert!(
                (left + 1.0).abs() < 1e-9,
                "plan {plan}, slope {slope}: local x {left} to the left"
            );
        }
    }
}

/// Steeper than about 25.8 degrees the default across was world x, so the
/// rafter turned its 200 mm side across the roof and lay flat.
#[test]
fn a_rafter_stands_on_its_narrow_side_at_every_slope() {
    for slope in [0.0, 25.0, 27.0, 45.0, 60.0, 85.0] {
        let width = width_across(slope);
        assert!(
            (width - 60.0).abs() < 1e-6,
            "at {slope} degrees the rafter is {width} mm across the roof, not 60"
        );
    }
}
