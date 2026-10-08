//! Snapshot-bound solid collision evidence, independent of render tessellation.
#[path = "collision_bounds.rs"]
mod bounds;
#[path = "collision_hull.rs"]
mod hull;
#[path = "collision_measurements.rs"]
mod measurements;
#[path = "collision_motion.rs"]
mod motion;
#[path = "collision_order.rs"]
mod order;
use crate::group_connectivity::validation_context as validation_context_with_groups;
use crate::validation::AssistantValidationSelection;
use crate::worker_pool::ExactWorkerUnavailable;
use ketchup_interaction::spatial::{
    SpatialQueryError, overlapping_bounds_for_sources_with_cancellation, overlapping_bounds_pairs,
};
use ketchup_model::document::{
    BodyId, DefinitionId, FeatureId, InstancePath, InstancePathStep, OccurrenceId, SceneOccurrence,
    Snapshot,
};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_model::exact_product::{
    ExactProductError, ExactResultRegistry, ExactSnapshotPreparation,
};
use ketchup_model::exact_validation::{
    GeneralBodyNarrowPhaseRelation, GeneralBodyParticipant, GeneralBodyValidationError,
    GeneralClearanceCase, GravitySupportContact, general_body_input_bytes,
    general_body_narrow_phase, general_body_validation_policy, general_body_validator_descriptor,
};
use ketchup_model::persistence::ContainerData;
use ketchup_model::tolerance::TolerancePolicy;
use ketchup_model::tolerance::limits;
use ketchup_model::validation::{
    DIAGNOSTIC_SCHEMA_V1, DiagnosticLocation, DiagnosticSeverity, EvidenceClass, EvidenceCounts,
    ValidationDiagnostic, ValidationInvocation, ValidationReport, ValidationState,
};
use ketchup_program::ExactPair;
use ketchup_scheduler::pair_query::{MAX_EXACT_PAIR_CANDIDATES, MAX_EXACT_PAIR_GRAPHS};
use ketchup_scheduler::{ExactPairCandidate, ExactPairQueryResult, ExactPairRelation, WorkerError};
use measurements::required_pairs;
pub use motion::{ExactMotionPair, exact_motion_pair_with_worker};
#[path = "collision_motion_program.rs"]
mod motion_program;
pub use motion_program::{ProgramMotionCheck, verify_rule_program_motion};
#[path = "collision_assembly.rs"]
mod assembly;
pub use assembly::verify_rule_program_assembly;
#[path = "collision_access.rs"]
mod access;
pub use access::verify_rule_program_tool_access;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

// Refuse the entire collision workload above this envelope; never truncate it.
const MAX_COLLISION_BODIES: usize = 512;
const MAX_SCOPED_COLLISION_OCCURRENCES: usize = 10_000;
const MAX_SCOPED_COLLISION_BODIES: usize = 10_000;
const MAX_SCOPED_COLLISION_CANDIDATES: usize = 10_000;
const MAX_COLLISION_GRAPH_BYTES: usize = 64 * 1024 * 1024;
// Pair batches send at most MAX_EXACT_PAIR_GRAPHS graphs each; this bounds the whole check.
const MAX_COLLISION_UNIQUE_GRAPHS: usize = 10_000;
const MAX_COLLISION_SOURCE_BYTES: usize = 64 * 1024 * 1024;
// Program checks: most declared assembly steps or tool approaches checked, and most
// pairs one motion or tool-access check evaluates before it reports a work limit.
const MAX_PROGRAM_DECLARATIONS: usize = 128;
const MAX_PROGRAM_CHECK_PAIRS: usize = 128;

/// An occurrence scope bound to one immutable canonical snapshot. Reusing it
/// after any mutation, Undo/Redo, or document replacement fails closed.
#[derive(Clone, Debug)]
pub struct CollisionScope {
    document_id: u64,
    revision: u64,
    canonical_digest: String,
    occurrence_ids: BTreeSet<OccurrenceId>,
    measured_pairs: BTreeSet<(InstancePath, InstancePath)>,
}

impl CollisionScope {
    pub fn bind(
        snapshot: &Snapshot,
        occurrence_ids: impl IntoIterator<Item = OccurrenceId>,
    ) -> Self {
        Self {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            occurrence_ids: occurrence_ids.into_iter().collect(),
            measured_pairs: BTreeSet::new(),
        }
    }

    /// The visible parts of `after` that `before` did not show the same way
    /// (new, moved, shown again, or of a definition that changed or is in
    /// `reshaped`), bound to `after`. Checked against every visible body they
    /// cover the changed parts and their neighbours (AGENTS.md §3). `None`
    /// when no visible part changed.
    pub fn changed_parts(
        before: &Snapshot,
        after: &Snapshot,
        reshaped: &BTreeSet<DefinitionId>,
    ) -> Option<Self> {
        let placed = before
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .map(|occurrence| {
                (
                    occurrence.instance_path,
                    (occurrence.definition_id, occurrence.transform),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let changed = after
            .scene_query()
            .into_iter()
            .filter(|occurrence| {
                occurrence.visible
                    && (placed.get(&occurrence.instance_path)
                        != Some(&(occurrence.definition_id, occurrence.transform))
                        || reshaped.contains(&occurrence.definition_id)
                        || before.definition(occurrence.definition_id)
                            != after.definition(occurrence.definition_id))
            })
            .map(|occurrence| occurrence.instance_path.root_occurrence())
            .collect::<BTreeSet<_>>();
        (!changed.is_empty()).then(|| Self::bind(after, changed))
    }

    fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id().0
            && self.revision == snapshot.revision_id()
            && self.canonical_digest == snapshot.canonical_digest()
    }
}

/// No-worker compatibility entry point. Only canonical translated rectangular
/// extrusions are analytically evaluated; all other solids fail closed.
pub fn assistant_validation_context(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
) -> Value {
    let collision = collision_report(snapshot, selection, None, None, None, None, None);
    validation_context_with_groups(snapshot, exact_results, selection, collision, &[])
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
    assistant_validation_context_with_worker_cancellation(
        snapshot,
        exact_results,
        selection,
        container,
        worker_path,
        timeout,
        Arc::new(AtomicBool::new(false)),
    )
}

pub fn assistant_validation_context_with_worker_cancellation(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancellation: Arc<AtomicBool>,
) -> Value {
    local_validation_context_with_worker_cancellation(
        snapshot,
        exact_results,
        selection,
        container,
        worker_path,
        timeout,
        cancellation,
        None,
    )
}

/// The validation of an edit: collision runs over `changed` (see
/// [`CollisionScope::changed_parts`]) and their neighbours. Gravity and group
/// connectivity follow load and contact paths through the whole model, so a
/// selection that asks for them checks the whole visible model.
#[allow(clippy::too_many_arguments)]
pub fn local_validation_context_with_worker_cancellation(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    selection: &AssistantValidationSelection,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    cancellation: Arc<AtomicBool>,
    changed: Option<&CollisionScope>,
) -> Value {
    let scope = changed.filter(|_| {
        !selection.requested.contains("gravity_support")
            && !selection.requested.contains("group_connectivity")
    });
    let mut gravity_contacts = Vec::new();
    let collision = collision_report(
        snapshot,
        selection,
        Some((container, worker_path, timeout)),
        scope,
        Some(cancellation),
        Some(&mut gravity_contacts),
        None,
    );
    validation_context_with_groups(
        snapshot,
        exact_results,
        selection,
        collision,
        &gravity_contacts,
    )
}

/// Exact collision validation for a snapshot-bound occurrence scope. Spatial
/// bounds may only reject pairs; every retained in-scope or boundary pair is
/// sent through the same native BRep worker as full-model validation.
pub fn scoped_collision_report_with_worker(
    snapshot: &Snapshot,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    scope: &CollisionScope,
    cancelled: Arc<AtomicBool>,
) -> Value {
    collision_report(
        snapshot,
        &AssistantValidationSelection::only(&["collision"]),
        Some((container, worker_path, timeout)),
        Some(scope),
        Some(cancelled),
        None,
        None,
    )
}

pub struct FabricationCollisionValidation {
    pub cases: Vec<GeneralClearanceCase>,
    pub report: ValidationReport,
}

/// Why the fabrication participants cannot be bound to a collision check.
#[derive(Debug, Clone, PartialEq)]
pub enum FabricationCollisionError {
    /// The same body of the same instance was listed twice.
    DuplicateParticipant(InstancePath),
    /// A participant pair is not a valid clearance case.
    InvalidClearance(GeneralBodyValidationError),
}

impl std::fmt::Display for FabricationCollisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateParticipant(path) => {
                write!(f, "duplicate fabrication collision participant {path:?}")
            }
            Self::InvalidClearance(error) => write!(f, "invalid fabrication clearance: {error}"),
        }
    }
}

