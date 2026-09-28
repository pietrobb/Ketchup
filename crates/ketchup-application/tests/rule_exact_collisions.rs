//! Box overlaps a rule program cannot decide are settled by the exact solids.
use ketchup_application::{DocumentSession, SessionSettings, verify_rule_program_collisions};
use ketchup_core::document::RuleProgramSource;
use ketchup_core::persistence::ContainerData;
use ketchup_program::COLLISION_UNVERIFIED;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const ROUND_STOOL: &str = include_str!("../../../examples/programs/round_stool.star");

fn verified(source: String) -> (ketchup_program::Report, serde_json::Value) {
    let mut session = DocumentSession::new(SessionSettings::default());
    let applied = session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "stool.star".to_owned(),
                source,
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    let mut report = applied.report;
    let summary = verify_rule_program_collisions(
        &applied.snapshot,
        &mut report,
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
        Arc::new(AtomicBool::new(false)),
    )
    .expect("the program left overlaps for the exact check");
    (report, summary)
}

#[test]
fn legs_in_a_round_seat_without_holes_collide_in_the_exact_solids() {
    let source = ROUND_STOOL.replace("    subtract(seat, leg)\n", "    pass\n");
    let (report, summary) = verified(source);
    assert_eq!(summary["state"], "verified", "{summary}");
    assert_eq!(summary["collisions"], 3, "{summary}");
    let collisions: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .collect();
    assert_eq!(collisions.len(), 3, "{:#?}", report.issues);
    assert!(!report.ok);
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED)
    );
}

fn exact_kinds(source: &str) -> (Vec<String>, serde_json::Value) {
    let (report, summary) = verified(source.to_owned());
    assert_eq!(summary["state"], "verified", "{summary}");
    let kinds = report
        .issues
        .iter()
        .map(|issue| issue.kind.to_owned())
        .filter(|kind| kind == "collision" || kind == COLLISION_UNVERIFIED)
        .collect();
    (kinds, summary)
}

#[test]
fn rounded_profile_corners_are_exact_arcs_in_the_solid() {
    // Corner arc centre (40, 40), radius 40: a 10 mm cube in the corner stays
    // outside the arc (its far corner is 42.4 mm from the centre), a cube
    // moved to (8, 8) reaches 31 mm from it and cuts into the top.
    let top = "top = extrude(\"top\", distance = 18, \
               profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40))\n";
    let (clear, summary) = exact_kinds(&format!(
        "{top}c = box(\"c\", (10, 10, 10), at = (0, 0, 4))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{top}c = box(\"c\", (10, 10, 10), at = (8, 8, 4))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn an_arched_apron_leaves_its_arch_open() {
    // The arch rises from (0, 0) through (300, 60) to (600, 0); below it the
    // apron is empty.
    let apron = "apron = extrude(\"apron\", distance = 20, profile = [\
                 [\"top\", [600, 120], [0, 120]], [\"left\", [0, 120], [0, 0]], \
                 [\"arch\", [0, 0], [600, 0], {\"through\": (300, 60)}], \
                 [\"right\", [600, 0], [600, 120]]])\n";
    let (clear, summary) = exact_kinds(&format!(
        "{apron}c = box(\"c\", (40, 40, 10), at = (280, 0, 5))\n"
    ));
    assert!(clear.is_empty(), "{clear:?} {summary}");
    let (hit, summary) = exact_kinds(&format!(
        "{apron}c = box(\"c\", (40, 40, 10), at = (280, 30, 5))\n"
    ));
    assert_eq!(hit, ["collision"], "{summary}");
}

#[test]
fn a_part_inside_the_empty_corner_of_a_triangle_is_cleared() {
    // The cube sits in the corner the triangle leaves empty; the boxes overlap.
    let source = "t = extrude(\"t\", profile = [(0, 0), (100, 0), (0, 100)], distance = 20)\n\
                  c = box(\"c\", (20, 20, 20), at = (70, 70, 0))\n"
        .to_owned();
    let (report, summary) = verified(source);
    assert_eq!(summary["state"], "verified", "{summary}");
    assert_eq!(summary["cleared"], 1, "{summary}");
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED || issue.kind == "collision"),
        "{:#?}",
        report.issues
    );
}
