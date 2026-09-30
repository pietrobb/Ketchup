//! Hollow parts, countersunk holes, mirrored parts, circular patterns and
//! ellipse / regular polygon profiles, checked on the exact solids against
//! the analytic volumes, with the requests that must be refused.

use crate::operations_support;

use ketchup_model::persistence;
use ketchup_program::run;
use operations_support::*;
use std::collections::BTreeMap;
use std::f64::consts::PI;

fn assert_close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

fn assert_point(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() <= 1.0e-6,
            "{actual:?} != {expected:?}"
        );
    }
}

fn program_model(program: &str) -> ketchup_program::model::ProgramModel {
    run("extras.star", program, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .0
        .model
}

/// Why the program itself refuses to evaluate.
fn eval_error(program: &str) -> String {
    match run("extras.star", program, &BTreeMap::new()) {
        Ok(_) => panic!("the program must be refused:\n{program}"),
        Err(error) => format!("{error:#}"),
    }
}

// --- shell -------------------------------------------------------------

#[test]
fn a_shell_open_at_the_top_leaves_walls_of_the_given_thickness() {
    let mut worker = worker();
    let program = format!("{BLOCK}shell(block, thickness=5, open=[\"z+\"])");
    let hollow = A * B * C - (A - 10.0) * (B - 10.0) * (C - 5.0);
    assert_volume(volume(&mut worker, &program, "block"), hollow);
    // Six outer walls (the top now a rim) and five inner walls.
    assert_eq!(face_count(&mut worker, &program, "block"), 11);

    // The opened face survives saving and reopening the document.
    let snapshot = session(&program).unwrap().snapshot();
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert!(reopened.is_editable());
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );

    // The inner walls are named: round the inside of the floor along x-.
    let rounded = format!(
        "{program}\nfillet(block, edges=[[\"block shell.x-\", \"block shell.z-\"]], radius=2)"
    );
    assert_volume(
        volume(&mut worker, &rounded, "block"),
        hollow + fillet_loss(2.0, B - 10.0),
    );
}

#[test]
fn a_shell_open_at_two_faces_is_a_tube() {
    let mut worker = worker();
    let program = format!("{BLOCK}shell(block, thickness=4, open=[\"x-\", \"x+\"], name=\"tube\")");
    assert_volume(
        volume(&mut worker, &program, "block"),
        A * (B * C - (B - 8.0) * (C - 8.0)),
    );
}

#[test]
fn impossible_shells_are_refused() {
    let closed = eval_error(&format!("{BLOCK}shell(block, thickness=5, open=[])"));
    assert!(closed.contains("open at least one face"), "{closed}");
    let thick = eval_error(&format!("{BLOCK}shell(block, thickness=20, open=[\"z+\"])"));
    assert!(
        thick.contains("under half the part's thinnest size 40"),
        "{thick}"
    );
    let unknown = eval_error(&format!("{BLOCK}shell(block, thickness=5, open=[\"lid\"])"));
    assert!(unknown.contains("lid"), "{unknown}");
    let twice = eval_error(&format!(
        "{BLOCK}shell(block, thickness=5, open=[\"z+\", \"z+\"])"
    ));
    assert!(twice.contains("listed twice"), "{twice}");
    let named = eval_error(&format!(
        "{BLOCK}shell(block, thickness=5, open=[\"z+\"], name=\"a.b\")"
    ));
    assert!(named.contains("prefixes the inner walls"), "{named}");
}

// --- countersunk hole --------------------------------------------------

