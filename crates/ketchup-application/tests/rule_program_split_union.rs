//! Cutting a part in two with a plane and joining parts into one, checked
//! on the exact solids: the pieces add up to the whole, a join holds both
//! volumes, and impossible requests are refused with the reason.

mod operations_support;

use ketchup_program::{run, validate};
use operations_support::*;
use std::collections::BTreeMap;

/// Validator findings of `program` (collisions, missing contacts, ...).
fn issues(program: &str) -> Vec<String> {
    let model = run("split.star", program, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model;
    validate(&model)
        .into_iter()
        .map(|issue| format!("{}: {}", issue.kind, issue.message))
        .collect()
}

#[test]
fn a_split_keeps_both_halves_and_they_add_up_to_the_part() {
    let mut worker = worker();
    let program = format!("{BLOCK}split(block, point=[30, 0, 0], normal=[1, 0, 0], name=\"end\")");
    assert_eq!(part_names(&program), ["block", "end"]);
    assert_volume(volume(&mut worker, &program, "block"), 30.0 * B * C);
    assert_volume(volume(&mut worker, &program, "end"), 70.0 * B * C);
    assert_eq!(face_count(&mut worker, &program, "block"), 6);
    assert_eq!(face_count(&mut worker, &program, "end"), 6);
    // The halves touch across the cut; neither collides with the other.
    assert!(issues(&program).is_empty(), "{:?}", issues(&program));

    // The new face has a name, so its edges can be rounded.
    let rounded = format!("{program}\nfillet(block, edges=[[\"split.z-\", \"y-\"]], radius=3)");
    assert_volume(
        volume(&mut worker, &rounded, "block"),
        30.0 * B * C - fillet_loss(3.0, C),
    );
}

#[test]
fn an_oblique_plane_splits_a_rotated_part_without_losing_material() {
    let mut worker = worker();
    // The block's centre after the turn and move is about (228.3, 101.0, 20).
    let program = format!(
        "{BLOCK}rotate(block, axis=[0, 0, 1], angle=30)\nmove(block, by=[200, 50, 0])\nsplit(block, point=[228.3, 101.0, 20], normal=[1, 1, 1])"
    );
    let first = volume(&mut worker, &program, "block");
    let second = volume(&mut worker, &program, "block 2");
    assert_volume(first + second, A * B * C);
    assert!(
        first > 0.2 * A * B * C && second > 0.2 * A * B * C,
        "{first} {second}"
    );
    // A slanted cut through a box leaves seven faces on each side at most.
    for part in ["block", "block 2"] {
        let faces = face_count(&mut worker, &program, part);
        assert!((5..=7).contains(&faces), "{part}: {faces}");
    }
}

#[test]
fn a_plane_that_misses_the_part_is_refused() {
    let error = apply_error(&format!(
        "{BLOCK}split(block, point=[150, 0, 0], normal=[1, 0, 0])"
    ));
    assert!(error.contains("misses the part"), "{error}");
    assert!(error.contains("spans 0.0 to 100.0"), "{error}");
}

#[test]
fn overlapping_parts_join_into_one_solid_with_all_their_shaping() {
    let mut worker = worker();
    // A 20 x 20 post standing 20 mm deep in the block and 60 mm above it,
    // with one rounded vertical edge.
    let program = format!(
        "{BLOCK}post = box(\"post\", [20, 20, 80], at=[10, 10, 20])\nfillet(post, edges=[[\"x+\", \"y+\"]], radius=4)\nunion(block, post, name=\"joined\")"
    );
    assert_eq!(part_names(&program), ["block"]);
    // Inside the block the round is filled anyway; above it the post keeps it.
    assert_volume(
        volume(&mut worker, &program, "block"),
        A * B * C + 20.0 * 20.0 * 60.0 - fillet_loss(4.0, 60.0),
    );
    assert!(issues(&program).is_empty(), "{:?}", issues(&program));
}

#[test]
fn touching_parts_join_and_their_flush_walls_become_one() {
    let mut worker = worker();
    let program =
        format!("{BLOCK}cap = box(\"cap\", [100, 60, 10], at=[0, 0, 40])\nunion(block, cap)");
    assert_eq!(part_names(&program), ["block"]);
    assert_volume(volume(&mut worker, &program, "block"), A * B * (C + 10.0));
    assert_eq!(face_count(&mut worker, &program, "block"), 6);
}

#[test]
fn a_join_of_parts_apart_or_with_holes_is_refused() {
    let apart = apply_error(&format!(
        "{BLOCK}far = box(\"far\", [10, 10, 10], at=[150, 0, 0])\nunion(block, far)"
    ));
    assert!(apart.contains("50.000 mm apart"), "{apart}");

    let drilled = apply_error(&format!(
        "{BLOCK}cap = box(\"cap\", [100, 60, 10], at=[0, 0, 40])\nhole(cap, \"z+\", at=(50, 30), diameter=8, depth=5)\nunion(block, cap)"
    ));
    assert!(drilled.contains("has holes or pockets"), "{drilled}");
}