impl std::error::Error for FabricationCollisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DuplicateParticipant(_) => None,
            Self::InvalidClearance(error) => Some(error),
        }
    }
}

/// Why the imported exact sources of a collision check cannot be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollisionSourceError {
    MissingBlob,
    ResourceLimit,
    InvalidBlob,
}

impl CollisionSourceError {
    fn code(self) -> &'static str {
        match self {
            Self::MissingBlob => "missing_imported_source_blob",
            Self::ResourceLimit => "imported_source_resource_limit",
            Self::InvalidBlob => "invalid_imported_source_blob",
        }
    }
}

/// Bind the ordinary full-model native BRep collision result to the complete,
/// deterministic set of manufacturing participants consumed by fabrication.
pub fn fabrication_collision_validation_with_worker(
    snapshot: &Snapshot,
    participants: &[GeneralBodyParticipant],
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
) -> Result<FabricationCollisionValidation, FabricationCollisionError> {
    let mut participants = participants.to_vec();
    participants.sort_by(|left, right| {
        left.instance_path()
            .cmp(right.instance_path())
            .then_with(|| left.source().cmp(right.source()))
    });
    if let Some(pair) = participants.windows(2).find(|pair| {
        pair[0].instance_path() == pair[1].instance_path() && pair[0].source() == pair[1].source()
    }) {
        return Err(FabricationCollisionError::DuplicateParticipant(
            pair[0].instance_path().clone(),
        ));
    }
    // The cases only bind the participant set to the report: the native check
    // below covers every pair with a participant, not just the listed ones. A
    // chain names each participant with n - 1 cases; all n² pairs would be
    // gigabytes for a model of 1800 members.
    let cases = participants
        .windows(2)
        .map(|pair| {
            GeneralClearanceCase::new(pair[0].clone(), pair[1].clone(), 0.0)
                .map_err(FabricationCollisionError::InvalidClearance)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Scoped to the participants' occurrences: each is checked against every
    // visible body, and the scope admits whole buildings (the unscoped scene stops
    // at 512 occurrences). Without participants the whole model is checked.
    let scope = (!participants.is_empty()).then(|| {
        CollisionScope::bind(
            snapshot,
            participants
                .iter()
                .map(|participant| participant.instance_path().root_occurrence()),
        )
    });
    let collision = collision_report(
        snapshot,
        &AssistantValidationSelection::only(&["collision"]),
        Some((container, worker_path, timeout)),
        scope.as_ref(),
        None,
        None,
        None,
    );
    let policy = general_body_validation_policy();
    let mut descriptor = general_body_validator_descriptor();
    descriptor.implementation_id =
        "ketchup.application.general-bodies.native-brep-common-volume.v1".to_owned();
    descriptor.implementation_version = "1.0.0".to_owned();
    let input = general_body_input_bytes(&cases);
    let invocation = ValidationInvocation::bind(snapshot, &descriptor, &policy, Vec::new(), &input);
    let complete = collision["complete"].as_bool() == Some(true);
    let state = match (complete, collision["state"].as_str()) {
        (true, Some("passed")) => ValidationState::Passed,
        (true, Some("failed")) => ValidationState::Failed,
        _ => ValidationState::NotEvaluated,
    };
    let diagnostics = collision["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|issue| ValidationDiagnostic {
            schema: DIAGNOSTIC_SCHEMA_V1,
            code: issue["code"]
                .as_str()
                .unwrap_or("collision.detected")
                .to_owned(),
            severity: DiagnosticSeverity::Error,
            evidence_class: EvidenceClass::Exact,
            location: DiagnosticLocation {
                entity: None,
                exact_body: None,
                joint: None,
            },
            policy_id: invocation.policy_id.clone(),
            policy_version: invocation.policy_version,
            evidence: issue.to_string(),
        })
        .collect::<Vec<_>>();
    let mut unresolved_conditions = collision["not_evaluated"]
        .as_array()
        .into_iter()
        .flatten()
        .map(Value::to_string)
        .collect::<Vec<_>>();
    unresolved_conditions.extend(
        collision["unavailable_occurrences"]
            .as_array()
            .into_iter()
            .flatten()
            .map(Value::to_string),
    );
    if state == ValidationState::NotEvaluated && unresolved_conditions.is_empty() {
        unresolved_conditions.push("native BRep collision validation was incomplete".to_owned());
    }
    Ok(FabricationCollisionValidation {
        cases,
        report: ValidationReport {
            invocation,
            state,
            evidence_counts: EvidenceCounts {
                exact: collision["checked_pair_count"]
                    .as_u64()
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(0),
                tolerant: 0,
            },
            diagnostics,
            assumptions: vec![
                "certified world bounds reject only disjoint pairs".to_owned(),
                "non-penetrating panel hulls decide their pairs analytically".to_owned(),
                "every other retained pair is evaluated by native BRep common volume".to_owned(),
            ],
            unresolved_conditions,
        },
    })
}

struct Body {
    occurrence: SceneOccurrence,
    body_id: BodyId,
    producer: FeatureId,
    graph: Option<usize>,
    analytic: Option<GeneralBodyParticipant>,
}
/// Why the exact pair check stopped before every candidate pair was answered.
#[derive(Debug)]
enum ExactPairFailure {
    WorkerUnavailable(ExactWorkerUnavailable),
    /// The worker refused a batch of candidate pairs.
    Batch {
        cause: WorkerError,
    },
    /// The worker answered a batch with fewer or more results than pairs.
    Incomplete {
        expected: usize,
        received: usize,
    },
}

impl ExactPairFailure {
    fn cancel_remaining(&self, cancelled: &AtomicBool) -> Value {
        let evidence = if cancelled.load(Ordering::Acquire) {
            json!({"reason": "exact_collision_cancelled"})
        } else {
            self.evidence()
        };
        cancelled.store(true, Ordering::Release);
        evidence
    }

    fn evidence(&self) -> Value {
        match self {
            Self::WorkerUnavailable(ExactWorkerUnavailable::NotFound) => {
                json!({"reason": "exact_worker_unavailable"})
            }
            Self::WorkerUnavailable(ExactWorkerUnavailable::Checkout { cause }) => {
                json!({"reason": "exact_worker_unavailable", "cause": cause.to_string()})
            }
            Self::Batch { cause } => {
                json!({"reason": "exact_pair_batch_failed", "cause": cause.to_string()})
            }
            Self::Incomplete { expected, received } => json!({
                "reason": "exact_pair_batch_incomplete", "expected": expected, "received": received
            }),
        }
    }
}

fn start_pair_batches(
    path: Option<PathBuf>,
    pairs: Vec<(usize, usize, ExactPairCandidate)>,
    graphs: Vec<ExactBRepGraph>,
    sources: BTreeMap<String, Vec<u8>>,
    contact_tolerance_mm: f64,
    cancelled: Arc<AtomicBool>,
) -> mpsc::Receiver<ExactPairBatch> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        send_exact_pair_batches(
            path.as_deref(),
            &pairs,
            &graphs,
            &sources,
            contact_tolerance_mm,
            &cancelled,
            &tx,
        );
    });
    rx
}

