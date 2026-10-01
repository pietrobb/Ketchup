//! Threads as one sweep of a profile along `helix()` with a fixed `up`,
//! checked on the exact solids: the profile keeps its axial section on every
//! turn, so Pappus gives the volume and the end caps give the height.

use crate::operations_support::{solid, worker};
use std::f64::consts::TAU;

const RADIUS: f64 = 4.2;
const PITCH: f64 = 1.5;
const TURNS: f64 = 12.0;
const DEPTH: f64 = 0.8;
const WIDTH: f64 = 1.3;

fn thread(left: bool) -> String {
    let (left, up) = if left { ("True", -1) } else { ("False", 1) };
    format!(
        "sweep(\"thread\", profile = v_thread(depth = {DEPTH}, width = {WIDTH}),\n      \
         path = helix(radius = {RADIUS}, pitch = {PITCH}, turns = {TURNS}, left = {left}),\n      \
         up = (0, 0, {up}))\n"
    )
}

fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} != {expected} (± {tolerance})"
    );
}

#[test]
fn a_v_thread_keeps_its_axial_section_on_every_turn() {
    let mut worker = worker();
    for left in [false, true] {
        let program = thread(left);
        let package = solid(&mut worker, &program, "thread").unwrap_or_else(|error| {
            panic!("{error}\n{program}");
        });
        assert_eq!(package.topology_counts[4], 1, "{program}");
        // The tooth's centroid stays a third of its depth outside the path,
        // so the solid is its area carried round that radius TURNS times; a
        // profile turning about the path, or a tooth pointing at the axis,
        // would move the centroid and the volume by percent.
        let expected = WIDTH * DEPTH / 2.0 * TAU * (RADIUS + DEPTH / 3.0) * TURNS;
        assert_close(
            package.volume_mm3,
            expected,
            2.0e-3 * expected,
            &format!("volume (left = {left})"),
        );
        // Base corners stay half the width above and below the path ends,
        // the tip on the radius `DEPTH` outside it.
        let [min, max] = package.bounds_mm;
        assert_close(min[2], -WIDTH / 2.0, 1.0e-2, "bottom");
        assert_close(max[2], PITCH * TURNS + WIDTH / 2.0, 1.0e-2, "top");
        assert_close(max[0], RADIUS + DEPTH, 1.0e-2, "tip radius");
    }
}

#[test]
fn up_along_the_path_is_refused_where_it_runs_along_it() {
    let error = crate::operations_support::apply_error(
        "sweep(\"rod\", profile = v_thread(depth = 1, width = 2),\n      \
         path = [(0, 0, 0), (0, 0, 50)], up = (0, 0, 1))\n",
    );
    assert!(error.contains("runs along up"), "{error}");
}