#[test]
fn a_countersunk_hole_removes_a_cylinder_and_a_cone() {
    let mut worker = worker();
    // 8 mm hole 20 deep, 16 mm head at 90 degrees: the cone is 4 mm deep.
    let program = format!(
        "{BLOCK}countersunk_hole(block, \"z+\", at=(50, 30), diameter=8, depth=20, head=16)"
    );
    let removed = PI * 16.0 * 16.0 + PI * 4.0 / 3.0 * (16.0 + 32.0 + 64.0);
    assert_close(
        volume(&mut worker, &program, "block"),
        A * B * C - removed,
        1.0e-6,
    );

    // Into a side face, at a steeper 60 degree head.
    let side = format!(
        "{BLOCK}countersunk_hole(block, \"x-\", at=(30, 20), diameter=6, depth=30, head=12, angle=60)"
    );
    let sink = 3.0 / (30.0_f64).to_radians().tan();
    let removed = PI * 9.0 * (30.0 - sink) + PI * sink / 3.0 * (9.0 + 18.0 + 36.0);
    assert_close(
        volume(&mut worker, &side, "block"),
        A * B * C - removed,
        1.0e-6,
    );
}

#[test]
fn impossible_countersinks_are_refused() {
    let narrow = eval_error(&format!(
        "{BLOCK}countersunk_hole(block, \"z+\", at=(50, 30), diameter=8, depth=20, head=8)"
    ));
    assert!(narrow.contains("must be wider than the hole"), "{narrow}");
    let deep = eval_error(&format!(
        "{BLOCK}countersunk_hole(block, \"z+\", at=(50, 30), diameter=8, depth=3, head=16)"
    ));
    assert!(deep.contains("cone is deeper than"), "{deep}");
    let flat = eval_error(&format!(
        "{BLOCK}countersunk_hole(block, \"z+\", at=(50, 30), diameter=8, depth=20, head=16, angle=180)"
    ));
    assert!(flat.contains("between 0 and 180"), "{flat}");
}

// --- mirror ------------------------------------------------------------

const NOTCHED: &str = "block = box(\"block\", [100, 60, 40])\nnotch = box(\"notch\", [20, 60, 10], at=[0, 0, 30], tool=True)\nsubtract(block, notch)\n";

/// Local x of the solid's centre of mass, from its exact tessellation (exact
/// for planar faces).
fn centroid_x(
    worker: &mut ketchup_scheduler::ExactWorkerSupervisor,
    program: &str,
    part: &str,
) -> f64 {
    let package = solid(worker, program, part).unwrap();
    let (mut volume, mut moment) = (0.0, 0.0);
    for triangle in &package.triangles {
        let [a, b, c] = triangle
            .vertex_indices
            .map(|index| package.vertices[index as usize].position_mm);
        let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
        volume += det / 6.0;
        moment += det / 6.0 * (a[0] + b[0] + c[0]) / 4.0;
    }
    moment / volume
}

/// The notch at local x 0..20 shifts the centre of mass towards +x.
const NOTCHED_CENTROID_X: f64 = (240_000.0 * 50.0 - 12_000.0 * 10.0) / 228_000.0;

#[test]
fn a_mirror_moves_the_shaping_to_the_other_side() {
    let mut worker = worker();
    let mirrored = format!("{NOTCHED}mirror(block, axis=\"x\")");
    let whole = A * B * C - 20.0 * 60.0 * 10.0;
    assert_volume(volume(&mut worker, NOTCHED, "block"), whole);
    assert_volume(volume(&mut worker, &mirrored, "block"), whole);
    assert_close(
        centroid_x(&mut worker, NOTCHED, "block"),
        NOTCHED_CENTROID_X,
        1.0e-9,
    );
    assert_close(
        centroid_x(&mut worker, &mirrored, "block"),
        A - NOTCHED_CENTROID_X,
        1.0e-9,
    );
    let package = solid(&mut worker, &mirrored, "block").unwrap();
    assert_point(package.bounds_mm[0], [0.0, 0.0, 0.0]);
    assert_point(package.bounds_mm[1], [A, B, C]);
}

