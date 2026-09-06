//! Snapshot-bound solid collision evidence, independent of render tessellation.
#[path = "collision_bounds.rs"]
mod bounds;
use crate::validation::{AssistantValidationSelection, assistant_validation_context_base};
use ketchup_core::document::{
    BodyId, FeatureId, InstancePath, InstancePathStep, SceneOccurrence, Snapshot,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_core::exact_product::{ExactResultRegistry, exact_body_terminal_features};
use ketchup_core::exact_validation::{
    GeneralBodyNarrowPhaseRelation, GeneralBodyParticipant, general_body_narrow_phase,
};
use ketchup_core::persistence::ContainerData;
use ketchup_core::prismatic::TolerancePolicy;
use ketchup_core::validation::EvidenceClass;
use ketchup_interaction::spatial::overlapping_bounds_pairs;
use ketchup_scheduler::pair_query::{MAX_EXACT_PAIR_CANDIDATES, MAX_EXACT_PAIR_GRAPHS};
use ketchup_scheduler::{ExactPairCandidate, ExactPairRelation, ExactWorkerSupervisor};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

// Refuse the entire collision workload above this envelope; never truncate it.
const MAX_COLLISION_BODIES: usize = 512;
const MAX_COLLISION_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// No-worker compatibility entry point. Only canonical translated rectangular
/// extrusions are analytically evaluated; all other solids fail closed.
pub fn assistant_validation_context(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
) -> Value {
    let collision = collision_report(snapshot, selection, None);
    assistant_validation_context_base(snapshot, exact_results, selection, collision)
}

/// Shared desktop/session/repair entry point. `None` discovers the worker beside
/// the executable. Container blobs must belong to this snapshot's document.
/// Read-only: no registry publication, canonical mutation, or Undo entry.
pub fn assistant_validation_context_with_worker(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
) -> Value {
    let collision = collision_report(snapshot, selection, Some((container, worker_path, timeout)));
    assistant_validation_context_base(snapshot, exact_results, selection, collision)
}

struct Body {
    occurrence: SceneOccurrence,
    body_id: BodyId,
    producer: FeatureId,
    graph: Option<usize>,
    analytic: Option<GeneralBodyParticipant>,
}
fn path_json(path: &InstancePath) -> Value {
    json!({"root_occurrence_id": path.root_occurrence().0, "steps": path.steps().iter().map(|step| match step {
        InstancePathStep::Group(id) => json!({"group_id": id.0}),
        InstancePathStep::Occurrence(id) => json!({"occurrence_id": id.0}),
    }).collect::<Vec<_>>()})
}
fn identity(body: &Body) -> Value {
    json!({"occurrence_id": body.occurrence.instance_path.root_occurrence().0,
        "instance_path": path_json(&body.occurrence.instance_path), "name": body.occurrence.occurrence_name,
        "definition_id": body.occurrence.definition_id.0, "body_id": body.body_id.0,
        "producer_feature_id": body.producer.0})
}
fn issue(left: &Body, right: &Body, evidence: Value) -> Value {
    json!({"code": "collision.detected", "severity": "error", "evidence_class": "exact",
        "left_occurrence_id": left.occurrence.instance_path.root_occurrence().0,
        "right_occurrence_id": right.occurrence.instance_path.root_occurrence().0,
        "left_name": left.occurrence.occurrence_name, "right_name": right.occurrence.occurrence_name,
        "left_instance_path": path_json(&left.occurrence.instance_path), "right_instance_path": path_json(&right.occurrence.instance_path),
        "left": identity(left), "right": identity(right), "evidence": evidence})
}

fn collision_report(
    snapshot: &Snapshot,
    selection: &AssistantValidationSelection,
    worker: Option<(&ContainerData, Option<PathBuf>, Duration)>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({"state": "skipped", "complete": false, "checked_occurrence_count": 0,
        "checked_body_count": 0, "checked_pair_count": 0, "total_pair_count": 0,
        "broad_phase_rejected_pair_count": 0, "narrow_phase_pair_count": 0,
        "issue_count": 0, "issues_complete": true, "issues": [], "not_evaluated": [], "unavailable_occurrences": [],
        "resource_limits": {"max_bodies": MAX_COLLISION_BODIES, "max_graphs_per_batch": MAX_EXACT_PAIR_GRAPHS,
            "max_pairs_per_batch": MAX_EXACT_PAIR_CANDIDATES, "max_imported_source_bytes": MAX_COLLISION_SOURCE_BYTES},
        "method": if worker.is_some() {"worker_brep_common_volume"} else {"canonical_box_analytic"}});
    if !selection.is_valid() || !selection.requested.contains("collision") {
        return report;
    }
    let visible = snapshot
        .scene_query()
        .into_iter()
        .filter(|o| o.visible)
        .collect::<Vec<_>>();
    report["visible_occurrence_count"] = json!(visible.len());
    if visible.len() > MAX_COLLISION_BODIES {
        report["state"] = json!("not_evaluated");
        report["not_evaluated"] = json!([{"reason": "collision_occurrence_resource_limit", "actual": visible.len(), "limit": MAX_COLLISION_BODIES}]);
        return report;
    }
    let mut bodies = Vec::new();
    let mut graphs = Vec::new();
    let mut graph_indices = BTreeMap::new();
    let mut unavailable = Vec::new();
    let mut failures = Vec::new();
    for occurrence in &visible {
        let terminals = match exact_body_terminal_features(snapshot, occurrence.definition_id) {
            Ok(terminals) => terminals,
            other => {
                unavailable.push(json!({"occurrence_id": occurrence.instance_path.root_occurrence().0,
                    "instance_path": path_json(&occurrence.instance_path), "name": occurrence.occurrence_name,
                    "reason": format!("unavailable_body_producers: {other:?}")}));
                continue;
            }
        };
        for (body_id, producer) in terminals.into_iter().filter(|(id, _)| {
            snapshot
                .definition(occurrence.definition_id)
                .and_then(|definition| definition.body(*id))
                .is_some_and(|body| body.visible())
        }) {
            if bodies.len() == MAX_COLLISION_BODIES {
                report["state"] = json!("not_evaluated");
                report["not_evaluated"] = json!([{"reason": "collision_body_resource_limit", "limit": MAX_COLLISION_BODIES}]);
                return report;
            }
            // Never accept render packages as proof that the canonical solid is a box.
            let analytic = GeneralBodyParticipant::accept(
                snapshot,
                &ExactResultRegistry::default(),
                occurrence.instance_path.clone(),
                TolerancePolicy::default(),
            )
            .ok()
            .filter(|body| matches!(body.evidence_class(), EvidenceClass::Exact));
            let mut body = Body {
                occurrence: occurrence.clone(),
                body_id,
                producer,
                graph: None,
                analytic,
            };
            if worker.is_some() {
                let key = (occurrence.definition_id, producer);
                if let Some(index) = graph_indices.get(&key) {
                    body.graph = Some(*index);
                } else {
                    match ExactBRepGraph::from_snapshot(
                        snapshot,
                        occurrence.definition_id,
                        producer,
                    ) {
                        Ok(graph) => {
                            body.graph = Some(graphs.len());
                            graph_indices.insert(key, graphs.len());
                            graphs.push(graph);
                        }
                        Err(error) => {
                            let mut entry = identity(&body);
                            entry["reason"] = json!(format!("exact_graph_unavailable: {error}"));
                            unavailable.push(entry);
                        }
                    }
                }
            } else if body.analytic.is_none() {
                let mut entry = identity(&body);
                entry["reason"] = json!("exact_worker_required_for_non_box_geometry");
                unavailable.push(entry);
            }
            bodies.push(body);
        }
    }
    let total_pairs = bodies.len() * bodies.len().saturating_sub(1) / 2;
    let mut issues = Vec::new();
    let mut checked = 0;
    let mut broad_rejected = 0;
    let mut checked_bodies = BTreeSet::new();
    if let Some((container, worker_path, timeout)) = worker {
        let sources = (|| -> Result<BTreeMap<String, Vec<u8>>, String> {
            let mut sources = BTreeMap::new();
            let mut bytes = 0usize;
            for graph in &graphs {
                for node in &graph.nodes {
                    if let ExactBRepOperation::ImportedExact {
                        source_sha256,
                        source_byte_len,
                        ..
                    } = &node.operation
                    {
                        let hash = source_sha256
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>();
                        if sources.contains_key(&hash) {
                            continue;
                        }
                        let source = container
                            .blobs()
                            .get(&hash)
                            .ok_or("missing_imported_source_blob")?;
                        bytes = bytes
                            .checked_add(source.len())
                            .ok_or("imported_source_resource_limit")?;
                        if bytes > MAX_COLLISION_SOURCE_BYTES {
                            return Err("imported_source_resource_limit".into());
                        }
                        if source.len() as u64 != *source_byte_len
                            || ketchup_core::graph::sha256_bytes(source) != *source_sha256
                        {
                            return Err("invalid_imported_source_blob".into());
                        }
                        sources.insert(hash, source.clone());
                    }
                }
            }
            Ok(sources)
        })();
        let local_bounds = graphs
            .iter()
            .map(bounds::certified_bounds)
            .collect::<Vec<_>>();
        let world_bounds = bodies
            .iter()
            .map(|body| {
                local_bounds.get(body.graph?).copied().flatten()?.world(
                    *body.occurrence.transform.matrix(),
                    TolerancePolicy::default().epsilon_mm(),
                )
            })
            .collect::<Vec<_>>();
        let mut pairs = Vec::new();
        // Every visible solid must pass native validation, even with no neighbors.
        // These self-queries never contribute issues or checked pair counts.
        for (index, body) in bodies.iter().enumerate() {
            if let Some(graph) = body.graph {
                pairs.push((
                    index,
                    index,
                    ExactPairCandidate {
                        left_graph: graph,
                        right_graph: graph,
                        left_transform: *body.occurrence.transform.matrix(),
                        right_transform: *body.occurrence.transform.matrix(),
                    },
                ));
            }
        }
        let bounded = world_bounds
            .iter()
            .enumerate()
            .filter_map(|(index, bounds)| bounds.map(|bounds| (index, bounds.coordinates())))
            .collect::<Vec<_>>();
        let mut candidates = overlapping_bounds_pairs(
            &bounded
                .iter()
                .map(|(_, coordinates)| *coordinates)
                .collect::<Vec<_>>(),
        )
        .map(|(candidate_pairs, _)| {
            candidate_pairs
                .into_iter()
                .map(|(left, right)| (bounded[left].0, bounded[right].0))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_else(|_| {
            (0..bodies.len())
                .flat_map(|left| (left + 1..bodies.len()).map(move |right| (left, right)))
                .collect()
        });
        // An uncertifiable bound can reject nothing, but it must not disable BVH
        // rejection between all other certified bodies.
        for unbounded in world_bounds
            .iter()
            .enumerate()
            .filter_map(|(index, bounds)| bounds.is_none().then_some(index))
        {
            candidates.extend((0..bodies.len()).filter_map(|other| {
                (other != unbounded).then_some((unbounded.min(other), unbounded.max(other)))
            }));
        }
        broad_rejected = total_pairs.saturating_sub(candidates.len());
        checked = broad_rejected;
        let graph_block = MAX_EXACT_PAIR_GRAPHS / 2;
        let mut candidates = candidates.into_iter().collect::<Vec<_>>();
        candidates
            .sort_by_key(|(left, right)| (left / graph_block, right / graph_block, *left, *right));
        for (left, right) in candidates {
            let (Some(l), Some(r)) = (bodies[left].graph, bodies[right].graph) else {
                continue;
            };
            pairs.push((
                left,
                right,
                ExactPairCandidate {
                    left_graph: l,
                    right_graph: r,
                    left_transform: *bodies[left].occurrence.transform.matrix(),
                    right_transform: *bodies[right].occurrence.transform.matrix(),
                },
            ));
        }
        let path = worker_path.or_else(|| {
            crate::evaluation::exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        match (sources, path) {
            (Err(reason), _) => failures.push(json!({"reason": reason})),
            (Ok(_), _) if pairs.is_empty() => {}
            (Ok(_), None) => failures.push(json!({"reason": "exact_worker_unavailable"})),
            (Ok(sources), Some(path)) => {
                let cancelled = Arc::new(AtomicBool::new(false));
                let cancel_worker = cancelled.clone();
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut supervisor = match ExactWorkerSupervisor::spawn_with_cancellation(
                        path,
                        &cancel_worker,
                    ) {
                        Ok(supervisor) => supervisor,
                        Err(error) => {
                            let _ = tx.send(Err(format!("exact_worker_unavailable: {error}")));
                            return;
                        }
                    };
                    let mut offset = 0;
                    while offset < pairs.len() {
                        let mut end = offset;
                        let mut unique = BTreeSet::new();
                        while end < pairs.len() && end - offset < MAX_EXACT_PAIR_CANDIDATES {
                            let pair = &pairs[end].2;
                            let mut next = unique.clone();
                            next.insert(pair.left_graph);
                            next.insert(pair.right_graph);
                            if next.len() > MAX_EXACT_PAIR_GRAPHS {
                                break;
                            }
                            unique = next;
                            end += 1;
                        }
                        let candidates = pairs[offset..end]
                            .iter()
                            .map(|p| p.2.clone())
                            .collect::<Vec<_>>();
                        match supervisor.query_exact_brep_pairs_with_cancellation(
                            &graphs,
                            &candidates,
                            &sources,
                            TolerancePolicy::default().epsilon_mm(),
                            &cancel_worker,
                        ) {
                            Ok(results) if results.len() == candidates.len() => {
                                let entries = pairs[offset..end]
                                    .iter()
                                    .zip(results)
                                    .map(|((l, r, _), result)| (*l, *r, result))
                                    .collect::<Vec<_>>();
                                if tx.send(Ok(entries)).is_err() {
                                    return;
                                }
                            }
                            result => {
                                let _ =
                                    tx.send(Err(format!("exact_pair_batch_failed: {result:?}")));
                                return;
                            }
                        }
                        offset = end;
                    }
                });
                loop {
                    match rx.recv_timeout(timeout.saturating_sub(started.elapsed())) {
                        Ok(Ok(entries)) => {
                            for (left, right, result) in entries {
                                if left == right {
                                    checked_bodies.insert(left);
                                }
                                if left != right {
                                    checked += 1;
                                    if result.relation == ExactPairRelation::Penetrating {
                                        issues.push(issue(&bodies[left], &bodies[right], json!({"method": "occt_brep_common_volume", "common_volume_mm3": result.common_volume_mm3, "distance_mm": result.distance_mm})));
                                    }
                                }
                            }
                        }
                        Ok(Err(reason)) => {
                            failures.push(json!({"reason": reason}));
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            cancelled.store(true, Ordering::Release);
                            failures.push(json!({"reason": "exact_collision_timeout", "timeout_ms": timeout.as_millis()}));
                            break;
                        }
                    }
                }
            }
        }
    } else {
        for (index, body) in bodies.iter().enumerate() {
            if body.analytic.is_some() {
                checked_bodies.insert(index);
            }
        }
        for left in 0..bodies.len() {
            for right in left + 1..bodies.len() {
                if let (Some(l), Some(r)) = (&bodies[left].analytic, &bodies[right].analytic) {
                    match general_body_narrow_phase(l, r, TolerancePolicy::default()) {
                    Ok(result) => { checked += 1; if result.relation == GeneralBodyNarrowPhaseRelation::Intersecting {
                        issues.push(issue(&bodies[left], &bodies[right], json!({"method": "canonical_box_analytic", "signed_separation_mm": result.signed_separation_mm})));
                    } },
                    Err(error) => failures.push(json!({"left": identity(&bodies[left]), "right": identity(&bodies[right]), "reason": format!("{error:?}")})),
                }
                }
            }
        }
    }
    if !unavailable.is_empty() {
        failures.push(json!({"reason": "incomplete_exact_geometry_coverage"}));
    }
    if checked != total_pairs || checked_bodies.len() != bodies.len() {
        failures.push(json!({"reason": "incomplete_exact_pair_coverage", "unchecked_pair_count": total_pairs.saturating_sub(checked)}));
    }
    let checked_occurrences = checked_bodies
        .iter()
        .map(|i| &bodies[*i].occurrence.instance_path)
        .collect::<BTreeSet<_>>()
        .len();
    report["state"] = json!(if !issues.is_empty() {
        "failed"
    } else if failures.is_empty() {
        "passed"
    } else {
        "not_evaluated"
    });
    report["complete"] = json!(failures.is_empty());
    report["checked_occurrence_count"] = json!(checked_occurrences);
    report["checked_body_count"] = json!(checked_bodies.len());
    report["total_body_count"] = json!(bodies.len());
    report["checked_pair_count"] = json!(checked);
    report["broad_phase_rejected_pair_count"] = json!(broad_rejected);
    report["narrow_phase_pair_count"] = json!(checked - broad_rejected);
    report["total_pair_count"] = json!(total_pairs);
    report["issue_count"] = json!(issues.len());
    report["issues"] = json!(issues);
    report["not_evaluated"] = json!(failures);
    report["unavailable_occurrences"] = json!(unavailable);
    report
}
