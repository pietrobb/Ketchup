//! Review 2026-11-22 R1122ac-PG-1102: a loop placing every board at the same
//! spot overlaps each with every other; the checks measured all 499 500 pairs
//! of 1 000 boards, 28 s, and 5 000 ran past 300 s. They list the first
//! overlaps, say that more overlap, and settle the rest by the groups the
//! parts form. Parts sharing a range on one axis but apart on another are
//! not paired at all.

use ketchup_program::{OverlapStatus, RELATIONS_TRUNCATED, run};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// `count` boards 100 x 100 x 18, all at the origin.
fn crowd(count: usize) -> String {
    format!("for i in range({count}):\n    board(\"b/%d\" % i, (100, 100, 18), at = (0, 0, 0))\n")
}

#[test]
fn boards_on_one_spot_are_reported_overlapping_not_measured_pair_by_pair() {
    let (_, report) = run("crowd.star", &crowd(1_000), &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"));
    let collisions: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "collision")
        .collect();
    assert_eq!(collisions.len(), 101, "{collisions:?}");
    assert!(
        collisions.iter().any(|issue| issue
            .message
            .starts_with("more than 100 pairs of parts overlap")),
        "{collisions:?}"
    );
    assert!(
        report.relations.len() <= 20_000,
        "{}",
        report.relations.len()
    );
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == "floating_part"),
        "the bottom board rests on the floor and holds the rest"
    );
}

#[test]
#[ignore = "time budget, run in the nightly release perf_ step"]
fn perf_five_thousand_boards_on_one_spot_are_checked_in_seconds() {
    let started = Instant::now();
    let (_, report) = run("crowd.star", &crowd(5_000), &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"));
    let elapsed = started.elapsed();
    assert!(
        elapsed < ketchup_test_env::perf_budget(Duration::from_secs(5)),
        "{elapsed:?}"
    );
    assert!(report.issues.iter().any(|issue| issue.kind == "collision"));
}

/// Review 2026-11-24 R1124og-PG-903 (`cap_hides.star`): 70 boards with a blind
/// hole at the origin make 2 415 pairs only the exact solids settle; past the
/// first 2 000 the check stopped, so `p` and `q` further on, overlapping by
/// 50 x 100 x 18 mm, were never paired and the program passed with exit 0.
#[test]
fn a_collision_past_the_listed_unverified_pairs_is_still_found() {
    let source = "for i in range(70):\n    \
                  b = board(\"b/%d\" % i, (100, 100, 18), at = (0, 0, 0))\n    \
                  hole(b, \"z+\", world = (50, 50, 18), diameter = 8, depth = 10, id = \"h\")\n\
                  board(\"p\", (100, 100, 18), at = (1000, 0, 0))\n\
                  board(\"q\", (100, 100, 18), at = (1050, 0, 0))\n";
    let (_, report) =
        run("cap_hides.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let unverified = report
        .issues
        .iter()
        .filter(|issue| issue.kind == ketchup_program::COLLISION_UNVERIFIED)
        .count();
    assert_eq!(
        unverified, 2_001,
        "2 000 listed and one saying more were not"
    );
    let collision = report
        .issues
        .iter()
        .find(|issue| issue.kind == "collision")
        .unwrap_or_else(|| panic!("p and q overlap: {:?}", report.exit_code()));
    assert_eq!(collision.parts, ["p", "q"], "{collision:?}");
    assert_eq!(report.exit_code(), 1);
}

/// Review 2026-11-24 R1124og-PG-904 (`crowd15.star`): past the first 100
/// collisions the check stops, and `x` and `y`, which it never reached, were
/// related as `boxes_only`, "the exact solids do not overlap".
#[test]
fn an_overlap_the_check_never_reached_is_unchecked_not_cleared() {
    let source = format!(
        "{}board(\"x\", (100, 100, 18), at = (1000, 0, 0))\n\
         board(\"y\", (100, 100, 18), at = (1050, 0, 0))\n",
        crowd(15)
    );
    let (_, report) =
        run("crowd15.star", &source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let status = |a: &str, b: &str| {
        report
            .relations
            .iter()
            .find(|relation| relation.parts == [a, b])
            .and_then(|relation| relation.status)
    };
    assert_eq!(status("x", "y"), Some(OverlapStatus::Unchecked));
    assert_eq!(status("b/0", "b/1"), Some(OverlapStatus::Collision));
    // Without the 15 the pair is reached and is a collision.
    let (_, alone) = run(
        "pair.star",
        &source.replace(&crowd(15), ""),
        &BTreeMap::new(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        alone.relations[0].status,
        Some(OverlapStatus::Collision),
        "{:?}",
        alone.relations
    );
}

/// R1124og-PG-904: relations past 20 000 overlapping pairs were dropped
/// without a word; 201 boards on one spot overlap in 20 100 pairs.
#[test]
fn relations_left_out_past_their_limit_are_reported() {
    let (_, report) =
        run("crowd.star", &crowd(201), &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(report.relations.len(), 20_000);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.kind == RELATIONS_TRUNCATED
                && issue.message.starts_with("more than 20000 pairs")),
        "{:?}",
        report
            .issues
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>()
    );
    let (_, fewer) =
        run("crowd.star", &crowd(200), &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        !fewer
            .issues
            .iter()
            .any(|issue| issue.kind == RELATIONS_TRUNCATED)
    );
}

/// The search past the listed unverified pairs stays fast when every pair is
/// one: 1 000 drilled boards on one spot are 499 500 pairs.
#[test]
#[ignore = "time budget, run in the nightly release perf_ step"]
fn perf_a_thousand_drilled_boards_on_one_spot_are_checked_in_seconds() {
    let source = "for i in range(1000):\n    \
                  b = board(\"b/%d\" % i, (100, 100, 18), at = (0, 0, 0))\n    \
                  hole(b, \"z+\", world = (50, 50, 18), diameter = 8, depth = 10, id = \"h\")\n";
    let started = Instant::now();
    let (_, report) =
        run("drilled.star", source, &BTreeMap::new()).unwrap_or_else(|error| panic!("{error}"));
    let elapsed = started.elapsed();
    assert!(
        elapsed < ketchup_test_env::perf_budget(Duration::from_secs(5)),
        "{elapsed:?}"
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.kind == ketchup_program::COLLISION_UNVERIFIED)
    );
}

/// `count` boards 100 x 100 x 18 stacked 82 mm apart: every one shares the
/// x and y range of every other, and none is near another.
fn stack(count: usize) -> String {
    format!(
        "for i in range({count}):\n    board(\"b/%d\" % i, (100, 100, 18), at = (0, 0, i * 100))\n"
    )
}

/// Capability review 1026: the pairs were found along x alone, so boards
/// sharing an x range were measured pair by pair however far apart (5 000
/// stacked boards, 21 s; in a row along x, 0.15 s).
#[test]
#[ignore = "time budget, run in the nightly release perf_ step"]
fn perf_five_thousand_stacked_boards_are_checked_as_fast_as_a_row() {
    let started = Instant::now();
    let (_, report) = run("stack.star", &stack(5_000), &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"));
    let elapsed = started.elapsed();
    assert!(
        elapsed < ketchup_test_env::perf_budget(Duration::from_secs(2)),
        "{elapsed:?}"
    );
    assert!(report.relations.is_empty(), "{:?}", &report.relations[..1]);
}
