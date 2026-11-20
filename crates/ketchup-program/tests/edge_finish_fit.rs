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
        message.contains("10 mm across") && message.contains("less than 10 mm"),
        "{message}"
    );
    let message = refusal("chamfer(p, edges = [[\"x+\", \"y+\"]], distance = 40)\n");
    assert!(message.contains("less than 40 mm"), "{message}");
    assert_eq!(
        refusal("fillet(p, edges = [[\"z+\", \"x+\"]], radius = 4)\n"),
        ""
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
