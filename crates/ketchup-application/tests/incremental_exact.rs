//! After a complete exact check, the next change of the program measures again
//! only the parts it touched and answers as a full check does (review
//! 2026-10-09 §5.2 point 12).
use ketchup_application::program_isolation::CheckDeadline;
use ketchup_application::{
    DocumentSession, ExactBaseline, ProgramEdit, SessionSettings, verify_rule_program_exact_since,
};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use ketchup_model::persistence::ContainerData;
use ketchup_program::{ProgramModel, Report};
use serde_json::Value;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");
const BUDGET: Duration = Duration::from_secs(if cfg!(debug_assertions) { 600 } else { 90 });

fn source(text: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "tiny-house.star".to_owned(),
        source: text.to_owned(),
        overrides: Default::default(),
    }
}

/// The summary, the issues with the relations, and the next baseline of one
/// exact check.
fn check(
    baseline: Option<&ExactBaseline>,
    base: &Snapshot,
    (snapshot, model, report): (&Snapshot, &ProgramModel, &Report),
) -> (Value, Value, Option<ExactBaseline>) {
    let mut report = report.clone();
    let edit = ProgramEdit {
        baseline,
        before: base,
        after: snapshot,
    };
    let (summary, next) = verify_rule_program_exact_since(
        edit,
        model,
        &mut report,
        &ContainerData::default(),
        None,
        CheckDeadline::after(BUDGET),
        Arc::new(AtomicBool::new(false)),
    );
    let summary = summary.expect("the house has exact candidates");
    assert_eq!(summary["state"], "verified", "{summary:#}");
    let answer = serde_json::to_value((&report.issues, &report.relations)).unwrap();
    (summary, answer, next)
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_a_change_measures_only_what_it_touched_and_answers_as_a_full_check() {
    let mut session = DocumentSession::new(SessionSettings::default());
    let first = session.apply_rule_program(source(HOUSE), false).unwrap();
    let first_view = (&first.snapshot, &first.model, &first.report);
    let (full, _, baseline) = check(None, &first.snapshot, first_view);
    let baseline = baseline.expect("a complete check gives a baseline");
    let all = full["measured_part_count"].as_u64().unwrap();
    assert!(all > 100, "{full:#}");

    // A narrower bed touches one part: only it is measured, the rest keep
    // their answers, and the issues are those of a full check.
    let bed = session
        .apply_rule_program(
            source(&HOUSE.replace("BED_W = 1400", "BED_W = 1300")),
            false,
        )
        .unwrap();
    let bed_view = (&bed.snapshot, &bed.model, &bed.report);
    let (since, since_issues, next) = check(Some(&baseline), &first.snapshot, bed_view);
    let (_, full_issues, _) = check(None, &first.snapshot, bed_view);
    assert_eq!(since["measured_part_count"], 1, "{since:#}");
    assert!(
        since["kept_part_count"].as_u64().unwrap() > 100,
        "{since:#}"
    );
    assert_eq!(since_issues, full_issues);
    let next = next.expect("the incremental check gives the next baseline");

    // Holes narrower than the chimney: the floor and roof boards changed and
    // their unchanged chimney neighbour is measured again, so the new
    // collisions are found, the same as a full check finds them.
    let tight = HOUSE
        .replace("BED_W = 1400", "BED_W = 1300")
        .replace("FIRE = 50 ", "FIRE = -20 ");
    let tight = session.apply_rule_program(source(&tight), false).unwrap();
    let tight_view = (&tight.snapshot, &tight.model, &tight.report);
    let (since, since_issues, _) = check(Some(&next), &bed.snapshot, tight_view);
    let (full, full_issues, _) = check(None, &bed.snapshot, tight_view);
    assert!(since["collisions"].as_u64().unwrap() > 0, "{since:#}");
    assert!(
        since["measured_part_count"].as_u64().unwrap() < all,
        "{since:#}"
    );
    assert_eq!(
        since["collisions"], full["collisions"],
        "{since:#}\n{full:#}"
    );
    assert_eq!(since_issues, full_issues);

    // A baseline of another revision than the one changed counts for nothing.
    let (stale, stale_issues, _) = check(Some(&baseline), &bed.snapshot, tight_view);
    assert_eq!(
        stale["measured_part_count"], full["measured_part_count"],
        "{stale:#}"
    );
    assert_eq!(stale["kept_part_count"], 0, "{stale:#}");
    assert_eq!(stale_issues, full_issues);
}
