//! Bind a declared program motion to canonical solids and full instance paths.
use super::*;
use ketchup_model::document::Transform;
use ketchup_program::{ProgramModel, motion::ProgramMotion};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramMotionCheck {
    pub name: String,
    pub from: f64,
    pub to: f64,
}

pub(super) struct MotionBody {
    pub(super) occurrence: SceneOccurrence,
    body_id: BodyId,
    pub(super) graph: ExactBRepGraph,
    moving: bool,
}

fn selection<'a>(
    model: &'a ProgramModel,
    request: &ProgramMotionCheck,
) -> Result<(&'a ProgramMotion, BTreeSet<String>, Transform), Value> {
    let motion = model
        .motions
        .iter()
        .find(|m| m.name == request.name)
        .ok_or_else(|| json!({"reason": "unknown_motion", "name": request.name}))?;
    if !motion.limits.contains(request.from) || !motion.limits.contains(request.to) {
        return Err(
            json!({"reason": "motion_outside_declared_limits", "limits": [motion.limits.min(), motion.limits.max()]}),
        );
    }
    let moving = model.member_names(&motion.endpoints[1]);
    for other in &model.motions {
        if other.name != motion.name && other.position != 0.0 {
            let parent = model.member_names(&other.endpoints[1]);
            if moving.is_subset(&parent) {
                return Err(
                    json!({"reason": "driven_parent_frame_not_supported", "parent_motion": other.name}),
                );
            }
        }
    }
    let inverse = motion
        .transform()
        .and_then(Transform::rigid_inverse)
        .ok_or_else(|| json!({"reason": "invalid_current_motion_pose"}))?;
    Ok((motion, moving, inverse))
}

