//! Ordered insertion paths reuse the exact motion checker; no global assembly search.
use super::*;
use ketchup_program::ProgramModel;

fn ordered_members(model: &ProgramModel) -> Result<Vec<BTreeSet<String>>, Value> {
    if model.assembly_steps.is_empty() {
        return Err(json!({"reason": "missing_assembly_order_and_paths"}));
    }
    if model.assembly_steps.len() > MAX_PROGRAM_DECLARATIONS {
        return Err(json!({"reason": "assembly_step_limit", "limit": MAX_PROGRAM_DECLARATIONS}));
    }
    let mut inserted = BTreeSet::new();
    let mut members = Vec::new();
    for (index, step) in model.assembly_steps.iter().enumerate() {
        let Some(motion) = model.motions.iter().find(|m| m.name == step.motion) else {
            return Err(
                json!({"reason": "unknown_assembly_motion", "step": index, "motion": step.motion}),
            );
        };
        if !motion.limits.contains(step.from) || !motion.limits.contains(step.to) {
            return Err(
                json!({"reason": "assembly_path_outside_limits", "step": index, "motion": step.motion}),
            );
        }
        if step.to != motion.position || step.from == step.to {
            return Err(
                json!({"reason": "assembly_path_must_end_at_current_pose", "step": index,
                "motion": step.motion, "current_position": motion.position,
                "hint": "Declare a nonzero insertion ending at the modeled final position."}),
            );
        }
        let moving = model.member_names(&motion.endpoints[1]);
        if moving.is_empty() || !inserted.is_disjoint(&moving) {
            return Err(
                json!({"reason": "empty_or_repeated_assembly_members", "step": index,
                "parts": moving, "hint": "Each rigid part/group must be inserted exactly once."}),
            );
        }
        inserted.extend(moving.clone());
        members.push(moving);
    }
    Ok(members)
}

/// Parts not mentioned by any insertion start assembled at their current poses.
/// Future insertion members are absent. After each verified step its members stay
/// at their modeled final poses. Unproven steps stop the sequence, not the editor.
pub fn verify_rule_program_assembly(
    snapshot: &Snapshot,
    model: &ProgramModel,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({"state": "incomplete", "complete": false, "steps": [],
    "not_evaluated": [], "scope": "declared_ordered_rigid_insertions",
    "assumptions": [
        "Parts without an insertion are already present at their modeled final poses.",
        "Future insertion members are absent until their step; installed members remain fixed.",
        "Internal rigid-group collisions and joint retention require separate static validation.",
        "Contact is permitted within native tolerance only with a continuous translation swept-hull proof; endpoint fit alone is not passage."
    ]});
    let members = match ordered_members(model) {
        Ok(members) => members,
        Err(reason) => {
            report["not_evaluated"] = json!([reason]);
            return report;
        }
    };
    let mut absent: BTreeSet<_> = members.iter().flatten().cloned().collect();
    let mut steps = Vec::new();
    for (index, (step, moving)) in model.assembly_steps.iter().zip(&members).enumerate() {
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= timeout {
            report["not_evaluated"] =
                json!([{"reason": "assembly_timeout_or_cancellation", "step": index}]);
            break;
        }
        for part in moving {
            absent.remove(part);
        }
        let request = ProgramMotionCheck {
            name: step.motion.clone(),
            from: step.from,
            to: step.to,
        };
        let mut check = motion_program::verify_motion_in_scene(
            snapshot,
            model,
            &request,
            &absent,
            true,
            worker_path.clone(),
            timeout.saturating_sub(started.elapsed()),
            Arc::clone(&cancelled),
        );
        check["step"] = json!(index);
        check["scope"] = json!("inserting_members_against_already_present_solids");
        let passed = check["state"] == "passed" && check["complete"] == true;
        if check["state"] == "failed" {
            report["state"] = json!("failed");
        }
        steps.push(check);
        if !passed {
            break;
        }
    }
    let complete = steps.len() == members.len()
        && steps
            .iter()
            .all(|step| step["complete"] == true && step["state"] == "passed");
    if complete {
        report["state"] = json!("passed");
    }
    report["complete"] = json!(complete);
    report["steps_checked"] = json!(steps.len());
    report["steps_total"] = json!(members.len());
    report["steps"] = json!(steps);
    report
}
