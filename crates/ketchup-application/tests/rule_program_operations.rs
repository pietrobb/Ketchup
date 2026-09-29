//! Rule-program operations on any part, checked on the exact solid the
//! window shows: volumes against closed-form values, faces named after cuts
//! and booleans, and operations applied in the order they are written.

mod operations_support;

use std::f64::consts::PI;

use operations_support::*;

#[test]
fn box_edges_round_and_bevel_by_named_faces() {
    let mut worker = worker();
    let one = format!("{BLOCK}fillet(block, edges=[[\"x+\", \"y+\"]], radius=5)");
    assert_volume(
        volume(&mut worker, &one, "block"),
        A * B * C - fillet_loss(5.0, C),
    );

    let vertical = format!(
        "{BLOCK}fillet(block, edges=[[\"x-\", \"y-\"], [\"x+\", \"y-\"], [\"x+\", \"y+\"], [\"x-\", \"y+\"]], radius=8)"
    );
    assert_volume(
        volume(&mut worker, &vertical, "block"),
        A * B * C - 4.0 * fillet_loss(8.0, C),
    );

    // Every edge rounded: the box shrunk by r, grown back by a ball of radius r.
    let r = 6.0_f64;
    let all = format!(
        "{BLOCK}fillet(block, edges=[{}], radius={r})",
        [
            ("x-", "y-"),
            ("x+", "y-"),
            ("x+", "y+"),
            ("x-", "y+"),
            ("z+", "x-"),
            ("z+", "x+"),
            ("z+", "y-"),
            ("z+", "y+"),
            ("z-", "x-"),
            ("z-", "x+"),
            ("z-", "y-"),
            ("z-", "y+"),
        ]
        .iter()
        .map(|(a, b)| format!("[\"{a}\", \"{b}\"]"))
        .collect::<Vec<_>>()
        .join(", ")
    );
    let (a, b, c) = (A - 2.0 * r, B - 2.0 * r, C - 2.0 * r);
    assert_volume(
        volume(&mut worker, &all, "block"),
        a * b * c
            + 2.0 * r * (a * b + a * c + b * c)
            + PI * r * r * (a + b + c)
            + 4.0 / 3.0 * PI * r.powi(3),
    );
    // 6 shrunk walls, 12 edge rounds and 8 corner balls.
    assert_eq!(face_count(&mut worker, &all, "block"), 26);
    assert_eq!(face_count(&mut worker, &one, "block"), 7);
    assert_eq!(face_count(&mut worker, &vertical, "block"), 10);

    let bevel = format!("{BLOCK}chamfer(block, edges=[[\"z+\", \"x+\"]], distance=4)");
    assert_volume(
        volume(&mut worker, &bevel, "block"),
        A * B * C - 0.5 * 4.0 * 4.0 * B,
    );
}

#[test]
fn a_rotated_and_moved_box_rounds_the_same_edge() {
    let mut worker = worker();
    let program = format!(
        "{BLOCK}rotate(block, axis=[1, 1, 0], angle=37)\nmove(block, by=[250, -40, 90])\nfillet(block, edges=[[\"x+\", \"y+\"]], radius=5)"
    );
    assert_volume(
        volume(&mut worker, &program, "block"),
        A * B * C - fillet_loss(5.0, C),
    );
}

#[test]
fn fillet_after_a_trim_rounds_the_new_wall() {
    let mut worker = worker();
    // Trim the block at x = 80, then round the corner between the front and
    // the wall the trim left (the trim tool's z- face lies on the plane).
    let program = format!(
        "{BLOCK}trim(block, point=[80, 0, 0], normal=[1, 0, 0], name=\"shorten\")\nfillet(block, edges=[[\"y-\", \"shorten.z-\"]], radius=5)"
    );
    assert_volume(
        volume(&mut worker, &program, "block"),
        80.0 * B * C - fillet_loss(5.0, C),
    );
}