fn receive_pair_batch(
    receiver: &mpsc::Receiver<ExactPairBatch>,
    request_cancelled: Option<&AtomicBool>,
    worker_cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<ExactPairBatch, mpsc::RecvTimeoutError> {
    let started = Instant::now();
    loop {
        if request_cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            worker_cancelled.store(true, Ordering::Release);
            return Ok(Err(ExactPairFailure::Batch {
                cause: WorkerError::Cancelled,
            }));
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(10))) {
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() < timeout => continue,
            result => return result,
        }
    }
}

const RANGES_PER_WORKER: usize = 6;

/// Isolated workers for `left_graphs` distinct left solids: one per eight of
/// them, so process startup never dominates, and at most one per two cores.
fn pair_worker_count(left_graphs: usize) -> usize {
    crate::worker_pool::parallel_worker_limit()
        .min(left_graphs / 8)
        .max(1)
}

/// Independent graph ranges use several isolated workers; small checks keep one.
/// Results are forwarded in range order, regardless of worker completion order.
fn send_exact_pair_batches(
    path: Option<&Path>,
    pairs: &[(usize, usize, ExactPairCandidate)],
    graphs: &[ExactBRepGraph],
    sources: &BTreeMap<String, Vec<u8>>,
    contact_tolerance_mm: f64,
    cancelled: &AtomicBool,
    tx: &mpsc::Sender<ExactPairBatch>,
) {
    let left_graphs: BTreeSet<_> = pairs.iter().map(|pair| pair.2.left_graph).collect();
    let workers = pair_worker_count(left_graphs.len());
    if workers == 1 {
        send_serial_pair_batches(
            path,
            pairs,
            graphs,
            sources,
            contact_tolerance_mm,
            cancelled,
            tx,
        );
        return;
    }
    // Pair costs vary widely, so workers take small ranges from a shared queue
    // instead of one fixed share each; a range keeps neighbouring solids together.
    let ranks = order::locality_ranks(
        pairs
            .iter()
            .map(|pair| (pair.2.left_graph, pair.2.right_graph)),
    );
    let mut ordered = pairs.to_vec();
    ordered.sort_by_key(|pair| {
        let (left, right) = (ranks[&pair.2.left_graph], ranks[&pair.2.right_graph]);
        (left.min(right), left.max(right))
    });
    let ranges = ordered
        .chunks(ordered.len().div_ceil(workers * RANGES_PER_WORKER))
        .collect::<Vec<_>>();
    let (senders, receivers): (Vec<_>, Vec<_>) = ranges.iter().map(|_| mpsc::channel()).unzip();
    // A range nobody claims, because every worker stopped, must not block forwarding.
    let senders = std::sync::Mutex::new(senders.into_iter().map(Some).collect::<Vec<_>>());
    let next = AtomicUsize::new(0);
    let running = AtomicUsize::new(workers);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let claim = || {
                    let index = next.fetch_add(1, Ordering::AcqRel);
                    let sender = senders.lock().ok()?.get_mut(index)?.take()?;
                    Some((ranges[index], sender))
                };
                if let Some((range, range_tx)) = claim() {
                    match crate::worker_pool::checkout(path, cancelled) {
                        Ok(mut supervisor) => {
                            let mut current = Some((range, range_tx));
                            while let Some((range, range_tx)) = current.take() {
                                if !send_pair_range(
                                    &mut supervisor,
                                    range,
                                    graphs,
                                    sources,
                                    contact_tolerance_mm,
                                    cancelled,
                                    &range_tx,
                                ) {
                                    break;
                                }
                                current = claim();
                                if current.is_none() {
                                    // Every claimed range succeeded: the worker is reusable.
                                    supervisor.release();
                                    break;
                                }
                            }
                        }
                        Err(unavailable) => {
                            let _ = range_tx
                                .send(Err(ExactPairFailure::WorkerUnavailable(unavailable)));
                        }
                    }
                }
                if running.fetch_sub(1, Ordering::AcqRel) == 1
                    && let Ok(mut senders) = senders.lock()
                {
                    senders.clear();
                }
            });
        }
        for range_rx in receivers {
            for result in range_rx {
                if tx.send(result).is_err() {
                    cancelled.store(true, Ordering::Release);
                    return;
                }
            }
        }
    });
}

/// Asks one exact worker about `pairs` in batches the worker accepts and sends
/// each answered batch, or the failure that stopped the check, to `tx`.
fn send_serial_pair_batches(
    path: Option<&Path>,
    pairs: &[(usize, usize, ExactPairCandidate)],
    graphs: &[ExactBRepGraph],
    sources: &BTreeMap<String, Vec<u8>>,
    contact_tolerance_mm: f64,
    cancelled: &AtomicBool,
    tx: &mpsc::Sender<ExactPairBatch>,
) {
    let mut supervisor = match crate::worker_pool::checkout(path, cancelled) {
        Ok(supervisor) => supervisor,
        Err(unavailable) => {
            if tx
                .send(Err(ExactPairFailure::WorkerUnavailable(unavailable)))
                .is_err()
            {
                cancelled.store(true, Ordering::Release);
            }
            return;
        }
    };
    if send_pair_range(
        &mut supervisor,
        pairs,
        graphs,
        sources,
        contact_tolerance_mm,
        cancelled,
        tx,
    ) {
        supervisor.release();
    }
}

