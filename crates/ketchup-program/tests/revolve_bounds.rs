//! CR-09: support and broad phase must contain the actual swept material.
use ketchup_program::{evaluate, exact_candidates};

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1.0e-7, "{a} != {b}");
}

#[test]
fn x_axis_revolution_keeps_the_lower_wall_collision_candidate() {
    let evaluated = evaluate(
        "bounds.star",
        r#"
p = revolve("tube", profile=[(0,10),(100,10),(100,20),(0,20)], axis=[(0,0),(1,0)])
b = box("inside_wall", (5,2,2), at=(40,-19,-1))
"#,
        &Default::default(),
    )
    .expect("valid program");
    let part = evaluated.model.part("tube").expect("tube");
    let (min, max) = part.local_bounds();
    for (actual, expected) in min.into_iter().zip([0.0, -20.0, -20.0]) {
        close(actual, expected);
    }
    for (actual, expected) in max.into_iter().zip([100.0, 20.0, 20.0]) {
        close(actual, expected);
    }
    close(part.reach([0.0, -1.0, 0.0]), 20.0);
    assert_eq!(part.size_mm, [100.0, 40.0, 40.0]);
    assert!(exact_candidates(&evaluated.model).contains("tube"));
}

#[test]
fn shifted_oblique_axis_partial_turn_and_rigid_placement_preserve_support() {
    // Triangle in axial/radial coordinates (0,10), (80,10), (0,25),
    // mapped around an offset diagonal axis; not a rectangular profile.
    let source = r#"
p = revolve("p", profile=[(2,12),(50,76),(-10,21)], axis=[(10,6),(13,10)], angle=123)
rotate(p, axis=(1,2,3), angle=37, pivot=(0,0,0))
move(p, by=(101,-52,17))
"#;
    let evaluated = evaluate("bounds.star", source, &Default::default()).expect("valid program");
    let part = evaluated.model.part("p").expect("part");
    let (min, max) = part.local_bounds();
    let (world_min, world_max) = part.world_bounds();
    for point in [[2.0, 12.0], [50.0, 76.0], [-10.0, 21.0]] {
        let delta = [point[0] - 10.0, point[1] - 6.0];
        let height = 0.6 * delta[0] + 0.8 * delta[1];
        let radius = -0.8 * delta[0] + 0.6 * delta[1];
        for step in 0..=246 {
            let angle = (f64::from(step) * 0.5).to_radians();
            let local = [
                10.0 + 0.6 * height - 0.8 * radius * angle.cos(),
                6.0 + 0.8 * height + 0.6 * radius * angle.cos(),
                radius * angle.sin(),
            ];
            let world = part.to_world(local);
            for i in 0..3 {
                assert!(local[i] >= min[i] - 1.0e-7 && local[i] <= max[i] + 1.0e-7);
                assert!(world[i] >= world_min[i] - 1.0e-7 && world[i] <= world_max[i] + 1.0e-7);
            }
        }
    }
    // Independent dense angular oracle; the maximum over a linear profile is at a vertex.
    for direction in [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]] {
        let mut sampled = f64::NEG_INFINITY;
        for point in [[2.0, 12.0], [50.0, 76.0], [-10.0, 21.0]] {
            let delta = [point[0] - 10.0, point[1] - 6.0];
            let h = 0.6 * delta[0] + 0.8 * delta[1];
            let r = -0.8 * delta[0] + 0.6 * delta[1];
            for step in 0..=12300 {
                let t = (f64::from(step) * 0.01).to_radians();
                let w = part.to_world([
                    10.0 + 0.6 * h - 0.8 * r * t.cos(),
                    6.0 + 0.8 * h + 0.6 * r * t.cos(),
                    r * t.sin(),
                ]);
                sampled = sampled.max((0..3).map(|i| w[i] * direction[i]).sum());
            }
        }
        let exact = part.reach(direction);
        assert!(
            exact >= sampled - 1.0e-7 && exact - sampled < 1.0e-5,
            "{exact} vs {sampled}"
        );
    }
}
