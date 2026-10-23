//! Exact collision and distance answers for applied program solids.
use crate::collision::{CollisionScope, ExactPairFacts, scoped_exact_pairs_with_worker};
use crate::program_isolation::{CheckDeadline, complete_pending_checks, refine_report};
use ketchup_model::document::{InstancePath, Snapshot};
use ketchup_model::persistence::ContainerData;
use ketchup_program::{
    COLLISION_UNVERIFIED, CancelToken, ExactShapes, ProgramModel, Report, exact_candidates,
    model::Part, rechecked,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

/// Most declared distance measurements a report lists in full.
const REPORTED_MEASUREMENTS: usize = 40;

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
        shapes.checked_against(
            checked.iter().map(|id| names[id].clone()),
            names.values().cloned(),
        );
    }
    for ((left, right), fact) in facts {
        if let (Some(left), Some(right)) = (names.get(left), names.get(right)) {
            shapes.insert(left, right, *fact);
        }
    }
    shapes
}

/// Re-derives the program report from native answers without changing the
/// model, in the program worker. A refused re-derivation keeps the report as
/// it was and names why the check is incomplete. `shapes` are the native
/// answers by part name together with any answers kept from an earlier check;
/// they come back when the check is complete.
pub fn apply_exact_pairs(
    report: &mut Report,
    model: &ProgramModel,
    shapes: ExactShapes,
    (exact, facts): &(Value, ExactPairFacts),
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> (Value, Option<ExactShapes>) {
    let mut complete = exact["complete"].as_bool() == Some(true);
    let mut not_evaluated = exact["not_evaluated"].clone();
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
    match refine_report(model, &shapes, deadline, cancel) {
        Ok(refined) => report.apply_refined(refined),
        Err(error) => {
            complete = false;
            // The report keeps its own issues, so it needs those it left for this check.
            complete_pending_checks(report, model, deadline, cancel);
            let refused = json!({ "reason": error.code, "detail": error.message });
            match not_evaluated.as_array_mut() {
                Some(reasons) => reasons.insert(0, refused),
                None => not_evaluated = json!([refused]),
            }
        }
    }
    let reason = not_evaluated
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
    let summary = json!({
        "state": if complete && unresolved == 0 && unverified_groups == 0 { "verified" } else { "incomplete" },
        "collisions": collisions,
        "cleared": unverified_before.saturating_sub(unresolved + collisions.saturating_sub(collisions_before)),
        "unresolved": unresolved,
        "unverified_groups": unverified_groups,
        "measurements": &measurements[..measurements.len().min(REPORTED_MEASUREMENTS)],
        "measurements_total": measurements.len(),
        "measurements_truncated": measurements.len() > REPORTED_MEASUREMENTS,
        "distance_measurements_state": if measurements.is_empty() { "not_requested" } else if measurements.iter().all(|m| m["state"] == "verified") { "verified" } else { "incomplete" },
        "exact_pair_count": facts.len(),
        "checked_pair_count": exact["checked_pair_count"],
        "not_evaluated": not_evaluated,
    });
    (summary, complete.then_some(shapes))
}

/// Automatic exact checks for non-box contacts and declared distances.
pub fn verify_rule_program_exact(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    deadline: CheckDeadline,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    verify_candidates(
        snapshot,
        model,
        report,
        container,
        worker_path,
        deadline,
        cancelled,
        exact_candidates(model),
        None,
    )
    .0
}

/// Explicit read-only verification of every visible part owned by the program.
pub fn verify_rule_program_all(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    deadline: CheckDeadline,
    cancelled: Arc<AtomicBool>,
) -> Option<Value> {
    let candidates = model.parts.iter().map(|part| part.name.clone()).collect();
    verify_candidates(
        snapshot,
        model,
        report,
        container,
        worker_path,
        deadline,
        cancelled,
        candidates,
        None,
    )
    .0
}

/// What a complete exact check of a program answered, with its parts as they
/// were then: the next check of the program measures again only the pairs a
/// change can have touched (review 2026-10-09 §5.2 point 12).
#[derive(Clone, Debug)]
pub struct ExactBaseline {
    /// The document and revision that were checked.
    document: (u64, u64),
    parts: BTreeMap<String, Part>,
    shapes: ExactShapes,
}

/// An edit of a program and what the exact check knew before it.
#[derive(Clone, Copy)]
pub struct ProgramEdit<'a> {
    /// The last complete exact check of the program, if any.
    pub baseline: Option<&'a ExactBaseline>,
    /// The revision the edit was made to.
    pub before: &'a Snapshot,
    /// The revision the edit made.
    pub after: &'a Snapshot,
}

