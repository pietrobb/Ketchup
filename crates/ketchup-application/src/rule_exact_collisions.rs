//! Exact answers for the pairs of parts a rule program cannot measure on boxes.
//!
//! The program checks compare boxes. When a profile body, a moved face or a
//! boolean makes a part's solid differ from its box, collisions, contacts,
//! floating parts, joints, relations and expectations of its pairs are
//! measured here on the applied solids by the native BRep pair query, the same
//! one the window's collision check runs, and the report is re-derived from
//! those answers.

use crate::collision::{CollisionScope, ExactPairFacts, scoped_exact_pairs_with_worker};
use ketchup_core::document::{OccurrenceId, Snapshot};
use ketchup_core::persistence::ContainerData;
use ketchup_program::{
    COLLISION_UNVERIFIED, ExactPair, ExactShapes, ProgramModel, Report, exact_candidates,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;

fn count(report: &Report, kind: &str) -> usize {
    report
        .issues
        .iter()
        .filter(|issue| issue.kind == kind)
        .count()
}

/// Named exact answers: every measured pair, and when the check completed,
/// every other pair of a `checked` part with a present part, which the
/// bounds found apart.
fn named_shapes(
    facts: &ExactPairFacts,
    names: &BTreeMap<OccurrenceId, String>,
    checked: &BTreeSet<OccurrenceId>,
    complete: bool,
) -> ExactShapes {
    let mut shapes = ExactShapes::default();
    if complete {
        for id in checked {
            for (_, name) in names.iter().filter(|(other, _)| *other != id) {
                shapes.insert(&names[id], name, ExactPair::default());
            }
        }
    }
    for ((left, right), fact) in facts {
        if let (Some(left), Some(right)) = (names.get(left), names.get(right)) {
            shapes.insert(left, right, *fact);
        }
    }
    shapes
}

/// Re-derives `report` for `model` from the exact answers of one native
/// pass over `checked` occurrences and returns what the pass settled. Pairs
/// the pass did not measure keep their box answer; their unverified
/// overlaps carry the reason.
pub fn apply_exact_pairs(
    report: &mut Report,
    model: &ProgramModel,
    names: &BTreeMap<OccurrenceId, String>,
    checked: &BTreeSet<OccurrenceId>,
    exact: &Value,
    facts: &ExactPairFacts,
) -> Value {
    let complete = exact["complete"].as_bool() == Some(true);
    let shapes = named_shapes(facts, names, checked, complete);
    let (unverified_before, collisions_before) = (
        count(report, COLLISION_UNVERIFIED),
        count(report, "collision"),
    );
    report.refine(model, &shapes);
    let reason = exact["not_evaluated"]
        .as_array()
        .and_then(|reasons| reasons.first())
        .and_then(|reason| reason["reason"].as_str())
        .unwrap_or("exact check incomplete")
        .to_owned();
    let mut issues = std::mem::take(&mut report.issues);
    for issue in &mut issues {
        if issue.kind == COLLISION_UNVERIFIED {
            issue.hint = format!("The exact shape check did not finish ({reason}); apply again.");
        }
    }
    report.set_issues(issues);
    let unresolved = count(report, COLLISION_UNVERIFIED);
    let collisions = count(report, "collision").saturating_sub(collisions_before);
    json!({
        "state": if complete && unresolved == 0 { "verified" } else { "incomplete" },
        "collisions": collisions,
        "cleared": unverified_before.saturating_sub(unresolved + collisions),
        "unresolved": unresolved,
        "exact_pair_count": facts.len(),
        "checked_pair_count": exact["checked_pair_count"],
        "not_evaluated": exact["not_evaluated"],
    })
}

/// Runs the native BRep pair query for the parts of `model` whose boxes
/// misstate their solids in the applied `snapshot` and re-derives `report`
/// from it. `None` when every part is exactly its box, or no such part
/// touches another.
pub fn verify_rule_program_exact(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    let candidates = exact_candidates(model);
    if candidates.is_empty() {
        return None;
    }
    let names: BTreeMap<OccurrenceId, String> = snapshot
        .occurrences()
        .filter(|occurrence| occurrence.parent().is_none())
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect();
    let checked: BTreeSet<OccurrenceId> = names
        .iter()
        .filter(|(_, name)| candidates.contains(*name))
        .map(|(id, _)| *id)
        .collect();
    let scope = CollisionScope::bind(snapshot, checked.iter().copied());
    let (exact, facts) = scoped_exact_pairs_with_worker(
        snapshot,
        container,
        worker_path,
        timeout,
        &scope,
        cancelled,
    );
    Some(apply_exact_pairs(
        report, model, &names, &checked, &exact, &facts,
    ))
}