pub(super) fn bodies(
    snapshot: &Snapshot,
    moving: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> (Vec<MotionBody>, Vec<Value>) {
    let mut bodies = Vec::new();
    let mut unavailable = Vec::new();
    let scene = match snapshot.scene_query_bounded(
        MAX_COLLISION_BODIES,
        limits::INSTANCE_PATH_STEPS,
        limits::REPORT_TEXT_BYTES,
    ) {
        Ok(scene) => scene,
        Err(error) => {
            return (
                bodies,
                vec![json!({"reason": "motion_scene_limit", "cause": error.to_string()})],
            );
        }
    };
    let preparation = match ExactSnapshotPreparation::new(snapshot) {
        Ok(preparation) => preparation,
        Err(error) => {
            return (
                bodies,
                vec![
                    json!({"reason": "invalid_exact_dependency_graph", "cause": error.to_string()}),
                ],
            );
        }
    };
    let mut bytes = 0usize;
    for occurrence in scene.into_iter().filter(|p| p.visible) {
        if cancelled.load(Ordering::Acquire) {
            unavailable.push(json!({"reason": "motion_cancelled"}));
            break;
        }
        let terminals = match preparation.terminal_features(occurrence.definition_id) {
            Ok(terminals) => terminals,
            Err(error) => {
                unavailable.push(json!({"reason": "unavailable_body_producers", "instance_path": path_json(&occurrence.instance_path), "cause": error.to_string()}));
                continue;
            }
        };
        for (body_id, producer) in terminals {
            if !snapshot
                .definition(occurrence.definition_id)
                .and_then(|d| d.body(body_id))
                .is_some_and(|b| b.visible())
            {
                continue;
            }
            if bodies.len() >= MAX_COLLISION_BODIES || bytes > MAX_COLLISION_GRAPH_BYTES {
                unavailable.push(json!({"reason": "motion_geometry_resource_limit"}));
                return (bodies, unavailable);
            }
            match preparation.graph(occurrence.definition_id, producer) {
                Ok(graph) => {
                    let encoded = match graph.to_bytes() {
                        Ok(encoded) => encoded,
                        Err(error) => {
                            unavailable.push(json!({"reason": "invalid_exact_graph", "cause": error.to_string()}));
                            continue;
                        }
                    };
                    bytes = bytes.saturating_add(encoded.len());
                    if bytes > MAX_COLLISION_GRAPH_BYTES {
                        unavailable.push(json!({"reason": "motion_geometry_resource_limit"}));
                        return (bodies, unavailable);
                    }
                    let name = crate::rule_program_part_name(snapshot, &occurrence.instance_path);
                    bodies.push(MotionBody { moving: name.is_some_and(|n| moving.contains(&n)), occurrence: occurrence.clone(), body_id, graph });
                }
                Err(error) => unavailable.push(json!({"reason": "exact_graph_unavailable", "instance_path": path_json(&occurrence.instance_path), "cause": error.to_string()})),
            }
        }
    }
    (bodies, unavailable)
}

pub(super) fn body_identity(snapshot: &Snapshot, body: &MotionBody) -> Value {
    json!({"name": crate::rule_program_part_name(snapshot, &body.occurrence.instance_path)
        .unwrap_or_else(|| body.occurrence.occurrence_name.clone()),
        "instance_path": path_json(&body.occurrence.instance_path), "body_id": body.body_id.0})
}

/// Check one named motion, with all other joints held at their current poses.
/// Internal pairs of the rigid moving set are excluded; they do not change during
/// this motion. Geometry not owned by the program still participates as obstacles.
pub fn verify_rule_program_motion(
    snapshot: &Snapshot,
    model: &ProgramModel,
    request: &ProgramMotionCheck,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    verify_motion_in_scene(
        snapshot,
        model,
        request,
        &BTreeSet::new(),
        false,
        worker_path,
        timeout,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify_motion_in_scene(
    snapshot: &Snapshot,
    model: &ProgramModel,
    request: &ProgramMotionCheck,
    absent: &BTreeSet<String>,
    allow_contact: bool,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({"state": "incomplete", "complete": false, "name": request.name,
        "requested_interval": [request.from, request.to], "pairs": [], "not_evaluated": [],
        "scope": "moving_visible_solids_against_stationary_visible_solids",
        "assumptions": ["Other joints stay at their current positions; this is not simultaneous motion.",
            "Internal pairs of the rigid moving set are checked by static validation, not this motion check."]});
    let (motion, moving, inverse) = match selection(model, request) {
        Ok(selection) => selection,
        Err(reason) => {
            report["not_evaluated"] = json!([reason]);
            return report;
        }
    };
    let (mut bodies, mut unavailable) = bodies(snapshot, &moving, &cancelled);
    bodies.retain(|b| {
        crate::rule_program_part_name(snapshot, &b.occurrence.instance_path)
            .is_none_or(|name| !absent.contains(&name))
    });
    report["absent_parts"] = json!(absent);
    if started.elapsed() >= timeout || cancelled.load(Ordering::Acquire) {
        unavailable.push(json!({"reason": "motion_timeout_or_cancellation"}));
    }
    let movers = bodies.iter().filter(|b| b.moving).collect::<Vec<_>>();
    let obstacles = bodies.iter().filter(|b| !b.moving).collect::<Vec<_>>();
    if movers.is_empty() {
        unavailable.push(json!({"reason": "no_visible_moving_solids"}));
    }
    let mut pairs = Vec::new();
    let mut failed = false;
    'pairs: for left in &movers {
        for right in &obstacles {
            if pairs.len() >= 128
                || started.elapsed() >= timeout
                || cancelled.load(Ordering::Acquire)
            {
                unavailable.push(json!({"reason": "motion_work_limit_or_cancellation", "checked_pairs": pairs.len(), "total_pairs": movers.len() * obstacles.len()}));
                break 'pairs;
            }
            let pair = ExactMotionPair {
                moving: left.graph.clone(),
                obstacle: right.graph.clone(),
                zero_transform: inverse.compose(left.occurrence.transform),
                obstacle_transform: right.occurrence.transform,
                motion: motion.kind,
                from: request.from,
                to: request.to,
                tolerance_mm: snapshot.tolerance().linear_mm(),
                allow_contact,
            };
            let mut result = exact_motion_pair_with_worker(
                &pair,
                &BTreeMap::new(),
                worker_path.clone(),
                timeout.saturating_sub(started.elapsed()),
                Arc::clone(&cancelled),
            );
            result["moving"] = body_identity(snapshot, left);
            result["obstacle"] = body_identity(snapshot, right);
            failed |= result["state"] == "failed";
            pairs.push(result);
        }
    }
    let complete = unavailable.is_empty() && pairs.iter().all(|p| p["complete"] == true);
    report["state"] = json!(if failed {
        "failed"
    } else if complete {
        "passed"
    } else {
        "incomplete"
    });
    report["complete"] = json!(complete);
    summarize_pairs(&mut report, pairs);
    report["not_evaluated"] = json!(unavailable);
    report
}

pub(super) fn summarize_pairs(report: &mut Value, mut pairs: Vec<Value>) {
    report["pairs_total"] = json!(pairs.len());
    report["pairs_truncated"] = json!(pairs.len() > 16);
    pairs.sort_by_key(|p| {
        if p["state"] == "failed" {
            0
        } else if p["complete"] != true {
            1
        } else {
            2
        }
    });
    pairs.truncate(16);
    for pair in &mut pairs {
        for field in ["verified_intervals", "unresolved_intervals", "issues"] {
            if let Some(items) = pair[field].as_array_mut() {
                let total = items.len();
                items.truncate(8);
                pair[format!("{field}_total")] = json!(total);
                pair[format!("{field}_truncated")] = json!(total > 8);
            }
        }
    }
    report["pairs"] = json!(pairs);
}