/// [`verify_rule_program_exact`] of `edit.after` after an earlier check of
/// the same program: a pair of parts that are both as the baseline saw them
/// keeps its answer, and only the parts that changed and their neighbours
/// within the contact tolerance are measured again. The baseline counts only
/// when the edit was made to the very revision it checked (`edit.before`);
/// after any other edit, an undo or a check that did not finish, everything
/// is measured. Gives the baseline for the next check when this one is
/// complete.
pub fn verify_rule_program_exact_since(
    edit: ProgramEdit<'_>,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    deadline: CheckDeadline,
    cancelled: Arc<AtomicBool>,
) -> (Option<Value>, Option<ExactBaseline>) {
    verify_candidates(
        edit.after,
        model,
        report,
        container,
        worker_path,
        deadline,
        cancelled,
        exact_candidates(model),
        edit.baseline
            .filter(|baseline| baseline.document == revision(edit.before)),
    )
}

fn revision(snapshot: &Snapshot) -> (u64, u64) {
    (snapshot.document_id().0, snapshot.revision_id())
}

#[allow(clippy::too_many_arguments)]
fn verify_candidates(
    snapshot: &Snapshot,
    model: &ProgramModel,
    report: &mut Report,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    deadline: CheckDeadline,
    cancelled: Arc<AtomicBool>,
    candidates: BTreeSet<String>,
    baseline: Option<&ExactBaseline>,
) -> (Option<Value>, Option<ExactBaseline>) {
    if candidates.is_empty() {
        return (None, None);
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
    let visible: BTreeSet<String> = names.values().cloned().collect();
    let unchanged: BTreeSet<String> = baseline.map_or_else(BTreeSet::new, |baseline| {
        visible
            .iter()
            .filter(|name| {
                baseline
                    .parts
                    .get(*name)
                    .is_some_and(|before| model.part(name) == Some(before))
            })
            .cloned()
            .collect()
    });
    // A declared distance between two unchanged parts keeps its measurement.
    let distance_kept = |(a, b): &(String, String)| {
        unchanged.contains(a)
            && unchanged.contains(b)
            && baseline
                .and_then(|baseline| baseline.shapes.pair(a, b))
                .and_then(|pair| pair.gap_mm())
                .is_some()
    };
    let distances: Vec<_> = ketchup_program::exact::measured_pairs(model)
        .into_iter()
        .filter(|pair| !distance_kept(pair))
        .collect();
    let mut measured = if unchanged.is_empty() {
        candidates
    } else {
        let changed = visible.difference(&unchanged).cloned().collect();
        rechecked(model, &candidates, &unchanged, &changed)
    };
    measured.extend(distances.iter().flat_map(|(a, b)| [a.clone(), b.clone()]));
    let kept: BTreeSet<String> = unchanged.difference(&measured).cloned().collect();
    let reused = baseline.map_or_else(ExactShapes::default, |baseline| {
        baseline
            .shapes
            .kept(&unchanged, &kept, visible.iter().cloned())
    });
    let checked: BTreeSet<InstancePath> = names
        .iter()
        .filter(|(_, name)| measured.contains(*name))
        .map(|(path, _)| path.clone())
        .collect();
    let paths: BTreeMap<_, _> = names
        .iter()
        .map(|(path, name)| (name.as_str(), path))
        .collect();
    let requested: Vec<_> = distances
        .into_iter()
        .filter_map(|(a, b)| {
            Some((
                (*paths.get(a.as_str())?).clone(),
                (*paths.get(b.as_str())?).clone(),
            ))
        })
        .collect();
    let cancel = CancelToken::from(Arc::clone(&cancelled));
    let answer = if checked.is_empty() {
        // Every pair keeps the answer of the baseline.
        let nothing = json!({"complete": true, "not_evaluated": [], "checked_pair_count": 0});
        (nothing, ExactPairFacts::new())
    } else {
        // Full paths keep siblings of shared component roots distinct.
        let scope =
            CollisionScope::bind(snapshot, checked.iter().map(InstancePath::root_occurrence))
                .measuring_pairs(requested);
        scoped_exact_pairs_with_worker(
            snapshot,
            container,
            worker_path,
            deadline.remaining(),
            &scope,
            cancelled,
        )
    };
    let mut shapes = reused;
    shapes.extend(named_shapes(
        &answer.1,
        &names,
        &checked,
        answer.0["complete"].as_bool() == Some(true),
    ));
    let (mut summary, shapes) =
        apply_exact_pairs(report, model, shapes, &answer, deadline, &cancel);
    summary["measured_part_count"] = json!(checked.len());
    summary["kept_part_count"] = json!(kept.len());
    let baseline = shapes.map(|shapes| ExactBaseline {
        document: revision(snapshot),
        parts: visible
            .iter()
            .filter_map(|name| Some((name.clone(), model.part(name)?.clone())))
            .collect(),
        shapes,
    });
    (Some(summary), baseline)
}