/// Sends every answered batch of `pairs` to `tx`; false when a failure (also
/// sent) or a closed receiver stopped it.
fn send_pair_range(
    supervisor: &mut crate::worker_pool::PooledExactWorker,
    pairs: &[(usize, usize, ExactPairCandidate)],
    graphs: &[ExactBRepGraph],
    sources: &BTreeMap<String, Vec<u8>>,
    contact_tolerance_mm: f64,
    cancelled: &AtomicBool,
    tx: &mpsc::Sender<ExactPairBatch>,
) -> bool {
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
            graphs,
            &candidates,
            sources,
            contact_tolerance_mm,
            cancelled,
        ) {
            Ok(results) if results.len() == candidates.len() => {
                let entries = pairs[offset..end]
                    .iter()
                    .zip(results)
                    .map(|((l, r, _), result)| (*l, *r, result))
                    .collect::<Vec<_>>();
                if tx.send(Ok(entries)).is_err() {
                    cancelled.store(true, Ordering::Release);
                    return false;
                }
            }
            Ok(results) => {
                if tx
                    .send(Err(ExactPairFailure::Incomplete {
                        expected: candidates.len(),
                        received: results.len(),
                    }))
                    .is_err()
                {
                    cancelled.store(true, Ordering::Release);
                }
                return false;
            }
            Err(cause) => {
                if tx.send(Err(ExactPairFailure::Batch { cause })).is_err() {
                    cancelled.store(true, Ordering::Release);
                }
                return false;
            }
        }
        offset = end;
    }
    true
}

/// One answered batch of candidate pairs, or why the check stopped.
type ExactPairBatch = Result<Vec<(usize, usize, ExactPairQueryResult)>, ExactPairFailure>;

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

fn add_contact_area(
    areas: &mut BTreeMap<(InstancePath, InstancePath), f64>,
    bodies: &[Body],
    left: usize,
    right: usize,
    area_mm2: f64,
) {
    let (left, right) = (
        &bodies[left].occurrence.instance_path,
        &bodies[right].occurrence.instance_path,
    );
    if left != right {
        let key = if left <= right {
            (left.clone(), right.clone())
        } else {
            (right.clone(), left.clone())
        };
        *areas.entry(key).or_default() += area_mm2;
    }
}

fn contact_candidates(
    snapshot: &Snapshot,
    selection: &AssistantValidationSelection,
    bodies: &[Body],
    scoped: &BTreeSet<usize>,
) -> BTreeSet<(usize, usize)> {
    if !selection.requested.contains("group_connectivity") {
        return BTreeSet::new();
    }
    crate::contact_joints::candidates(
        snapshot,
        bodies.iter().map(|body| &body.occurrence.instance_path),
    )
    .into_iter()
    .filter(|(a, b)| scoped.contains(a) || scoped.contains(b))
    .collect()
}

fn collision_hulls(graphs: &[ExactBRepGraph], bodies: &[Body]) -> Vec<Option<hull::WorldHull>> {
    let local = graphs.iter().map(hull::local_hull).collect::<Vec<_>>();
    bodies
        .iter()
        .map(|body| {
            local
                .get(body.graph?)
                .copied()
                .flatten()?
                .world(*body.occurrence.transform.matrix())
        })
        .collect()
}

/// Exact answers by the full paths of two leaf instances, smaller path first.
pub type ExactPairFacts = BTreeMap<(InstancePath, InstancePath), ExactPair>;

/// Merges body pairs only within the same two leaf instances.
fn add_pair_fact(
    facts: Option<&mut ExactPairFacts>,
    bodies: &[Body],
    (left, right): (usize, usize),
    fact: ExactPair,
    tolerance: TolerancePolicy,
) {
    let Some(facts) = facts else {
        return;
    };
    let (left, right) = (
        &bodies[left].occurrence.instance_path,
        &bodies[right].occurrence.instance_path,
    );
    if left == right {
        return;
    }
    let tolerance = tolerance.linear_mm();
    facts
        .entry((left.min(right).clone(), left.max(right).clone()))
        .and_modify(|known| {
            known.common_volume_mm3 += fact.common_volume_mm3;
            known.contact_area_mm2 += fact.contact_area_mm2;
            known.distance_mm = match (known.distance_mm, fact.distance_mm) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) if a <= tolerance => Some(a),
                _ => None,
            };
        })
        .or_insert(fact);
}

/// Exact answers for every pair of a snapshot-bound occurrence scope with
/// any visible body, from the same native pass as
/// [`scoped_collision_report_with_worker`]. Pairs the bounds reject are
/// apart and have no entry.
pub fn scoped_exact_pairs_with_worker(
    snapshot: &Snapshot,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    timeout: Duration,
    scope: &CollisionScope,
    cancelled: Arc<AtomicBool>,
) -> (Value, ExactPairFacts) {
    let mut facts = ExactPairFacts::new();
    let report = collision_report(
        snapshot,
        &AssistantValidationSelection::only(&["collision"]),
        Some((container, worker_path, timeout)),
        Some(scope),
        Some(cancelled),
        None,
        Some(&mut facts),
    );
    (report, facts)
}

/// The bodies a definition ends in, each with the feature producing it.
type TerminalBodies = Result<Vec<(BodyId, FeatureId)>, ExactProductError>;

/// The visible occurrences. Hidden ones only count against the scene bound;
/// the body limits of the collision check count visible ones. `None` (with the
/// reason in `report`) when the scene is past its bound.
fn visible_scene(snapshot: &Snapshot, report: &mut Value) -> Option<Vec<SceneOccurrence>> {
    match snapshot.scene_query_bounded(
        MAX_SCOPED_COLLISION_OCCURRENCES,
        limits::INSTANCE_PATH_STEPS,
        limits::REPORT_TEXT_BYTES,
    ) {
        Ok(occurrences) => Some(
            occurrences
                .into_iter()
                .filter(|occurrence| occurrence.visible)
                .collect(),
        ),
        Err(exceeded) => {
            report["state"] = json!("not_evaluated");
            report["visible_occurrence_count"] = Value::Null;
            report["visible_occurrence_count_at_least"] = json!(exceeded.observed_at_least);
            report["not_evaluated"] = json!([{
                "reason": "collision_scene_resource_limit",
                "resource": format!("{:?}", exceeded.kind),
                "limit": exceeded.limit,
                "observed_at_least": exceeded.observed_at_least,
            }]);
            None
        }
    }
}