#[test]
fn a_mirrored_copy_is_the_reflection_across_a_world_plane() {
    let mut worker = worker();
    let program = format!(
        "{NOTCHED}rotate(block, axis=[0, 0, 1], angle=30)\nmove(block, by=[50, 20, 0])\nmirrored(block, \"right\", point=[300, 0, 0], normal=[1, 0, 0])"
    );
    let model = program_model(&program);
    let (original, copy) = (model.part("block").unwrap(), model.part("right").unwrap());
    // Any point of the part and its mirror image in the copy.
    for local in [[10.0, 30.0, 35.0], [0.0, 0.0, 0.0], [100.0, 60.0, 40.0]] {
        let world = original.to_world(local);
        let image = copy.to_world([A - local[0], local[1], local[2]]);
        assert_point(image, [600.0 - world[0], world[1], world[2]]);
    }
    assert_volume(
        volume(&mut worker, &program, "right"),
        A * B * C - 20.0 * 60.0 * 10.0,
    );
    assert_close(
        centroid_x(&mut worker, &program, "right"),
        A - NOTCHED_CENTROID_X,
        1.0e-9,
    );
}

#[test]
fn a_mirror_can_come_before_or_after_holes_and_finishes() {
    let mut worker = worker();
    let drilled = A * B * C - PI * 16.0 * 10.0;
    for program in [
        format!("{BLOCK}hole(block, \"z+\", at=(20, 30), diameter=8, depth=10)\nmirror(block)"),
        format!("{BLOCK}mirror(block)\nhole(block, \"z+\", at=(20, 30), diameter=8, depth=10)"),
    ] {
        assert_volume(volume(&mut worker, &program, "block"), drilled);
        // The mirror carries the hole from x = 20 to x = 80 either way.
        let model = program_model(&program);
        let block = model.part("block").unwrap();
        let hole = &block.holes[0];
        assert_point(
            block.after_operations((hole.entry_mm, hole.inward)).0,
            [80.0, 30.0, 40.0],
        );
    }
    let rounded =
        format!("{BLOCK}mirror(block)\nfillet(block, edges=[[\"x+\", \"y+\"]], radius=3)");
    assert_volume(
        volume(&mut worker, &rounded, "block"),
        A * B * C - fillet_loss(3.0, C),
    );
    let axis = eval_error(&format!("{BLOCK}mirror(block, axis=\"w\")"));
    assert!(axis.contains("axis must be"), "{axis}");
}

// --- circular pattern --------------------------------------------------

#[test]
fn a_circular_pattern_turns_copies_evenly_about_the_axis() {
    let mut worker = worker();
    let program =
        "spoke = box(\"spoke\", [20, 10, 10], at=[100, -5, 0])\ncircular_pattern(spoke, 4)";
    let model = program_model(program);
    let names: Vec<_> = model.parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names, ["spoke", "spoke 2", "spoke 3", "spoke 4"]);
    // The spoke's far end centre (120, 0, 5) turns by 90 degrees per copy.
    for (name, expected) in [
        ("spoke 2", [0.0, 120.0, 5.0]),
        ("spoke 3", [-120.0, 0.0, 5.0]),
        ("spoke 4", [0.0, -120.0, 5.0]),
    ] {
        assert_point(
            model.part(name).unwrap().to_world([20.0, 5.0, 5.0]),
            expected,
        );
        assert_volume(volume(&mut worker, program, name), 20.0 * 10.0 * 10.0);
    }

    // Over 90 degrees three copies spread 0, 45 and 90 degrees.
    let spread = program_model(
        "spoke = box(\"spoke\", [20, 10, 10], at=[100, -5, 0])\ncircular_pattern(spoke, 3, angle=90, center=(0, 0, 0))",
    );
    let half = 120.0 * (0.5_f64).sqrt();
    assert_point(
        spread.part("spoke 2").unwrap().to_world([20.0, 5.0, 5.0]),
        [half, half, 5.0],
    );
    assert_point(
        spread.part("spoke 3").unwrap().to_world([20.0, 5.0, 5.0]),
        [0.0, 120.0, 5.0],
    );
}

#[test]
fn impossible_patterns_are_refused() {
    let one = eval_error("spoke = box(\"spoke\", [20, 10, 10])\ncircular_pattern(spoke, 1)");
    assert!(one.contains("at least 2"), "{one}");
    let names = eval_error(
        "spoke = box(\"spoke\", [20, 10, 10])\ncircular_pattern(spoke, 3, names=[\"a\"])",
    );
    assert!(names.contains("give 2 names"), "{names}");
    let angle =
        eval_error("spoke = box(\"spoke\", [20, 10, 10])\ncircular_pattern(spoke, 3, angle=400)");
    assert!(angle.contains("within (0, 360]"), "{angle}");
}

