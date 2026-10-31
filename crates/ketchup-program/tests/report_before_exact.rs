//! The report of an edit whose exact check follows lists only the collisions
//! that check settles; refined without exact answers it is the whole report
//! (review #136).
use ketchup_program::exact::ExactShapes;
use ketchup_program::{
    CHECKS_UNVERIFIED, COLLISION_UNVERIFIED, ProgramError, evaluate, refine, report,
    report_before_exact,
};
use std::collections::BTreeMap;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");
const PROJECT: &str = include_str!("../../../examples/programs/house-project.star");

fn counts(report: &ketchup_program::Report) -> (usize, usize) {
    let count = |kind: &str| {
        report
            .issues
            .iter()
            .filter(|issue| issue.kind == kind)
            .count()
    };
    (count(COLLISION_UNVERIFIED), count("collision"))
}

fn assert_refined_without_exact_is_whole(file: &str, source: &str) {
    let evaluated = evaluate(file, source, &BTreeMap::new()).unwrap();
    let whole = report(&evaluated);
    let mut before = report_before_exact(&evaluated);
    assert!(before.checks_pending());
    assert!(before.relations.is_empty(), "{file}");
    assert!(!whole.relations.is_empty(), "{file}");
    assert!(before.issues.len() <= whole.issues.len(), "{file}");
    // What the exact check cleared is counted on these.
    assert_eq!(counts(&before), counts(&whole), "{file}");
    assert!(counts(&whole).0 > 0, "{file}");
    assert_eq!(
        (&before.joints, &before.loads, &before.design),
        (&whole.joints, &whole.loads, &whole.design),
        "{file}"
    );
    before.apply_refined(refine(&evaluated.model, &ExactShapes::default()));
    assert!(!before.checks_pending());
    assert_eq!(before, whole, "{file}");
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_project_report_before_the_exact_check_refined_without_it_is_the_whole_report() {
    assert_refined_without_exact_is_whole("house-project.star", PROJECT);
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_report_before_the_exact_check_refined_without_it_is_the_whole_report() {
    assert_refined_without_exact_is_whole("tiny-house.star", HOUSE);
}

#[test]
fn pending_checks_that_did_not_finish_leave_a_report_that_says_so() {
    let source = "box('a', (10, 10, 10))\nbox('b', (10, 10, 10), at = (5, 0, 0))\n";
    let evaluated = evaluate("stopped.star", source, &BTreeMap::new()).unwrap();
    let mut before = report_before_exact(&evaluated);
    let collisions = counts(&before);
    before.stop_pending_checks(&ProgramError {
        code: "time_limit",
        message: "the checks ran out of time".to_owned(),
        location: None,
    });
    assert!(!before.checks_pending());
    assert_eq!(counts(&before), collisions);
    assert!(
        before
            .issues
            .iter()
            .any(|issue| issue.kind == CHECKS_UNVERIFIED)
    );
    assert!(!before.ok);
}
