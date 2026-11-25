//! Review 2026-11-25 R1125og-PG-1006 and PG-1007: what boxes alone settle
//! about two parts that overlap.

use ketchup_program::{COLLISION_UNVERIFIED, OverlapStatus, run};
use std::collections::BTreeMap;

fn status(report: &ketchup_program::Report, a: &str, b: &str) -> Option<OverlapStatus> {
    report
        .relations
        .iter()
        .find(|relation| relation.parts == [a, b])
        .and_then(|relation| relation.status)
}

/// R1125og-PG-1006: two drilled boards overlapping by 50 x 100 x 18 mm
/// passed with exit 0 and only a `collision_unverified` warning, although
/// their two holes remove at most about 1 000 mm³ of the 90 000 mm³ the
/// boxes share.
#[test]
fn drilled_boards_sharing_more_than_their_holes_remove_collide() {
    let source = "a = board(\"a\", (100, 100, 18))\n\
                  hole(a, \"z+\", world = (50, 50, 18), diameter = 8, depth = 10, id = \"h\")\n\
                  b = board(\"b\", (100, 100, 18), at = (50, 0, 0))\n\
                  hole(b, \"z+\", world = (100, 50, 18), diameter = 8, depth = 10, id = \"h\")\n";
    let (_, report) =
        run("drilled.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let collision = report
        .issues
        .iter()
        .find(|issue| issue.kind == "collision")
        .unwrap_or_else(|| panic!("{:#?}", report.issues));
    assert_eq!(collision.parts, ["a", "b"]);
    assert!(
        collision.message.contains("at least"),
        "{}",
        collision.message
    );
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED),
        "{:#?}",
        report.issues
    );
    assert_eq!(status(&report, "a", "b"), Some(OverlapStatus::Collision));
    assert_eq!(report.exit_code(), 1);
}

/// Holes that could remove all the volume the boxes share leave the
/// question to the exact solids.
#[test]
fn boards_whose_holes_could_remove_the_shared_volume_stay_unverified() {
    let source = "a = board(\"a\", (100, 100, 18))\n\
                  hole(a, \"z+\", world = (75, 50, 18), diameter = 90, depth = 18, id = \"h\")\n\
                  b = board(\"b\", (100, 100, 18), at = (50, 0, 0))\n";
    let (_, report) =
        run("drilled.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.kind == COLLISION_UNVERIFIED),
        "{:#?}",
        report.issues
    );
    assert!(
        !report.issues.iter().any(|issue| issue.kind == "collision"),
        "{:#?}",
        report.issues
    );
}

/// R1125og-PG-1007: the two copies of one beam in `alternatives()` are left
/// out of the collision check on purpose, yet their relation said
/// `boxes_only`, "the exact solids do not overlap".
#[test]
fn alternatives_overlap_as_alternatives_not_as_cleared_boxes() {
    let source = "box(\"beam simple\", (2000, 100, 200), material = \"KVH\", tags = [\"simple\"])\n\
                  box(\"beam detailed\", (2000, 100, 200), material = \"KVH\", tags = [\"detailed\"])\n\
                  alternatives([\"simple\", \"detailed\"])\n";
    let (_, report) =
        run("t.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let relation = report
        .relations
        .iter()
        .find(|relation| relation.parts.contains(&"beam simple".to_owned()))
        .unwrap_or_else(|| panic!("{:#?}", report.relations));
    assert_eq!(
        relation.status,
        Some(OverlapStatus::Alternatives),
        "{relation:?}"
    );
}

/// R1125og-PG-1007: the halves of a split share no volume, which the
/// collision check knows from their trims; their relation said `boxes_only`.
#[test]
fn split_halves_overlap_as_removed_not_as_cleared_boxes() {
    let source = "a = box(\"a\", [100, 60, 40])\n\
                  split(a, point=[50, 30, 20], normal=[1, 1, 1])\n";
    let (_, report) =
        run("t.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let statuses: Vec<_> = report
        .relations
        .iter()
        .filter_map(|relation| relation.status)
        .collect();
    assert_eq!(
        statuses,
        [OverlapStatus::Removed],
        "{:#?}",
        report.relations
    );
}
