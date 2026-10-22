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
