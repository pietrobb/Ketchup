//! Review 2026-11-22 R1122ac-PG-1102: a loop placing every board at the same
//! spot overlaps each with every other; the checks measured all 499 500 pairs
//! of 1 000 boards, 28 s, and 5 000 ran past 300 s. They list the first
//! overlaps, say that more overlap, and settle the rest by the groups the
//! parts form. Parts sharing a range on one axis but apart on another are
//! not paired at all.

use ketchup_program::run;
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
