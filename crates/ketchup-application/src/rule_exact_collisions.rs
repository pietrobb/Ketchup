//! Exact answers for the pairs of parts a rule program cannot measure on boxes.
//!
//! The program checks compare boxes. When a profile body, a moved face or a
//! boolean makes a part's solid differ from its box, collisions, contacts,
//! floating parts, joints, relations and expectations of its pairs are
//! measured here on the applied solids by the native BRep pair query, the same
//! one the window's collision check runs, and the report is re-derived from
//! those answers.

use crate::collision::{CollisionScope, ExactPairFacts, scoped_exact_pairs_with_worker};
use ketchup_model::document::{InstancePath, Snapshot};
use ketchup_model::persistence::ContainerData;
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
    names: &BTreeMap<InstancePath, String>,
    checked: &BTreeSet<InstancePath>,
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
/// pass over `checked` leaf instances and returns what the pass settled. Pairs
/// the pass did not measure keep their box answer; their unverified
/// overlaps carry the reason.
pub fn apply_exact_pairs(
    report: &mut Report,
    model: &ProgramModel,
    names: &BTreeMap<InstancePath, String>,
    checked: &BTreeSet<InstancePath>,
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
    let unverified_groups = count(report, "group_contact_unverified");
    let collisions = count(report, "collision").saturating_sub(collisions_before);
    json!({
        "state": if complete && unresolved == 0 && unverified_groups == 0 { "verified" } else { "incomplete" },
        "collisions": collisions,
        "cleared": unverified_before.saturating_sub(unresolved + collisions),
        "unresolved": unresolved,
        "unverified_groups": unverified_groups,
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
    let names: BTreeMap<InstancePath, String> = snapshot
        .scene_query()
        .into_iter()
        .filter(|part| part.visible)
        .filter_map(|part| {
            let name = crate::rule_program_part_name(snapshot, &part.instance_path)?;
            model.part(&name)?;
            Some((part.instance_path, name))
        })
        .collect();
    let checked: BTreeSet<InstancePath> = names
        .iter()
        .filter(|(_, name)| candidates.contains(*name))
        .map(|(path, _)| path.clone())
        .collect();
    // The worker scope includes all leaves of each selected root; facts retain
    // their full paths so siblings never collapse into the same pair.
    let scope = CollisionScope::bind(snapshot, checked.iter().map(InstancePath::root_occurrence));
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