// --- ellipse and regular polygon profiles ------------------------------

#[test]
fn an_ellipse_profile_extrudes_to_the_ellipse_volume() {
    let mut worker = worker();
    let program = "plate = extrude(\"plate\", profile=ellipse(50, 30), distance=10)";
    // Four cubic quarters stay within 0.03 % of the true ellipse.
    assert_close(
        volume(&mut worker, program, "plate"),
        PI * 50.0 * 30.0 * 10.0,
        1.0e-3,
    );
    assert_eq!(face_count(&mut worker, program, "plate"), 6);
    let package = solid(&mut worker, program, "plate").unwrap();
    assert_point(
        package.bounds_mm[0].map(|v| (v * 1.0e3).round() / 1.0e3),
        [-50.0, -30.0, 0.0],
    );
    assert_point(
        package.bounds_mm[1].map(|v| (v * 1.0e3).round() / 1.0e3),
        [50.0, 30.0, 10.0],
    );

    // Its quarter curves are named faces: round the top along one of them.
    let named = format!("{program}\nchamfer(plate, edges=[[\"side1\", \"end\"]], distance=1)");
    assert!(volume(&mut worker, &named, "plate") < PI * 50.0 * 30.0 * 10.0);
}

#[test]
fn a_circle_drawn_as_two_half_arcs_extrudes_to_a_disc() {
    let mut worker = worker();
    // Both halves share both endpoints; the exact kernel must still tell
    // the named first arc from the second one.
    let program = "disc = extrude(\"disc\", profile=[[\"a\", (40, 0), (-40, 0), {\"through\": (0, 40)}], [\"b\", (-40, 0), (40, 0), {\"through\": (0, -40)}]], distance=10)";
    assert_close(
        volume(&mut worker, program, "disc"),
        PI * 40.0 * 40.0 * 10.0,
        1.0e-9,
    );
    assert_eq!(face_count(&mut worker, program, "disc"), 4);
}

#[test]
fn a_regular_polygon_profile_extrudes_to_its_area() {
    let mut worker = worker();
    let program = "nut = extrude(\"nut\", profile=polygon(6, 50), distance=10)";
    let area = 1.5 * 3.0_f64.sqrt() * 50.0 * 50.0;
    assert_close(volume(&mut worker, program, "nut"), area * 10.0, 1.0e-9);
    assert_eq!(face_count(&mut worker, program, "nut"), 8);
    let turned =
        "tri = extrude(\"tri\", profile=polygon(3, 20, center=(100, 0), angle=90), distance=5)";
    let package = solid(&mut worker, turned, "tri").unwrap();
    // The first corner points along +y: (100, 20).
    assert_close(package.bounds_mm[1][1], 20.0, 1.0e-6);
    assert_close(
        package.volume_mm3,
        0.75 * 3.0_f64.sqrt() * 400.0 * 5.0,
        1.0e-9,
    );
}

#[test]
fn degenerate_profiles_are_refused() {
    let sides = eval_error("p = extrude(\"p\", profile=polygon(2, 50), distance=10)");
    assert!(sides.contains("at least 3"), "{sides}");
    let radius = eval_error("p = extrude(\"p\", profile=polygon(5, 0), distance=10)");
    assert!(radius.contains("radius must be positive"), "{radius}");
    let flat = eval_error("p = extrude(\"p\", profile=ellipse(50, 0), distance=10)");
    assert!(flat.contains("must be positive"), "{flat}");
    let controls = eval_error(
        "p = extrude(\"p\", profile=[[\"a\", (0, 0), (10, 0), {\"controls\": [(3, 3)]}], [\"b\", (10, 0), (0, 10)], [\"c\", (0, 10), (0, 0)]], distance=5)",
    );
    assert!(controls.contains("two points"), "{controls}");
}
