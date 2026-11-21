//! Edge finishes OCCT would refuse are refused by the program already, so
//! `ketchup-program check` does not pass them (review 2026-11-20 ac P3).

use ketchup_program::run;
use std::collections::BTreeMap;

const PLATE: &str = "p = box(\"p\", (40, 40, 10))\n";

fn refusal(finishes: &str) -> String {
    run(
        "finish.star",
        &format!("{PLATE}{finishes}"),
        &BTreeMap::new(),
    )
    .map(|_| String::new())
    .unwrap_or_else(|error| error.message)
}

#[test]
fn a_finish_wider_than_the_part_across_its_edge_is_refused_with_what_fits() {
    let message = refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 500)\n");
    assert!(
        message.contains("10 mm across") && message.contains("less than 9.99 mm"),
        "{message}"
    );
    let message = refusal("chamfer(p, edges = [[\"x+\", \"y+\"]], distance = 40)\n");
    assert!(message.contains("less than 39.99 mm"), "{message}");
    // The advice is the real limit: 9.995 mm is refused, so it may not say 10.
    let message = refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 9.995)\n");
    assert!(message.contains("less than 9.99 mm"), "{message}");
    assert_eq!(
        refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 9.98)\n"),
        ""
    );
    assert_eq!(
        refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 4)\n"),
        ""
    );
}

/// A finish takes its amount off both faces, so finishes on the two edges of a
/// face across its width must add up to less than the width; OCCT refuses
/// them otherwise (R1121ac P3).
#[test]
fn finishes_on_both_edges_of_a_face_must_fit_it_together() {
    // r = 6 on both 10 mm edges of the side x+.
    let message = refusal("fillet(p, edges = [[\"x+\", \"z+\"], [\"x+\", \"z-\"]], radius = 6)\n");
    assert!(
        message.contains("face \"x+\" is 10 mm across")
            && message.contains("take 6 mm")
            && message.contains("less than 3.99 mm"),
        "{message}"
    );
    // Chamfer 9 on both, given one finish at a time.
    let message = refusal(
        "chamfer(p, edges = [[\"z+\", \"y+\"]], distance = 9)\n\
         chamfer(p, edges = [[\"y+\", \"z-\"]], distance = 9)\n",
    );
    assert!(
        message.contains("face \"y+\" is 10 mm across") && message.contains("less than 0.99 mm"),
        "{message}"
    );
    // r = 30 on both 40 mm edges of the top of a cube.
    let plate = |finishes: &str| {
        run(
            "finish.star",
            &format!("p = box(\"p\", (40, 40, 40))\n{finishes}"),
            &BTreeMap::new(),
        )
        .map(|_| String::new())
        .unwrap_or_else(|error| error.message)
    };
    let message = plate("fillet(p, edges = [[\"z+\", \"x+\"], [\"z+\", \"x-\"]], radius = 30)\n");
    assert!(
        message.contains("face \"z+\" is 40 mm across") && message.contains("less than 9.99 mm"),
        "{message}"
    );
    // Together less than the width, and edges across another width, still build.
    assert_eq!(
        plate("fillet(p, edges = [[\"z+\", \"x+\"], [\"z+\", \"x-\"]], radius = 19)\n"),
        ""
    );
    assert_eq!(
        refusal(
            "fillet(p, edges = [[\"x+\", \"z+\"], [\"x+\", \"y+\"], [\"x+\", \"y-\"]], radius = 4)\n\
             chamfer(p, edges = [[\"x+\", \"z-\"]], distance = 5)\n"
        ),
        ""
    );
}

#[test]
fn a_finish_too_small_to_build_names_its_parameter() {
    let message = refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 0.005)\n");
    assert!(
        message.contains("radius must be more than 0.01 mm, got 0.005"),
        "{message}"
    );
    let message = refusal("chamfer(p, edges = [[\"z+\", \"x+\"]], distance = 0)\n");
    assert!(
        message.contains("distance must be more than 0.01 mm"),
        "{message}"
    );
}

#[test]
fn faces_that_do_not_meet_have_no_edge_to_finish() {
    let message = refusal("fillet(p, edges = [[\"x+\", \"x-\"]], radius = 2)\n");
    assert!(message.contains("share no edge"), "{message}");
}

#[test]
fn an_edge_takes_one_finish() {
    let message = refusal(
        "fillet(p, edges = [[\"z+\", \"x+\"]], radius = 2, name = \"round\")\n\
         chamfer(p, edges = [[\"x+\", \"z+\"]], distance = 1)\n",
    );
    assert!(
        message.contains("already finished by \"round\""),
        "{message}"
    );
    // Another edge of the same part still takes its own.
    assert_eq!(
        refusal(
            "fillet(p, edges = [[\"z+\", \"x+\"]], radius = 2)\n\
             chamfer(p, edges = [[\"z+\", \"y+\"]], distance = 1)\n"
        ),
        ""
    );
}
