//! Continuous rigid-motion clearance, using the same isolated exact pair queries as static checks.
use super::*;
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::assembly_joint::AssemblyJointKind;
use ketchup_model::document::Transform;

/// One moving solid against one fixed solid. `zero_transform` places the moving
/// graph in the joint's frame at position zero; the axis is expressed in that frame.
/// Both transforms and the axis must use the same world frame. Other driven joints
/// must be resolved by the caller, not silently treated as this one rigid motion.
pub struct ExactMotionPair {
    pub moving: ExactBRepGraph,
    pub obstacle: ExactBRepGraph,
    pub zero_transform: Transform,
    pub obstacle_transform: Transform,
    pub motion: AssemblyJointKind,
    pub from: f64,
    pub to: f64,
    pub tolerance_mm: f64,
    /// Permit the existing native contact tolerance only with a whole-interval hull proof.
    pub allow_contact: bool,
}

#[derive(Clone, Copy)]
struct Interval {
    from: f64,
    to: f64,
    depth: u32,
}

impl Interval {
    fn midpoint(self) -> f64 {
        self.from * 0.5 + self.to * 0.5
    }
    fn json(self) -> Value {
        json!([self.from, self.to])
    }
}

const MAX_MOTION_QUERIES: usize = 256;
const MAX_MOTION_DEPTH: u32 = 24;

impl ExactMotionPair {
    fn bounds(&self) -> Option<Aabb> {
        let coordinates = bounds::certified_bounds(&self.moving)?
            .world(*self.zero_transform.matrix(), 0.0)?
            .coordinates();
        Aabb::new(coordinates[0], coordinates[1]).ok()
    }

    fn candidate(&self, position: f64) -> Option<ExactPairCandidate> {
        let transform = self.motion.with_position(position)?.transform_from_zero()?;
        Some(ExactPairCandidate {
            left_graph: 0,
            right_graph: 1,
            left_transform: *transform.compose(self.zero_transform).matrix(),
            right_transform: *self.obstacle_transform.matrix(),
        })
    }

    fn contact_envelope(&self, interval: Interval) -> Option<hull::HullRelation> {
        if !self.allow_contact || !matches!(self.motion, AssemblyJointKind::Prismatic { .. }) {
            return None;
        }
        let start = self.candidate(interval.from)?;
        let end = self.candidate(interval.to)?;
        let local = hull::local_hull(&self.moving)?;
        let swept = local
            .world(start.left_transform)?
            .translation_envelope(local.world(end.left_transform)?)?;
        let obstacle = hull::local_hull(&self.obstacle)?.world(start.right_transform)?;
        Some(hull::relate(&swept, &obstacle, self.tolerance_mm))
    }

    fn travel(&self, bounds: Aabb, interval: Interval) -> Option<f64> {
        let mid = interval.midpoint();
        let left = self
            .motion
            .interval_displacement_bound_mm(bounds, mid, interval.from)?;
        let right = self
            .motion
            .interval_displacement_bound_mm(bounds, mid, interval.to)?;
        Some(left.max(right))
    }
}