#[test]
fn fillet_after_subtracting_a_hole_rounds_its_rim() {
    let mut worker = worker();
    let (hole, r) = (10.0_f64, 3.0_f64);
    // A through hole of radius 10 at (50, 30) made of two half circles.
    let program = format!(
        "{BLOCK}pin = extrude(\"pin\", profile=[[\"left\", [50, 20], [50, 40], {{\"center\": [50, 30], \"clockwise\": True}}], [\"right\", [50, 40], [50, 20], {{\"center\": [50, 30], \"clockwise\": True}}]], distance=60, at=[0, 0, -10], tool=True)\nsubtract(block, pin, name=\"drill\")\nfillet(block, edges=[[\"z+\", \"drill.{{half}}\"]], radius={r})"
    );
    // Pappus: the rounded-off corner region times the path of its centroid.
    let centroid = r * (5.0 / 6.0 - PI / 4.0) / (1.0 - PI / 4.0);
    let rim = 2.0 * PI * (hole + centroid) * (1.0 - PI / 4.0) * r * r;
    // The two halves are one round wall, so either name rounds the whole rim.
    for half in ["left", "right"] {
        let program = program.replace("{half}", half);
        assert_volume(
            volume(&mut worker, &program, "block"),
            A * B * C - PI * hole * hole * C - rim,
        );
        assert_eq!(face_count(&mut worker, &program, "block"), 8, "{program}");
    }
}

#[test]
fn operations_apply_in_the_order_they_are_written() {
    let mut worker = worker();
    // Round first, then trim through the rounded edge: the trim removes part
    // of the fillet, so less material is lost to it.
    let round_then_trim = format!(
        "{BLOCK}fillet(block, edges=[[\"x+\", \"y+\"]], radius=5)\ntrim(block, point=[0, 0, 30], normal=[0, 0, 1], name=\"lower\")"
    );
    assert_volume(
        volume(&mut worker, &round_then_trim, "block"),
        A * B * 30.0 - fillet_loss(5.0, 30.0),
    );
    // Trim first, then round an edge of the trimmed top.
    let trim_then_round = format!(
        "{BLOCK}trim(block, point=[0, 0, 30], normal=[0, 0, 1], name=\"lower\")\nfillet(block, edges=[[\"lower.z-\", \"x+\"]], radius=5)"
    );
    assert_volume(
        volume(&mut worker, &trim_then_round, "block"),
        A * B * 30.0 - fillet_loss(5.0, B),
    );
    // Both are six walls and one round, only placed differently.
    assert_eq!(face_count(&mut worker, &round_then_trim, "block"), 7);
    assert_eq!(face_count(&mut worker, &trim_then_round, "block"), 7);
}

