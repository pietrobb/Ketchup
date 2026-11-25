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

fn assert_certain_collision(source: &str, a: &str, b: &str) {
    let (_, report) =
        run("t.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.kind == "collision" && issue.parts == [a, b]),
        "{:#?}",
        report.issues
    );
    assert_eq!(status(&report, a, b), Some(OverlapStatus::Collision));
    assert_eq!(report.exit_code(), 1);
}

/// R1126og-PG-1102 `beams.star`: six through holes far from where a second
/// beam runs through the first were counted whole (about 3.7 M mm³) against
/// the 3.456 M mm³ the beams share, so the pair passed as unverified.
#[test]
fn holes_away_from_the_overlap_do_not_excuse_it() {
    let holes: String = [300, 900, 1500, 2500, 3100, 3700]
        .iter()
        .map(|x| {
            format!(
                "hole(a, \"y-\", world = ({x}, 0, 120), diameter = 14, depth = 120, \
                 through = True, id = \"h{x}\")\n"
            )
        })
        .collect();
    let source = format!(
        "a = box(\"a\", (4000, 120, 240))\n{holes}\
         b = box(\"b\", (120, 2000, 240), at = (1940, -940, 0))\n"
    );
    assert_certain_collision(&source, "a", "b");
}

/// R1126og-PG-1102 `throughpocket.star`: a 360 x 200 cutout milled 1 000 mm
/// deep far from the shelf counted its whole depth against the 90 720 mm³
/// the shelf sits in the side.
#[test]
fn a_deep_pocket_away_from_the_overlap_does_not_excuse_it() {
    let source = "side = box(\"side\", (18, 560, 720))\n\
                  pocket(side, \"x+\", rect = (200, 400, 560, 600), depth = 1000, id = \"cutout\")\n\
                  shelf = box(\"shelf\", (400, 560, 18), at = (9, 0, 0))\n";
    assert_certain_collision(source, "side", "shelf");
}

/// R1126og-PG-1102 `trimmed.star`: the tool of a trim is a cube far larger
/// than the leg, so its whole box excused a second leg lying entirely in
/// the part the trim keeps (a bolt hole at the foot sends the pair to the
/// exact solids at all).
#[test]
fn a_trim_away_from_the_overlap_does_not_excuse_it() {
    let source = "leg = box(\"leg\", (100, 100, 800))\n\
                  hole(leg, \"x-\", world = (0, 50, 40), diameter = 10, depth = 100, \
                  through = True, id = \"bolt\")\n\
                  trim(leg, (0, 0, 700), (0, 0, 1))\n\
                  inner = box(\"inner\", (50, 50, 400), at = (25, 25, 100))\n";
    assert_certain_collision(source, "leg", "inner");
}

/// Holes that could remove all the volume the boxes share leave the
/// question to the exact solids.
#[test]
fn boards_whose_holes_could_remove_the_shared_volume_stay_unverified() {
    let source = "a = board(\"a\", (100, 100, 18))\n\
                  hole(a, \"z+\", world = (75, 50, 18), diameter = 150, depth = 18, id = \"h\")\n\
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