/// Read-only clearance over the entire requested interval, not just sampled poses.
/// A positive native common volume proves a collision. An exact distance greater
/// than the conservative travel bound plus tolerance proves a whole interval clear.
/// Contact requires opt-in and a whole translation swept-hull proof within native
/// tolerance. Unsupported bounds, cancellation and exhausted work remain incomplete.
pub fn exact_motion_pair_with_worker(
    pair: &ExactMotionPair,
    sources: &BTreeMap<String, Vec<u8>>,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
) -> Value {
    let started = Instant::now();
    let mut report = json!({
        "state": "incomplete", "complete": false,
        "method": "occt_distance_with_interval_displacement_bound",
        "requested_interval": [pair.from, pair.to],
        "contact_policy": if pair.allow_contact { "native_tolerance_with_translation_swept_hull" } else { "positive_clearance" },
        "position_unit": if matches!(pair.motion, AssemblyJointKind::Prismatic { .. }) { "mm" } else { "degrees" },
        "checked_pose_count": 0, "verified_intervals": [], "unresolved_intervals": [],
        "issues": [], "not_evaluated": [],
        "resource_limits": {"max_queries": MAX_MOTION_QUERIES, "max_depth": MAX_MOTION_DEPTH}
    });
    if !pair.tolerance_mm.is_finite()
        || pair.tolerance_mm < 0.0
        || pair.candidate(pair.from).is_none()
        || pair.candidate(pair.to).is_none()
    {
        report["not_evaluated"] = json!([{"reason": "invalid_motion_interval_or_tolerance"}]);
        return report;
    }
    let graphs = vec![pair.moving.clone(), pair.obstacle.clone()];
    if let Some(reason) = graph_failure(&graphs) {
        report["not_evaluated"] = json!([reason]);
        return report;
    }
    let worker_path = worker_path.or_else(|| {
        crate::evaluation::exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
    });
    let full = Interval {
        from: pair.from,
        to: pair.to,
        depth: 0,
    };
    // Endpoints are checked explicitly; they can be penetrating even when the midpoint is clear.
    let mut pending = vec![
        full,
        Interval {
            to: pair.from,
            ..full
        },
        Interval {
            from: pair.to,
            ..full
        },
    ];
    let bounds = pair.bounds();
    let worker_cancelled = Arc::new(AtomicBool::new(false));
    let mut checked = 0;
    let mut verified = Vec::new();
    let mut unresolved = Vec::new();
    let mut issues = Vec::new();
    let mut reasons = Vec::new();
    while !pending.is_empty() {
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= timeout {
            reasons.push(json!({"reason": if cancelled.load(Ordering::Acquire) { "motion_cancelled" } else { "motion_timeout" }}));
            unresolved.extend(pending.iter().map(|i| i.json()));
            break;
        }
        let remaining = MAX_MOTION_QUERIES - checked;
        if remaining == 0 {
            reasons.push(json!({"reason": "motion_query_limit"}));
            unresolved.extend(pending.iter().map(|i| i.json()));
            break;
        }
        let batch: Vec<_> = pending.drain(..pending.len().min(remaining)).collect();
        let candidates: Option<Vec<_>> = batch
            .iter()
            .map(|interval| Some((0, 1, pair.candidate(interval.midpoint())?)))
            .collect();
        let Some(candidates) = candidates else {
            reasons.push(json!({"reason": "invalid_intermediate_motion_pose"}));
            unresolved.extend(batch.iter().chain(&pending).map(|i| i.json()));
            break;
        };
        let receiver = start_pair_batches(
            worker_path.clone(),
            candidates,
            graphs.clone(),
            sources.clone(),
            pair.tolerance_mm,
            Arc::clone(&worker_cancelled),
        );
        let result = receive_pair_batch(
            &receiver,
            Some(&cancelled),
            &worker_cancelled,
            timeout.saturating_sub(started.elapsed()),
        );
        let answers = match result
            .map_err(
                |error| json!({"reason": "motion_worker_wait_failed", "cause": error.to_string()}),
            )
            .and_then(|batch| batch.map_err(|error| error.evidence()))
        {
            Ok(answers) => answers,
            Err(reason) => {
                worker_cancelled.store(true, Ordering::Release);
                reasons.push(reason);
                unresolved.extend(batch.iter().chain(&pending).map(|i| i.json()));
                break;
            }
        };
        checked += answers.len();
        for (interval, (_, _, answer)) in batch.into_iter().zip(answers) {
            classify_interval(
                pair,
                bounds,
                interval,
                answer,
                &mut verified,
                &mut unresolved,
                &mut issues,
                &mut pending,
            );
        }
        if !issues.is_empty() {
            unresolved.extend(pending.iter().map(|i| i.json()));
            break;
        }
    }
    let complete = unresolved.is_empty() && reasons.is_empty();
    report["state"] = json!(if !issues.is_empty() {
        "failed"
    } else if complete {
        "passed"
    } else {
        "incomplete"
    });
    report["complete"] = json!(complete);
    report["checked_pose_count"] = json!(checked);
    report["verified_intervals"] = json!(verified);
    report["unresolved_intervals"] = json!(unresolved);
    report["issues"] = json!(issues);
    if !complete && reasons.is_empty() {
        reasons.push(json!({"reason": if bounds.is_none() { "uncertified_motion_envelope" } else { "unresolved_motion_intervals" }}));
    }
    report["not_evaluated"] = json!(reasons);
    report
}

fn graph_failure(graphs: &[ExactBRepGraph]) -> Option<Value> {
    let mut size = 0usize;
    for graph in graphs {
        match graph.to_bytes() {
            Ok(bytes) => size = size.saturating_add(bytes.len()),
            Err(error) => {
                return Some(json!({"reason": "invalid_exact_graph", "cause": error.to_string()}));
            }
        }
    }
    (size > MAX_COLLISION_GRAPH_BYTES)
        .then(|| json!({"reason": "exact_graph_bytes_resource_limit"}))
}

#[allow(clippy::too_many_arguments)]
fn classify_interval(
    pair: &ExactMotionPair,
    bounds: Option<Aabb>,
    interval: Interval,
    answer: ExactPairQueryResult,
    verified: &mut Vec<Value>,
    unresolved: &mut Vec<Value>,
    issues: &mut Vec<Value>,
    pending: &mut Vec<Interval>,
) {
    if answer.relation == ExactPairRelation::Penetrating {
        issues.push(json!({"kind": "collision", "position": interval.midpoint(),
            "common_volume_mm3": answer.common_volume_mm3, "distance_mm": answer.distance_mm}));
        // A witness proves failure, not that all other positions were checked.
        unresolved.push(interval.json());
        return;
    }
    let travel = if interval.from == interval.to {
        Some(0.0)
    } else {
        bounds.and_then(|b| pair.travel(b, interval))
    };
    if let Some(travel) = travel
        && answer.distance_mm > travel + pair.tolerance_mm
    {
        verified.push(
            json!({"interval": interval.json(), "sample_position": interval.midpoint(),
            "distance_mm": answer.distance_mm, "maximum_displacement_mm": travel,
            "clearance_lower_bound_mm": answer.distance_mm - travel - pair.tolerance_mm}),
        );
        return;
    }
    if matches!(
        pair.contact_envelope(interval),
        Some(hull::HullRelation::Separated | hull::HullRelation::Touching { .. })
    ) {
        verified.push(
            json!({"interval": interval.json(), "sample_position": interval.midpoint(),
            "method": "translation_swept_hull_with_native_contact_tolerance",
            "contact_tolerance_mm": pair.tolerance_mm, "distance_mm": answer.distance_mm}),
        );
        return;
    }
    let mid = interval.midpoint();
    if travel.is_none()
        || interval.depth >= MAX_MOTION_DEPTH
        || mid == interval.from
        || mid == interval.to
    {
        unresolved.push(interval.json());
    } else {
        pending.push(Interval {
            to: mid,
            depth: interval.depth + 1,
            ..interval
        });
        pending.push(Interval {
            from: mid,
            depth: interval.depth + 1,
            ..interval
        });
    }
}
