//! A report whose checks did not run, or that lists only part of itself, is
//! never `ok` by default and says what the whole one held (review 2026-10-17
//! D-3, D-4).
use ketchup_program::exact::ExactShapes;
use ketchup_program::{CHECKS_UNVERIFIED, ProgramError, Report, evaluate, refine, report};
use std::collections::BTreeMap;

fn stopped() -> ProgramError {
    ProgramError {
        code: "check_deadline",
        message: "the checks ran out of time".to_owned(),
        location: None,
    }
}

const TWO_BOXES: &str = "box('a', (10, 10, 10))\nbox('b', (10, 10, 10), at = (20, 0, 0))\n";

#[test]
fn a_report_whose_checks_did_not_finish_is_not_ok() {
    let evaluated = evaluate("stopped.star", TWO_BOXES, &BTreeMap::new()).unwrap();
    let unfinished = Report::unfinished(&evaluated, &stopped());
    assert_eq!(unfinished.errors, 0);
    assert!(!unfinished.ok, "nothing was checked");
}

#[test]
fn a_refine_after_stopped_checks_keeps_saying_the_loads_were_not_checked() {
    let evaluated = evaluate("stopped.star", TWO_BOXES, &BTreeMap::new()).unwrap();
    let mut unfinished = Report::unfinished(&evaluated, &stopped());
    unfinished.apply_refined(refine(&evaluated.model, &ExactShapes::default()));
    assert!(
        unfinished
            .issues
            .iter()
            .any(|issue| issue.kind == CHECKS_UNVERIFIED),
        "{:?}",
        unfinished.issues
    );
    assert!(!unfinished.ok);
}

#[test]
fn a_shortened_report_keeps_the_counts_of_the_whole_one() {
    let source = "for i in range(6):\n    box('b%d' % i, (100, 100, 100), at = (0, i * 0.01, 0))\n";
    let evaluated = evaluate("many.star", source, &BTreeMap::new()).unwrap();
    let whole = report(&evaluated);
    assert!(whole.issues.len() > 4 && whole.relations.len() > 4);
    let first = whole.first(2, 1);
    let totals = first.truncated.expect("a shortened report says so");
    assert_eq!(totals.issues, whole.issues.len());
    assert_eq!(totals.errors, whole.errors);
    assert_eq!(totals.relations, whole.relations.len());
    // Shortened again, it still counts the whole report, not the shortened one.
    assert_eq!(first.first(1, 0).truncated, Some(totals));
    assert_eq!(whole.truncated, None);
}
