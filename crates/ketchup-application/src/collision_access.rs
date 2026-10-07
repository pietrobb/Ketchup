//! Auxiliary volumes reuse the exact motion checker without publishing a part.
use super::*;
use ketchup_model::document::DocumentStore;
use ketchup_program::{ProgramModel, motion::ProgramToolAccess};

fn envelope(
    model: &ProgramModel,
    access: &ProgramToolAccess,
    cancelled: &AtomicBool,
) -> Result<motion_program::MotionBody, Value> {
    let (Some(name), Some(kind), Some(limits), Some(start)) =
        (&access.envelope, access.kind, access.limits, access.start)
    else {
        return Err(json!({"reason": "missing_tool_envelope_or_path"}));
    };
    if access.end != 0.0
        || start == access.end
        || !limits.contains(start)
        || !limits.contains(access.end)
        || kind
            .with_position(start)
            .and_then(|k| k.transform_from_zero())
            .is_none()
    {
        return Err(
            json!({"reason": "invalid_tool_approach", "hint": "Use a nonzero approach within its limits ending at zero, the envelope's modeled working pose."}),
        );
    }
    let part = model.tool(name).ok_or_else(|| json!({"reason": "missing_auxiliary_tool_volume", "envelope": name,
        "hint": "Declare a finite solid with tool=True; a point, ray or physical part is not a tool envelope."}))?;
    let mut scratch = DocumentStore::new();
    let batch = crate::planner::plan_rule_part_batch(&scratch, std::slice::from_ref(part))
        .map_err(|error| json!({"reason": "unsupported_tool_envelope", "cause": error}))?;
    scratch
        .apply_batch(&batch)
        .map_err(|error| json!({"reason": "invalid_tool_envelope", "cause": error.to_string()}))?;
    let (mut bodies, unavailable) =
        motion_program::bodies(&scratch.current(), &BTreeSet::new(), cancelled);
    if !unavailable.is_empty() || bodies.len() != 1 {
        return Err(
            json!({"reason": "unavailable_tool_solid", "details": unavailable, "body_count": bodies.len()}),
        );
    }
    Ok(bodies.remove(0))
}

#[allow(clippy::too_many_arguments)]
fn check_access(
    snapshot: &Snapshot,
    model: &ProgramModel,
    access: &ProgramToolAccess,
    obstacles: &[motion_program::MotionBody],
    unavailable: &[Value],
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({"name": access.name, "envelope": access.envelope,
        "state": "incomplete", "complete": false, "pairs": [], "not_evaluated": unavailable,
        "requested_interval": [access.start, access.end], "working_position": 0});
    let tool = match envelope(model, access, &cancelled) {
        Ok(tool) => tool,
        Err(reason) => {
            report["not_evaluated"] = json!([reason]);
            return report;
        }
    };
    let (Some(motion), Some(from)) = (access.kind, access.start) else {
        return report;
    };
    report["working_transform"] = json!(tool.occurrence.transform.matrix());
    let mut pairs = Vec::new();
    let mut reasons = unavailable.to_vec();
    for obstacle in obstacles {
        if pairs.len() >= MAX_PROGRAM_CHECK_PAIRS
            || started.elapsed() >= timeout
            || cancelled.load(Ordering::Acquire)
        {
            reasons.push(json!({"reason": "tool_access_work_limit_or_cancellation"}));
            break;
        }
        let pair = ExactMotionPair {
            moving: tool.graph.clone(),
            obstacle: obstacle.graph.clone(),
            zero_transform: tool.occurrence.transform,
            obstacle_transform: obstacle.occurrence.transform,
            motion,
            from,
            to: access.end,
            tolerance_mm: snapshot.tolerance().linear_mm(),
            allow_contact: false,
        };
        let mut result = exact_motion_pair_with_worker(
            &pair,
            &BTreeMap::new(),
            worker_path.clone(),
            timeout.saturating_sub(started.elapsed()),
            Arc::clone(&cancelled),
        );
        result["obstacle"] = motion_program::body_identity(snapshot, obstacle);
        result["moving"] = json!({"auxiliary_envelope": access.envelope});
        pairs.push(result);
    }
    if started.elapsed() >= timeout || cancelled.load(Ordering::Acquire) {
        reasons.push(json!({"reason": "tool_access_timeout_or_cancellation"}));
    }
    let complete = reasons.is_empty() && pairs.iter().all(|p| p["complete"] == true);
    report["state"] = json!(if pairs.iter().any(|p| p["state"] == "failed") {
        "failed"
    } else if complete {
        "passed"
    } else {
        "incomplete"
    });
    report["complete"] = json!(complete);
    report["not_evaluated"] = json!(reasons);
    motion_program::summarize_pairs(&mut report, pairs);
    report
}

/// Checks only declared approach volumes against the current visible scene.
/// Cutting, engagement, flexible cables, and operator reach are not implied.
pub fn verify_rule_program_tool_access(
    snapshot: &Snapshot,
    model: &ProgramModel,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({"state": "incomplete", "complete": false, "checks": [], "not_evaluated": [],
        "scope": "declared_auxiliary_volumes_against_visible_stationary_solids",
        "assumptions": ["Envelope geometry is at its working pose; motion axis and pivot use program-world coordinates.",
            "Positive clearance is required throughout the approach including both endpoints; cutting and engagement are not evaluated.",
            "Include the complete relevant tool and holder in the envelope. Other parts stay at their current poses."]});
    if model.tool_access.is_empty() || model.tool_access.len() > MAX_PROGRAM_DECLARATIONS {
        report["not_evaluated"] = json!([{"reason": "missing_or_excessive_tool_access_declarations", "limit": MAX_PROGRAM_DECLARATIONS}]);
        return report;
    }
    let (obstacles, unavailable) = motion_program::bodies(snapshot, &BTreeSet::new(), &cancelled);
    let mut checks = Vec::new();
    for access in &model.tool_access {
        if started.elapsed() >= timeout || cancelled.load(Ordering::Acquire) {
            report["not_evaluated"] = json!([{"reason": "tool_access_timeout_or_cancellation"}]);
            break;
        }
        checks.push(check_access(
            snapshot,
            model,
            access,
            &obstacles,
            &unavailable,
            worker_path.clone(),
            timeout.saturating_sub(started.elapsed()),
            Arc::clone(&cancelled),
        ));
    }
    let complete =
        checks.len() == model.tool_access.len() && checks.iter().all(|c| c["complete"] == true);
    report["state"] = json!(if checks.iter().any(|c| c["state"] == "failed") {
        "failed"
    } else if complete {
        "passed"
    } else {
        "incomplete"
    });
    report["complete"] = json!(complete);
    report["checks"] = json!(checks);
    report
}
