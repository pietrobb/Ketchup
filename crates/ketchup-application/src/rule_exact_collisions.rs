//! Exact answer for the box overlaps a rule program could not decide itself.
//!
//! The program check compares boxes only. When a profile body or a boolean
//! makes a part's solid differ from its box, it reports the overlap as
//! `collision_unverified`; here the native BRep collision check of the applied
//! document turns each of them into a collision or clears it.

use crate::collision::{CollisionScope, scoped_collision_report_with_worker};
use ketchup_core::document::Snapshot;
use ketchup_core::persistence::ContainerData;
use ketchup_program::{COLLISION_UNVERIFIED, Issue, Report, Severity};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

fn unverified(report: &Report) -> impl Iterator<Item = &Issue> {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == COLLISION_UNVERIFIED && issue.parts.len() == 2)
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

/// Folds an exact collision report into `report`: every unverified overlap of
/// two `checked` parts becomes a collision when the solids penetrate, and
/// disappears when the exact check completed without finding them. Anything
/// else stays unverified with the reason. Returns what the exact check did.
pub fn apply_exact_collisions(
    report: &mut Report,
    checked: &BTreeSet<String>,
    exact: &Value,
) -> Value {
    let complete = exact["complete"].as_bool() == Some(true);
    let penetrating: Vec<((String, String), Value)> = exact["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|issue| {
            let left = issue["left_name"].as_str()?;
            let right = issue["right_name"].as_str()?;
            Some((
                pair_key(left, right),
                issue["evidence"]["common_volume_mm3"].clone(),
            ))
        })
        .collect();
    let reason = exact["not_evaluated"]
        .as_array()
        .and_then(|reasons| reasons.first())
        .and_then(|reason| reason["reason"].as_str())
        .unwrap_or("exact collision check incomplete")
        .to_owned();
    let (mut collisions, mut cleared, mut unresolved) = (0usize, 0usize, 0usize);
    let mut issues = Vec::with_capacity(report.issues.len());
    for issue in std::mem::take(&mut report.issues) {
        if issue.kind != COLLISION_UNVERIFIED || issue.parts.len() != 2 {
            issues.push(issue);
            continue;
        }
        let (a, b) = (&issue.parts[0], &issue.parts[1]);
        let key = pair_key(a, b);
        if let Some((_, volume)) = penetrating.iter().find(|(pair, _)| *pair == key) {
            collisions += 1;
            let volume = volume
                .as_f64()
                .map_or(String::new(), |volume| format!(" ({volume:.1} mm³)"));
            issues.push(Issue {
                severity: Severity::Error,
                kind: "collision",
                message: format!("the solids of {a} and {b} overlap{volume}"),
                hint: "Move or resize one part so they only touch, or subtract one from \
                       the other, or declare a joint with a volume."
                    .to_owned(),
                ..issue
            });
        } else if complete && checked.contains(a) && checked.contains(b) {
            cleared += 1;
        } else {
            unresolved += 1;
            issues.push(Issue {
                hint: format!("The exact shape check did not finish ({reason}); apply again."),
                ..issue
            });
        }
    }
    report.set_issues(issues);
    json!({
        "state": if unresolved == 0 { "verified" } else { "incomplete" },
        "collisions": collisions,
        "cleared": cleared,
        "unresolved": unresolved,
        "checked_pair_count": exact["checked_pair_count"],
        "not_evaluated": exact["not_evaluated"],
    })
}

/// Parts named in the program's unverified overlaps, bound to `snapshot`, or
/// `None` when the program left nothing for the exact check.
#[must_use]
pub fn unverified_collision_scope(
    snapshot: &Snapshot,
    report: &Report,
) -> Option<(CollisionScope, BTreeSet<String>)> {
    let names: BTreeSet<&str> = unverified(report)
        .flat_map(|issue| issue.parts.iter().map(String::as_str))
        .collect();
    if names.is_empty() {
        return None;
    }
    let found: Vec<_> = snapshot
        .occurrences()
        .filter(|occurrence| occurrence.parent().is_none() && names.contains(occurrence.name()))
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect();
    let scope = CollisionScope::bind(snapshot, found.iter().map(|(id, _)| *id));
    Some((scope, found.into_iter().map(|(_, name)| name).collect()))
}

/// Runs the native BRep collision check for the parts of `report`'s
/// unverified overlaps in the applied `snapshot` and folds the answer into
/// `report`. `None` when there was nothing to check.
pub fn verify_rule_program_collisions(
    snapshot: &Snapshot,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    let (scope, checked) = unverified_collision_scope(snapshot, report)?;
    let exact = scoped_collision_report_with_worker(
        snapshot,
        container,
        worker_path,
        timeout,
        &scope,
        cancelled,
    );
    Some(apply_exact_collisions(report, &checked, &exact))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(issues: Vec<Issue>) -> Report {
        let (_, mut report) = ketchup_program::run("empty.star", "", &Default::default())
            .expect("an empty program evaluates");
        report.set_issues(issues);
        report
    }

    fn overlap(a: &str, b: &str) -> Issue {
        Issue {
            severity: Severity::Warning,
            kind: COLLISION_UNVERIFIED,
            parts: vec![a.to_owned(), b.to_owned()],
            message: String::new(),
            where_mm: None,
            hint: String::new(),
        }
    }

    fn names(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn penetrating_pair_becomes_collision_and_clear_pair_disappears() {
        let mut report = report(vec![overlap("seat", "leg_1"), overlap("leg_2", "seat")]);
        let exact = json!({"complete": true, "checked_pair_count": 2, "not_evaluated": [],
            "issues": [{"left_name": "leg_1", "right_name": "seat",
                "evidence": {"common_volume_mm3": 12.5}}]});
        let summary =
            apply_exact_collisions(&mut report, &names(&["seat", "leg_1", "leg_2"]), &exact);
        assert_eq!(summary["state"], "verified");
        assert_eq!(
            (summary["collisions"].clone(), summary["cleared"].clone()),
            (json!(1), json!(1))
        );
        assert_eq!((report.errors, report.warnings, report.ok), (1, 0, false));
        assert_eq!(report.issues[0].kind, "collision");
        assert_eq!(report.issues[0].parts, ["seat", "leg_1"]);
    }

    #[test]
    fn incomplete_check_keeps_overlaps_unverified() {
        let mut report = report(vec![overlap("seat", "leg_1")]);
        let exact = json!({"complete": false, "issues": [],
            "not_evaluated": [{"reason": "exact_worker_unavailable"}]});
        let summary = apply_exact_collisions(&mut report, &names(&["seat", "leg_1"]), &exact);
        assert_eq!(summary["state"], "incomplete");
        assert_eq!((report.errors, report.warnings), (0, 1));
        assert!(report.issues[0].hint.contains("exact_worker_unavailable"));
    }

    #[test]
    fn part_without_occurrence_stays_unverified() {
        let mut report = report(vec![overlap("seat", "ghost")]);
        let exact = json!({"complete": true, "issues": [], "not_evaluated": []});
        let summary = apply_exact_collisions(&mut report, &names(&["seat"]), &exact);
        assert_eq!(summary["unresolved"], 1);
        assert_eq!(report.warnings, 1);
    }
}
