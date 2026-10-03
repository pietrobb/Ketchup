//! Exact collision and distance answers for applied program solids.
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

/// Bounds can prove separation, but cannot supply an exact distance.
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

/// Re-derives the program report from native answers without changing the model.
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
    let measurements: Vec<_> = ketchup_program::exact::measured_pairs(model)
        .into_iter().map(|(a, b)| {
            let distance = shapes.pair(&a, &b).and_then(|pair| pair.gap_mm());
            json!({
                "parts": [a, b], "kind": "minimum_distance",
                "state": if distance.is_some() { "verified" } else { "not_evaluated" },
                "distance_mm": distance, "unit": "mm", "method": "occt_brep_distance",
                "reason": if distance.is_some() { Value::Null } else { json!("exact_distance_unavailable") }
            })
        }).collect();
    let (unverified_before, collisions_before) = (
        count(report, COLLISION_UNVERIFIED),
        count(report, "collision"),
    );
    report.refine(model, &shapes);
    let reason = exact["not_evaluated"]
        .as_array()
        .and_then(|reasons| reasons.first())
        .and_then(|reason| reason["reason"].as_str())
        .unwrap_or("exact check incomplete");
    let mut issues = std::mem::take(&mut report.issues);
    for issue in &mut issues {
        if issue.kind == COLLISION_UNVERIFIED {
            issue.hint =
                format!("The exact shape check did not finish ({reason}); validate again.");
        }
    }
    report.set_issues(issues);
    let unresolved = count(report, COLLISION_UNVERIFIED);
    let unverified_groups = count(report, "group_contact_unverified");
    let collisions = count(report, "collision");
    json!({
        "state": if complete && unresolved == 0 && unverified_groups == 0 { "verified" } else { "incomplete" },
        "collisions": collisions,
        "cleared": unverified_before.saturating_sub(unresolved + collisions.saturating_sub(collisions_before)),
        "unresolved": unresolved,
        "unverified_groups": unverified_groups,
        "measurements": &measurements[..measurements.len().min(40)],
        "measurements_total": measurements.len(),
        "measurements_truncated": measurements.len() > 40,
        "distance_measurements_state": if measurements.is_empty() { "not_requested" } else if measurements.iter().all(|m| m["state"] == "verified") { "verified" } else { "incomplete" },
        "exact_pair_count": facts.len(),
        "checked_pair_count": exact["checked_pair_count"],
        "not_evaluated": exact["not_evaluated"],
    })
}

/// Automatic exact checks for non-box contacts and declared distances.
pub fn verify_rule_program_exact(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    verify_candidates(
        snapshot,
        model,
        report,
        container,
        worker_path,
        timeout,
        cancelled,
        exact_candidates(model),
    )
}

/// Explicit read-only verification of every visible part owned by the program.
pub fn verify_rule_program_all(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    let candidates = model.parts.iter().map(|part| part.name.clone()).collect();
    verify_candidates(
        snapshot,
        model,
        report,
        container,
        worker_path,
        timeout,
        cancelled,
        candidates,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_candidates(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
    candidates: BTreeSet<String>,
) -> Option<Value> {
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
    let paths: BTreeMap<_, _> = names
        .iter()
        .map(|(path, name)| (name.as_str(), path))
        .collect();
    let requested = ketchup_program::exact::measured_pairs(model)
        .into_iter()
        .filter_map(|(a, b)| {
            Some((
                (*paths.get(a.as_str())?).clone(),
                (*paths.get(b.as_str())?).clone(),
            ))
        });
    // Full paths keep siblings of shared component roots distinct.
    let scope = CollisionScope::bind(snapshot, checked.iter().map(InstancePath::root_occurrence))
        .measuring_pairs(requested);
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