/// The report of a check that stopped before it finished, for `reason`.
fn not_evaluated(mut report: Value, reason: Value) -> Value {
    report["state"] = json!("not_evaluated");
    report["not_evaluated"] = json!([reason]);
    report
}

/// Why the check has to stop now: the request was cancelled, or a scoped check
/// ran past the worker's `timeout`.
fn interrupted(
    cancellation: Option<&Arc<AtomicBool>>,
    scoped: bool,
    timeout: Option<Duration>,
    started: Instant,
) -> Option<Value> {
    if cancellation.is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
        return Some(json!({"reason": "exact_collision_cancelled"}));
    }
    timeout
        .filter(|timeout| scoped && started.elapsed() >= *timeout)
        .map(|timeout| json!({"reason": "exact_collision_timeout", "timeout_ms": timeout.as_millis()}))
}

fn collision_report(
    snapshot: &Snapshot,
    selection: &AssistantValidationSelection,
    worker: Option<(&ContainerData, Option<PathBuf>, Duration)>,
    scope: Option<&CollisionScope>,
    cancellation: Option<Arc<AtomicBool>>,
    mut gravity_contacts: Option<&mut Vec<GravitySupportContact>>,
    pair_facts: Option<&mut ExactPairFacts>,
) -> Value {
    let started = Instant::now();
    let worker_timeout = worker.as_ref().map(|(_, _, timeout)| *timeout);
    let mut group_facts = ExactPairFacts::new();
    let mut pair_facts = if selection.requested.contains("group_connectivity") {
        Some(&mut group_facts)
    } else {
        pair_facts
    };
    let tolerance = snapshot.tolerance();
    let mut report = json!({"document_id": snapshot.document_id().0,
        "revision": snapshot.revision_id(), "canonical_digest": snapshot.canonical_digest(),
        "state": "skipped", "complete": false, "checked_occurrence_count": 0,
        "checked_body_count": 0, "checked_pair_count": 0, "total_pair_count": 0,
        "broad_phase_rejected_pair_count": 0, "narrow_phase_pair_count": 0,
        "issue_count": 0, "issues_complete": true, "issues": [], "not_evaluated": [], "unavailable_occurrences": [],
        "resource_limits": {"max_bodies": MAX_COLLISION_BODIES,
            "max_scoped_occurrences": MAX_SCOPED_COLLISION_OCCURRENCES,
            "max_scoped_bodies": MAX_SCOPED_COLLISION_BODIES,
            "max_scoped_candidates": MAX_SCOPED_COLLISION_CANDIDATES,
            "max_scene_path_steps": limits::INSTANCE_PATH_STEPS,
            "max_scene_text_bytes": limits::REPORT_TEXT_BYTES,
            "max_graph_bytes": MAX_COLLISION_GRAPH_BYTES,
            "max_unique_graphs": MAX_COLLISION_UNIQUE_GRAPHS,
            "max_graphs_per_batch": MAX_EXACT_PAIR_GRAPHS,
            "max_pairs_per_batch": MAX_EXACT_PAIR_CANDIDATES, "max_imported_source_bytes": MAX_COLLISION_SOURCE_BYTES},
        "method": if worker.is_some() {"worker_brep_common_volume"} else {"canonical_box_analytic"}});
    let collect_gravity_contacts = worker.is_some()
        && gravity_contacts.is_some()
        && selection.requested.contains("gravity_support");
    if !selection.is_valid()
        || (!selection.requested.contains("collision")
            && !collect_gravity_contacts
            && !selection.requested.contains("group_connectivity"))
    {
        return report;
    }
    if let Some(reason) = interrupted(cancellation.as_ref(), scope.is_some(), None, started) {
        return not_evaluated(report, reason);
    }
    if let Some(scope) = scope {
        report["scope"] = json!({
            "mode": "snapshot_bound_occurrences",
            "requested_occurrence_count": scope.occurrence_ids.len(),
            "snapshot_bound": true,
        });
        if !scope.is_current(snapshot) {
            return not_evaluated(report, json!({"reason": "stale_collision_scope"}));
        }
        if scope.occurrence_ids.is_empty() {
            return not_evaluated(report, json!({"reason": "empty_collision_scope"}));
        }
        if scope.occurrence_ids.len() > MAX_SCOPED_COLLISION_OCCURRENCES {
            return not_evaluated(
                report,
                json!({
                    "reason": "collision_scope_resource_limit",
                    "actual": scope.occurrence_ids.len(),
                    "limit": MAX_SCOPED_COLLISION_OCCURRENCES,
                }),
            );
        }
        let missing = scope
            .occurrence_ids
            .iter()
            .filter(|id| snapshot.occurrence(**id).is_none())
            .map(|id| id.0)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return not_evaluated(
                report,
                json!({
                    "reason": "missing_collision_scope_occurrences",
                    "occurrence_ids": missing,
                }),
            );
        }
    }
    let Some(visible) = visible_scene(snapshot, &mut report) else {
        return report;
    };
    // A whole model past the small-scene limit (1800 parts) is checked by the
    // spatially indexed scoped pass with every visible occurrence in scope.
    let whole_model_scope = (scope.is_none() && visible.len() > MAX_COLLISION_BODIES).then(|| {
        report["scope"] = json!({
            "mode": "whole_visible_model",
            "requested_occurrence_count": visible.len(),
            "snapshot_bound": true,
        });
        CollisionScope::bind(
            snapshot,
            visible
                .iter()
                .map(|occurrence| occurrence.instance_path.root_occurrence()),
        )
    });
    let scope = scope.or(whole_model_scope.as_ref());
    if let Some(reason) = interrupted(cancellation.as_ref(), scope.is_some(), None, started) {
        return not_evaluated(report, reason);
    }
    let exact_preparation = match ExactSnapshotPreparation::new(snapshot) {
        Ok(preparation) => preparation,
        Err(error) => {
            return not_evaluated(
                report,
                json!({
                    "reason": "invalid_exact_dependency_graph",
                    "detail": format!("{error:?}"),
                }),
            );
        }
    };
    if let Some(reason) = interrupted(
        cancellation.as_ref(),
        scope.is_some(),
        worker_timeout,
        started,
    ) {
        return not_evaluated(report, reason);
    }
    report["visible_occurrence_count"] = json!(visible.len());
    let mut bodies = Vec::new();
    let mut graphs = Vec::new();
    let mut graph_indices = BTreeMap::new();
    let mut graph_failures = BTreeMap::new();
    let mut graph_bytes = 0usize;
    let mut graph_attempt_count = 0usize;
    let mut terminal_cache: BTreeMap<DefinitionId, TerminalBodies> = BTreeMap::new();
    let mut unavailable = Vec::new();
    let mut failures = Vec::new();
    for occurrence in &visible {
        if let Some(reason) = interrupted(
            cancellation.as_ref(),
            scope.is_some(),
            worker_timeout,
            started,
        ) {
            return not_evaluated(report, reason);
        }
        let terminals = terminal_cache
            .entry(occurrence.definition_id)
            .or_insert_with(|| {
                exact_preparation
                    .terminal_features(occurrence.definition_id)
                    .map(|terminals| terminals.into_iter().collect())
            });
        let terminals = match terminals {
            Ok(terminals) => terminals,
            Err(error) => {
                unavailable.push(json!({"occurrence_id": occurrence.instance_path.root_occurrence().0,
                    "instance_path": path_json(&occurrence.instance_path), "name": occurrence.occurrence_name,
                    "reason": format!("unavailable_body_producers: {error:?}")}));
                continue;
            }
        };
        for (body_id, producer) in terminals.iter().copied().filter(|(id, _)| {
            snapshot
                .definition(occurrence.definition_id)
                .and_then(|definition| definition.body(*id))
                .is_some_and(|body| body.visible())
        }) {
            if let Some(reason) = interrupted(
                cancellation.as_ref(),
                scope.is_some(),
                worker_timeout,
                started,
            ) {
                return not_evaluated(report, reason);
            }
            if scope.is_none() && bodies.len() == MAX_COLLISION_BODIES {
                return not_evaluated(
                    report,
                    json!({"reason": "collision_body_resource_limit", "limit": MAX_COLLISION_BODIES}),
                );
            }
            if scope.is_some() && bodies.len() == MAX_SCOPED_COLLISION_BODIES {
                return not_evaluated(
                    report,
                    json!({"reason": "scoped_collision_body_resource_limit",
                    "limit": MAX_SCOPED_COLLISION_BODIES}),
                );
            }
            // Never accept render packages as proof that the canonical solid is a box.
            // Native-worker validation does not consume this analytic fallback.
            let analytic = worker
                .is_none()
                .then(|| {
                    GeneralBodyParticipant::accept(
                        snapshot,
                        &ExactResultRegistry::default(),
                        occurrence.instance_path.clone(),
                        tolerance,
                    )
                    .ok()
                    .filter(|body| matches!(body.evidence_class(), EvidenceClass::Exact))
                })
                .flatten();
            let mut body = Body {
                occurrence: occurrence.clone(),
                body_id,
                producer,
                graph: None,
                analytic,
            };
            if let Some((_, _, timeout)) = &worker {
                let key = (occurrence.definition_id, producer);
                if let Some(index) = graph_indices.get(&key) {
                    body.graph = Some(*index);
                } else if let Some(reason) = graph_failures.get(&key) {
                    let mut entry = identity(&body);
                    entry["reason"] = json!(reason);
                    unavailable.push(entry);
                } else {
                    if graph_attempt_count == MAX_COLLISION_UNIQUE_GRAPHS {
                        return not_evaluated(
                            report,
                            json!({
                                "reason": "exact_graph_count_resource_limit",
                                "limit": MAX_COLLISION_UNIQUE_GRAPHS,
                            }),
                        );
                    }
                    graph_attempt_count += 1;
                    let graph = exact_preparation.graph(occurrence.definition_id, producer);
                    if let Some(reason) = interrupted(
                        cancellation.as_ref(),
                        scope.is_some(),
                        Some(*timeout),
                        started,
                    )
                    .filter(|_| graph.is_ok())
                    {
                        return not_evaluated(report, reason);
                    }
                    match graph {
                        Ok(graph) => match graph.to_bytes() {
                            Ok(bytes)
                                if graph_bytes.saturating_add(bytes.len())
                                    <= MAX_COLLISION_GRAPH_BYTES =>
                            {
                                graph_bytes += bytes.len();
                                body.graph = Some(graphs.len());
                                graph_indices.insert(key, graphs.len());
                                graphs.push(graph);
                            }
                            Ok(bytes) => {
                                return not_evaluated(
                                    report,
                                    json!({
                                        "reason": "exact_graph_bytes_resource_limit",
                                        "limit": MAX_COLLISION_GRAPH_BYTES,
                                        "observed_at_least": graph_bytes.saturating_add(bytes.len()),
                                    }),
                                );
                            }
                            Err(error) => {
                                let reason = format!("exact_graph_unavailable: {error}");
                                graph_failures.insert(key, reason.clone());
                                let mut entry = identity(&body);
                                entry["reason"] = json!(reason);
                                unavailable.push(entry);
                            }
                        },
                        Err(error) => {
                            let reason = format!("exact_graph_unavailable: {error}");
                            graph_failures.insert(key, reason.clone());
                            let mut entry = identity(&body);
                            entry["reason"] = json!(reason);
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
    let scoped_indices = scope
        .map(|scope| {
            bodies
                .iter()
                .enumerate()
                .filter_map(|(index, body)| {
                    scope
                        .occurrence_ids
                        .contains(&body.occurrence.instance_path.root_occurrence())
                        .then_some(index)
                })
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_else(|| (0..bodies.len()).collect());
    if let Some(scope) = scope {
        let resolved = scoped_indices
            .iter()
            .map(|index| bodies[*index].occurrence.instance_path.root_occurrence())
            .collect::<BTreeSet<_>>();
        for id in scope.occurrence_ids.difference(&resolved) {
            unavailable.push(json!({
                "occurrence_id": id.0,
                "reason": "scoped_occurrence_has_no_visible_exact_body",
            }));
        }
        report["scope"]["resolved_occurrence_count"] = json!(resolved.len());
        report["scope"]["scoped_body_count"] = json!(scoped_indices.len());
    }
    let scoped_body_count = scoped_indices.len();
    let total_pairs = if scope.is_some() {
        scoped_body_count * bodies.len().saturating_sub(scoped_body_count)
            + scoped_body_count * scoped_body_count.saturating_sub(1) / 2
    } else {
        bodies.len() * bodies.len().saturating_sub(1) / 2
    };
    let mut issues = Vec::new();
    let mut exact_contact_areas = BTreeMap::<(InstancePath, InstancePath), f64>::new();
    let mut checked = 0;
    let mut broad_rejected = 0;
    let mut checked_bodies = BTreeSet::new();
    if let Some((container, worker_path, timeout)) = worker {
        if let Some(reason) = interrupted(
            cancellation.as_ref(),
            scope.is_some(),
            Some(timeout),
            started,
        ) {
            return not_evaluated(report, reason);
        }
        let sources = (|| -> Result<BTreeMap<String, Vec<u8>>, CollisionSourceError> {
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
                            .ok_or(CollisionSourceError::MissingBlob)?;
                        bytes = bytes
                            .checked_add(source.len())
                            .ok_or(CollisionSourceError::ResourceLimit)?;
                        if bytes > MAX_COLLISION_SOURCE_BYTES {
                            return Err(CollisionSourceError::ResourceLimit);
                        }
                        if source.len() as u64 != *source_byte_len
                            || ketchup_model::graph::sha256_bytes(source) != *source_sha256
                        {
                            return Err(CollisionSourceError::InvalidBlob);
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
                local_bounds
                    .get(body.graph?)
                    .copied()
                    .flatten()?
                    .world(*body.occurrence.transform.matrix(), tolerance.linear_mm())
            })
            .collect::<Vec<_>>();
        let mut pairs = Vec::new();
        // Every visible solid must pass native validation, even with no neighbors.
        // These self-queries never contribute issues or checked pair counts.
        for (index, body) in bodies.iter().enumerate() {
            if !scoped_indices.contains(&index) {
                continue;
            }
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
        let bounded_scoped_count = bounded
            .iter()
            .filter(|(index, _)| scoped_indices.contains(index))
            .count();
        let bounded_relevant_pairs = bounded_scoped_count
            * bounded.len().saturating_sub(bounded_scoped_count)
            + bounded_scoped_count * bounded_scoped_count.saturating_sub(1) / 2;
        let bounded_coordinates = bounded
            .iter()
            .map(|(_, coordinates)| *coordinates)
            .collect::<Vec<_>>();
        let never_cancelled = AtomicBool::new(false);
        let spatial_cancelled = cancellation.as_deref().unwrap_or(&never_cancelled);
        let spatial_pairs = if scope.is_some() {
            let scoped_sources = bounded
                .iter()
                .enumerate()
                .filter_map(|(position, (body_index, _))| {
                    scoped_indices.contains(body_index).then_some(position)
                })
                .collect::<Vec<_>>();
            overlapping_bounds_for_sources_with_cancellation(
                &bounded_coordinates,
                &scoped_sources,
                MAX_SCOPED_COLLISION_CANDIDATES,
                spatial_cancelled,
            )
        } else {
            overlapping_bounds_pairs(&bounded_coordinates)
        };
        let contact_pairs = required_pairs(snapshot, selection, &bodies, &scoped_indices, scope);
        let mut spatial_complete = true;
        let mut candidates = match spatial_pairs {
            Ok((candidate_pairs, _)) => candidate_pairs
                .into_iter()
                .map(|(left, right)| (bounded[left].0, bounded[right].0))
                .collect::<BTreeSet<_>>(),
            Err(error) if scope.is_some() => {
                spatial_complete = false;
                failures.push(json!({
                    "reason": if error == SpatialQueryError::CandidateLimitExceeded {
                        "collision_candidate_resource_limit"
                    } else {
                        "collision_spatial_index_failed"
                    },
                    "detail": format!("{error:?}"),
                    "limit": MAX_SCOPED_COLLISION_CANDIDATES,
                }));
                BTreeSet::new()
            }
            Err(_) => (0..bodies.len())
                .flat_map(|left| (left + 1..bodies.len()).map(move |right| (left, right)))
                .collect(),
        };
        if let Some(reason) = interrupted(
            cancellation.as_ref(),
            scope.is_some(),
            Some(timeout),
            started,
        ) {
            return not_evaluated(report, reason);
        }
        candidates.extend(contact_pairs.iter().copied());
        let unbounded = world_bounds
            .iter()
            .enumerate()
            .filter_map(|(index, bounds)| bounds.is_none().then_some(index))
            .collect::<Vec<_>>();
        if scope.is_none() {
            // An uncertifiable bound can reject nothing in full-model mode.
            for &unbounded in &unbounded {
                candidates.extend((0..bodies.len()).filter_map(|other| {
                    (other != unbounded).then_some((unbounded.min(other), unbounded.max(other)))
                }));
            }
            broad_rejected = total_pairs.saturating_sub(candidates.len());
        } else {
            broad_rejected = if spatial_complete {
                bounded_relevant_pairs.saturating_sub(candidates.len())
            } else {
                0
            };
            // An uncertifiable bound (e.g. a revolved solid) can reject nothing:
            // pair it with every body it is relevant to, like full-model mode.
            for &unbounded in &unbounded {
                let partners: Vec<usize> = if scoped_indices.contains(&unbounded) {
                    (0..bodies.len()).collect()
                } else {
                    scoped_indices.iter().copied().collect()
                };
                candidates.extend(partners.into_iter().filter_map(|other| {
                    (other != unbounded).then_some((unbounded.min(other), unbounded.max(other)))
                }));
            }
            let boundary_occurrences = candidates
                .iter()
                .flat_map(|(left, right)| [*left, *right])
                .filter(|index| !scoped_indices.contains(index))
                .map(|index| bodies[index].occurrence.instance_path.clone())
                .collect::<BTreeSet<_>>();
            report["scope"]["boundary_occurrence_count"] = json!(boundary_occurrences.len());
            report["scope"]["candidate_coverage_complete"] = json!(spatial_complete);
            report["scope"]["indexed_body_count"] = json!(bounded.len());
        }
        checked = broad_rejected;
        let graph_block = MAX_EXACT_PAIR_GRAPHS / 2;
        let mut candidates = candidates.into_iter().collect::<Vec<_>>();
        candidates
            .sort_by_key(|(left, right)| (left / graph_block, right / graph_block, *left, *right));
        // Separated hulls prove no contact; touching hulls prove contact only for
        // uncut boxes. Cuts may remove the entire bearing face, so ask OCCT.
        let world_hulls = collision_hulls(&graphs, &bodies);
        let mut hull_decided = 0usize;
        for (left, right) in candidates {
            let (Some(l), Some(r)) = (bodies[left].graph, bodies[right].graph) else {
                continue;
            };
            if !contact_pairs.contains(&(left, right))
                && let (Some(a), Some(b)) = (&world_hulls[left], &world_hulls[right])
            {
                let decided = hull::measure(a, b, [&graphs[l], &graphs[r]], tolerance.linear_mm());

                if let Some((area_mm2, distance_mm)) = decided {
                    checked += 1;
                    hull_decided += 1;
                    if collect_gravity_contacts {
                        add_contact_area(&mut exact_contact_areas, &bodies, left, right, area_mm2);
                    }
                    add_pair_fact(
                        pair_facts.as_deref_mut(),
                        &bodies,
                        (left, right),
                        ExactPair {
                            common_volume_mm3: 0.0,
                            contact_area_mm2: area_mm2,
                            distance_mm,
                        },
                        tolerance,
                    );
                    continue;
                }
            }
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
        report["hull_decided_pair_count"] = json!(hull_decided);
        let path = worker_path.or_else(|| {
            crate::evaluation::exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        match (sources, path) {
            (Err(error), _) => failures.push(json!({"reason": error.code()})),
            (Ok(_), _) if pairs.is_empty() => {}
            (Ok(sources), path) => {
                // A failed check stops its workers, not the enclosing edit or chat request.
                let cancelled = Arc::new(AtomicBool::new(false));
                let request_cancelled = cancellation.as_deref();
                let rx = start_pair_batches(
                    path,
                    pairs,
                    graphs,
                    sources,
                    tolerance.linear_mm(),
                    cancelled.clone(),
                );
                loop {
                    match receive_pair_batch(
                        &rx,
                        request_cancelled,
                        &cancelled,
                        timeout.saturating_sub(started.elapsed()),
                    ) {
                        Ok(Ok(entries)) => {
                            for (left, right, result) in entries {
                                if left == right {
                                    checked_bodies.insert(left);
                                }
                                if left != right {
                                    checked += 1;
                                    if collect_gravity_contacts {
                                        add_contact_area(
                                            &mut exact_contact_areas,
                                            &bodies,
                                            left,
                                            right,
                                            result.common_contact_area_mm2,
                                        );
                                    }
                                    add_pair_fact(
                                        pair_facts.as_deref_mut(),
                                        &bodies,
                                        (left, right),
                                        ExactPair {
                                            common_volume_mm3: result.common_volume_mm3,
                                            contact_area_mm2: result.common_contact_area_mm2,
                                            distance_mm: Some(result.distance_mm),
                                        },
                                        tolerance,
                                    );
                                    if result.relation == ExactPairRelation::Penetrating {
                                        issues.push(issue(&bodies[left], &bodies[right], json!({"method": "occt_brep_common_volume", "common_volume_mm3": result.common_volume_mm3, "distance_mm": result.distance_mm})));
                                    }
                                }
                            }
                        }
                        Ok(Err(failure)) => {
                            failures.push(failure.cancel_remaining(&cancelled));
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
                    match general_body_narrow_phase(l, r, tolerance) {
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
    if checked != total_pairs || checked_bodies.len() != scoped_body_count {
        failures.push(json!({"reason": "incomplete_exact_pair_coverage",
            "unchecked_pair_count": total_pairs.saturating_sub(checked),
            "unchecked_scoped_body_count": scoped_body_count.saturating_sub(checked_bodies.len())}));
    }
    if collect_gravity_contacts && failures.is_empty() {
        let contacts = exact_contact_areas
            .into_iter()
            .map(|((left, right), area)| GravitySupportContact::new(left, right, area))
            .collect::<Result<Vec<_>, _>>();
        match contacts {
            Ok(contacts) => {
                if let Some(output) = gravity_contacts.take() {
                    *output = contacts;
                }
            }
            Err(error) => failures.push(json!({
                "reason": "invalid_exact_gravity_contact",
                "detail": format!("{error:?}"),
            })),
        }
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
    report["total_body_count"] = json!(if scope.is_some() {
        scoped_body_count
    } else {
        bodies.len()
    });
    report["model_body_count"] = json!(bodies.len());
    report["graph_bytes"] = json!(graph_bytes);
    report["checked_pair_count"] = json!(checked);
    report["broad_phase_rejected_pair_count"] = json!(broad_rejected);
    report["narrow_phase_pair_count"] = json!(checked - broad_rejected);
    report["total_pair_count"] = json!(total_pairs);
    report["issue_count"] = json!(issues.len());
    report["issues"] = json!(issues);
    report["not_evaluated"] = json!(failures);
    report["unavailable_occurrences"] = json!(unavailable);
    crate::group_connectivity::finish_collision(report, snapshot, selection, pair_facts.as_deref())
}

#[cfg(test)]
#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_parallel_receiver_cancels_workers_and_joins() {
        let _turn = super::integration_support::file_turn();
        let path = crate::evaluation::exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("native exact worker required");
        let mut session = crate::DocumentSession::new(crate::SessionSettings::default());
        let applied = session.apply_rule_program(
            ketchup_model::document::RuleProgramSource {
                file_name: "cancel.star".into(),
                source: "for i in range(20):\n    p=box('part '+str(i),(100,100,18),at=(i*200,0,0))\n    for j in range(32):\n        hole(p,'z+',at=(10+(j%8)*10,10+(j//8)*20),diameter=4,depth=8)\n".into(),
                overrides: BTreeMap::new(),
            }, false).unwrap();
        let mut graphs: Vec<_> = applied
            .snapshot
            .occurrences()
            .map(|occurrence| {
                let definition = applied
                    .snapshot
                    .definition(occurrence.definition_id())
                    .unwrap();
                ExactBRepGraph::from_snapshot(
                    &applied.snapshot,
                    definition.id(),
                    *definition.feature_ids().last().unwrap(),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(graphs.len(), 20);
        // Fail the first range before native loading; the other has real drilled bodies to build.
        graphs[0].graph_digest = "invalid graph identity".into();
        let pairs: Vec<_> = (0..graphs.len())
            .map(|index| {
                (
                    index,
                    index,
                    ExactPairCandidate {
                        left_graph: index,
                        right_graph: index,
                        left_transform: *ketchup_model::document::Transform::identity().matrix(),
                        right_transform: *ketchup_model::document::Transform::identity().matrix(),
                    },
                )
            })
            .collect();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let (finished, done) = mpsc::channel();
        let coordinator = std::thread::spawn(move || {
            send_exact_pair_batches(
                Some(&path),
                &pairs,
                &graphs,
                &BTreeMap::new(),
                0.01,
                &worker_cancelled,
                &tx,
            );
            finished.send(()).unwrap();
        });
        done.recv_timeout(Duration::from_secs(15))
            .expect("cancelled workers must terminate and join");
        coordinator.join().unwrap();
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(
            session.snapshot().scene_query(),
            applied.snapshot.scene_query()
        );
    }

    #[test]
    fn every_worker_a_pair_check_can_start_stays_warm_for_the_next_check() {
        let _turn = super::integration_support::file_turn();
        let path = crate::evaluation::exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("native exact worker required");
        let most = pair_worker_count(usize::MAX);
        let cancelled = AtomicBool::new(false);
        let workers: Vec<_> = (0..most)
            .map(|_| crate::worker_pool::checkout(Some(&path), &cancelled).unwrap())
            .collect();
        for worker in workers {
            worker.release();
        }
        assert_eq!(crate::worker_pool::idle_count(), most);
    }

    #[test]
    fn pair_failures_report_their_kind_and_keep_the_worker_cause() {
        let spawn = || WorkerError::Spawn("no such file".to_owned());
        let unavailable = ExactPairFailure::WorkerUnavailable(ExactWorkerUnavailable::Checkout {
            cause: spawn(),
        });
        assert_eq!(
            unavailable.evidence(),
            json!({"reason": "exact_worker_unavailable", "cause": spawn().to_string()})
        );
        assert_eq!(
            ExactPairFailure::WorkerUnavailable(ExactWorkerUnavailable::NotFound).evidence(),
            json!({"reason": "exact_worker_unavailable"})
        );
        assert_eq!(
            ExactPairFailure::Batch {
                cause: WorkerError::Cancelled
            }
            .evidence()["reason"],
            "exact_pair_batch_failed"
        );
        assert_eq!(
            ExactPairFailure::Incomplete {
                expected: 3,
                received: 1
            }
            .evidence(),
            json!({"reason": "exact_pair_batch_incomplete", "expected": 3, "received": 1})
        );
    }
}
