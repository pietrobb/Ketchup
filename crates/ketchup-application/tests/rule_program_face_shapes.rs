//! Any closed profile milled into or standing out of any face of any part,
//! checked on the exact solid: volume against the profile area, the pocket
//! floor where the face and depth put it, and bad requests refused.

mod operations_support;

use ketchup_scheduler::ExactWorkerSupervisor;
use operations_support::*;

/// A triangle that fits every face of the 100 x 60 x 40 block; area 600,
/// centroid (30, 15) in face coordinates.
const TRIANGLE: &str = "[(10, 5), (50, 5), (30, 35)]";
const AREA: f64 = 600.0;
const SIZE: [f64; 3] = [A, B, C];

/// (normal axis, u axis, v axis) of a face name, as the program defines them.
fn axes(face: &str) -> (usize, usize, usize) {
    match &face[..1] {
        "x" => (0, 1, 2),
        "y" => (1, 0, 2),
        _ => (2, 0, 1),
    }
}

/// Local point `depth` mm inside `face` at face coordinates (u, v).
fn inside(face: &str, u: f64, v: f64, depth: f64) -> [f64; 3] {
    let (n, u_axis, v_axis) = axes(face);
    let mut point = [0.0; 3];
    point[u_axis] = u;
    point[v_axis] = v;
    point[n] = if face.ends_with('+') {
        SIZE[n] - depth
    } else {
        depth
    };
    point
}

/// Whether every one of `corners` is a vertex of the exact solid of `part`.
fn has_corners(
    worker: &mut ExactWorkerSupervisor,
    program: &str,
    part: &str,
    corners: &[[f64; 3]],
) -> bool {
    let package = solid(worker, program, part).unwrap();
    corners.iter().all(|corner| {
        package.vertices.iter().any(|vertex| {
            (0..3).all(|axis| (vertex.position_mm[axis] - corner[axis]).abs() < 1.0e-6)
        })
    })
}

/// The triangle's corners `depth` mm inside `face` (negative: outside).
fn triangle_at(face: &str, depth: f64) -> [[f64; 3]; 3] {
    [(10.0, 5.0), (50.0, 5.0), (30.0, 35.0)].map(|(u, v)| inside(face, u, v, depth))
}

#[test]
fn a_profile_pocket_lands_on_each_of_the_six_faces() {
    let mut worker = worker();
    for face in ["x-", "x+", "y-", "y+", "z-", "z+"] {
        let program = format!("{BLOCK}pocket_shape(block, \"{face}\", {TRIANGLE}, 6)");
        assert_volume(
            volume(&mut worker, &program, "block"),
            A * B * C - AREA * 6.0,
        );
        // 6 box faces, 3 walls and the floor.
        assert_eq!(face_count(&mut worker, &program, "block"), 10, "{face}");
        assert!(
            has_corners(&mut worker, &program, "block", &triangle_at(face, 6.0)),
            "{face}: no triangular floor 6 mm inside"
        );
    }
}

#[test]
fn a_boss_stands_out_of_each_of_the_six_faces() {
    let mut worker = worker();
    for face in ["x-", "x+", "y-", "y+", "z-", "z+"] {
        let program = format!("{BLOCK}boss(block, \"{face}\", {TRIANGLE}, 8)");
        let package = solid(&mut worker, &program, "block").unwrap();
        assert_volume(package.volume_mm3, A * B * C + AREA * 8.0);
        assert_eq!(package.topology_counts[2], 10, "{face}");
        let (n, _, _) = axes(face);
        let [min, max] = package.bounds_mm;
        let (expected_min, expected_max) = if face.ends_with('+') {
            (0.0, SIZE[n] + 8.0)
        } else {
            (-8.0, SIZE[n])
        };
        assert!(
            (min[n] - expected_min).abs() < 1.0e-3 && (max[n] - expected_max).abs() < 1.0e-3,
            "{face}: bounds {:?}",
            package.bounds_mm
        );
        assert!(
            has_corners(&mut worker, &program, "block", &triangle_at(face, -8.0)),
            "{face}"
        );
    }
}

#[test]
fn a_pocket_runs_off_an_edge_and_follows_a_rotated_part() {
    let mut worker = worker();
    // A 25 x 20 notch through the x- edge of the top: only 20 x 20 lies in the block.
    let notch =
        format!("{BLOCK}pocket_shape(block, \"z+\", [(-5, 10), (20, 10), (20, 30), (-5, 30)], 7)");
    assert_volume(
        volume(&mut worker, &notch, "block"),
        A * B * C - 20.0 * 20.0 * 7.0,
    );
    assert_eq!(face_count(&mut worker, &notch, "block"), 10);

    // The same pocket on a turned and moved block sits in the same place of
    // the block's own frame.
    let turned = format!(
        "{BLOCK}rotate(block, axis=[1, 1, 0], angle=35)\nmove(block, by=[40, -20, 15])\npocket_shape(block, \"y+\", {TRIANGLE}, 6)"
    );
    assert_volume(
        volume(&mut worker, &turned, "block"),
        A * B * C - AREA * 6.0,
    );
    assert!(has_corners(
        &mut worker,
        &turned,
        "block",
        &triangle_at("y+", 6.0)
    ));
}

#[test]
fn shapes_apply_in_order_on_a_profile_part_and_after_other_operations() {
    let mut worker = worker();
    // A rounded edge first, then a pocket through it: the pocket takes the
    // round with it, so the rounded material is not counted twice.
    let rounded_then_pocket = format!(
        "{BLOCK}fillet(block, edges=[[\"x-\", \"z+\"]], radius=5)\npocket_shape(block, \"y-\", [(-1, 30), (101, 30), (101, 41), (-1, 41)], 10)"
    );
    // The pocket takes the top 10 mm of the front 10 mm, round included, so
    // the round is only missing along the other 50 mm.
    assert_volume(
        volume(&mut worker, &rounded_then_pocket, "block"),
        A * B * C - fillet_loss(5.0, 50.0) - A * 10.0 * 10.0,
    );

    // A program part: a 60 x 60 plate extruded 20 high, a boss on its side.
    let plate =
        "plate = extrude(\"plate\", profile=[(0, 0), (60, 0), (60, 60), (0, 60)], distance=20)\n";
    let program = format!("{plate}boss(plate, \"x+\", [(10, 5), (50, 5), (30, 15)], 4)");
    assert_volume(
        volume(&mut worker, &program, "plate"),
        60.0 * 60.0 * 20.0 + 200.0 * 4.0,
    );
}

#[test]
fn bad_shape_requests_are_refused_with_the_reason() {
    let face = apply_error(&format!(
        "{BLOCK}pocket_shape(block, \"top\", {TRIANGLE}, 6)"
    ));
    assert!(
        face.contains("face must be one of x-, x+, y-, y+, z-, z+"),
        "{face}"
    );
    let depth = apply_error(&format!(
        "{BLOCK}pocket_shape(block, \"z+\", {TRIANGLE}, 0)"
    ));
    assert!(depth.contains("depth must be positive"), "{depth}");
    let height = apply_error(&format!("{BLOCK}boss(block, \"z+\", {TRIANGLE}, -2)"));
    assert!(height.contains("height must be positive"), "{height}");
}