#[test]
fn commuting_operations_agree_and_later_ones_see_earlier_results() {
    let mut worker = worker();
    let trim = "trim(block, point=[0, 0, 30], normal=[0, 0, 1], name=\"lower\")\n";
    let pull = "push_pull(block, face=\"x+\", distance=15, name=\"longer\")\n";
    // Trimming the height and pulling the end commute.
    let trim_pull = volume(&mut worker, &format!("{BLOCK}{trim}{pull}"), "block");
    let pull_trim = volume(&mut worker, &format!("{BLOCK}{pull}{trim}"), "block");
    assert_volume(trim_pull, (A + 15.0) * B * 30.0);
    assert_volume(pull_trim, trim_pull);
    assert_eq!(
        face_count(&mut worker, &format!("{BLOCK}{trim}{pull}"), "block"),
        6
    );
    assert_eq!(
        face_count(&mut worker, &format!("{BLOCK}{pull}{trim}"), "block"),
        6
    );

    // Rounding a vertical edge and pulling the top commute too.
    let round = "fillet(block, edges=[[\"x+\", \"y+\"]], radius=5)\n";
    let taller = "push_pull(block, face=\"z+\", distance=10, name=\"taller\")\n";
    let pull_round = format!("{BLOCK}{taller}{round}");
    assert_volume(
        volume(&mut worker, &pull_round, "block"),
        A * B * (C + 10.0) - fillet_loss(5.0, C + 10.0),
    );
    assert_eq!(face_count(&mut worker, &pull_round, "block"), 7);

    // A push_pull after a fillet carries the round along the longer wall.
    let round_pull = format!(
        "{BLOCK}fillet(block, edges=[[\"x+\", \"y+\"]], radius=5)\npush_pull(block, face=\"z+\", distance=10, name=\"taller\")"
    );
    assert_volume(
        volume(&mut worker, &round_pull, "block"),
        A * B * (C + 10.0) - fillet_loss(5.0, C + 10.0),
    );
    // Walls and rounds are exact planes and cylinders, so the pulled round is
    // still one face.
    let pulled = solid(&mut worker, &round_pull, "block").unwrap();
    assert!(
        pulled
            .face_evidence
            .iter()
            .any(|face| face.surface_kind == "cylinder"),
        "{:?}",
        pulled.face_evidence
    );
    assert_eq!(face_count(&mut worker, &round_pull, "block"), 7);

    // A hole drilled after rounding the top edge does not touch the round;
    // rounding the hole's rim needs the hole first, so the reverse is refused.
    let pin = "pin = box(\"pin\", [10, 10, 60], at=[45, 25, -10], tool=True)\n";
    let round_drill = format!(
        "{BLOCK}{pin}fillet(block, edges=[[\"z+\", \"y-\"]], radius=4)\nsubtract(block, pin, name=\"drill\")"
    );
    assert_volume(
        volume(&mut worker, &round_drill, "block"),
        A * B * C - fillet_loss(4.0, A) - 10.0 * 10.0 * C,
    );
    let rim_before_hole = format!(
        "{BLOCK}{pin}fillet(block, edges=[[\"z+\", \"drill.x-\"]], radius=2)\nsubtract(block, pin, name=\"drill\")"
    );
    let error = match session(&rim_before_hole) {
        Err(error) => error,
        Ok(_) => solid(&mut worker, &rim_before_hole, "block").unwrap_err(),
    };
    assert!(error.contains("drill"), "{error}");
}

#[test]
fn box_push_pull_and_cut_now_shape_the_part() {
    let mut worker = worker();
    let pulled = format!("{BLOCK}push_pull(block, face=\"x+\", distance=15, name=\"longer\")");
    assert_volume(volume(&mut worker, &pulled, "block"), (A + 15.0) * B * C);
    let slot = format!(
        "{BLOCK}cut(block, profile=[[\"a\", [40, -1], [60, -1]], [\"b\", [60, -1], [60, 61]], [\"c\", [60, 61], [40, 61]], [\"d\", [40, 61], [40, -1]]], depth=10, name=\"slot\")\nfillet(block, edges=[[\"slot.b\", \"z+#2\"]], radius=2)"
    );
    assert_volume(
        volume(&mut worker, &slot, "block"),
        A * B * C - 20.0 * B * 10.0 - fillet_loss(2.0, B),
    );
}

#[test]
fn a_wrong_face_or_too_large_radius_is_explained() {
    let mut worker = worker();
    let error = apply_error(&format!(
        "{BLOCK}fillet(block, edges=[[\"x+\", \"top\"]], radius=5)"
    ));
    assert!(error.contains("face \"top\" does not exist"), "{error}");
    assert!(error.contains("x-, x+, y-, y+, z-, z+"), "{error}");

    let error = solid(
        &mut worker,
        &format!("{BLOCK}fillet(block, edges=[[\"x+\", \"x-\"]], radius=5)"),
        "block",
    )
    .unwrap_err();
    assert!(error.contains("do not share an edge"), "{error}");

    // The x+ face is only 40 mm tall, so a 45 mm round cannot fit on its top edge.
    let error = solid(
        &mut worker,
        &format!("{BLOCK}fillet(block, edges=[[\"z+\", \"x+\"]], radius=45)"),
        "block",
    )
    .unwrap_err();
    assert!(error.contains("finish did not complete"), "{error}");
}
