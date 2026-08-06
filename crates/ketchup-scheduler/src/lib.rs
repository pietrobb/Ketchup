#![forbid(unsafe_code)]

use ketchup_core::beam_m5::{
    BeamExactPiecePackage, BeamExactPieceRequest, BeamM5Error, BeamNotchFaceRole,
    BeamWorkerFaceEvidence, BeamWorkerResult, HalfLapParticipant, build_piece_package,
};
use ketchup_core::document::{DerivedIdentity, NodeId, SlotPath, SlotSegment};
use ketchup_core::exact_product::{
    ExactFaceRole, ExactProductError, ExactRectangleRequest, ExactRenderPackage,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::prismatic::{Aabb, JointId};
use ketchup_exact::GeometryErrorCode;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct AcceptanceIdentity {
    pub document_scope: u64,
    pub derived_identity: DerivedIdentity,
    pub input_digest: String,
    pub evaluator: String,
    pub backend: Option<String>,
    pub schema: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ScheduledIdentity {
    node_id: NodeId,
    acceptance: AcceptanceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScheduledVersion {
    revision_id: u64,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobToken {
    pub node_id: NodeId,
    pub revision_id: u64,
    pub generation: u64,
    pub acceptance: AcceptanceIdentity,
}

impl JobToken {
    #[must_use]
    pub fn input_digest(&self) -> &str {
        &self.acceptance.input_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedResult {
    pub token: JobToken,
    pub result_fingerprint: String,
    pub charge_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertOutcome {
    Current,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheStats {
    pub entry_count: usize,
    pub used_bytes: usize,
    pub budget_bytes: usize,
    pub evictions: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CacheKey {
    node_id: NodeId,
    revision_id: u64,
    generation: u64,
    acceptance: AcceptanceIdentity,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    result_fingerprint: String,
    charge_bytes: usize,
}

pub struct EvaluationScheduler {
    current_revision: u64,
    generations: BTreeMap<NodeId, u64>,
    scheduled_inputs: BTreeMap<ScheduledIdentity, ScheduledVersion>,
    cache: BTreeMap<CacheKey, CacheEntry>,
    lru: VecDeque<CacheKey>,
    cache_budget_bytes: usize,
    cache_used_bytes: usize,
    evictions: u64,
}

impl EvaluationScheduler {
    #[must_use]
    pub fn new(cache_budget_bytes: usize) -> Self {
        Self {
            current_revision: 0,
            generations: BTreeMap::new(),
            scheduled_inputs: BTreeMap::new(),
            cache: BTreeMap::new(),
            lru: VecDeque::new(),
            cache_budget_bytes,
            cache_used_bytes: 0,
            evictions: 0,
        }
    }

    #[must_use]
    pub const fn current_revision(&self) -> u64 {
        self.current_revision
    }

    pub fn advance_revision(
        &mut self,
        revision_id: u64,
        dirty_nodes: impl IntoIterator<Item = NodeId>,
    ) -> Result<(), SchedulerError> {
        if revision_id <= self.current_revision {
            return Err(SchedulerError::NonMonotonicRevision {
                current: self.current_revision,
                proposed: revision_id,
            });
        }
        self.current_revision = revision_id;
        for node_id in dirty_nodes {
            *self.generations.entry(node_id).or_default() += 1;
            self.scheduled_inputs
                .retain(|identity, _| identity.node_id != node_id);
        }
        Ok(())
    }

    pub fn schedule(
        &mut self,
        node_id: NodeId,
        input_digest: impl Into<String>,
    ) -> Result<JobToken, SchedulerError> {
        let input_digest = input_digest.into();
        if input_digest.is_empty() {
            return Err(SchedulerError::EmptyInputDigest);
        }
        let segment = SlotSegment::new(node_id, "value", "root")
            .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?;
        let identity = AcceptanceIdentity {
            document_scope: 1,
            derived_identity: DerivedIdentity::new(
                node_id,
                SlotPath::new(vec![segment])
                    .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?,
            )
            .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?,
            input_digest,
            evaluator: ketchup_core::graph::EVALUATOR_ID_V1.to_owned(),
            backend: Some(ketchup_core::graph::DEFAULT_BACKEND_ID.to_owned()),
            schema: ketchup_core::graph::GRAPH_SCHEMA_ID_V1.to_owned(),
            tolerance: ketchup_core::document::TOLERANCE_PROFILE_V1.to_owned(),
        };
        self.schedule_with_identity(node_id, identity)
    }

    pub fn schedule_with_identity(
        &mut self,
        node_id: NodeId,
        acceptance: AcceptanceIdentity,
    ) -> Result<JobToken, SchedulerError> {
        if !is_valid_acceptance_identity(node_id, &acceptance) {
            return Err(SchedulerError::InvalidAcceptanceIdentity);
        }
        let generation = *self.generations.entry(node_id).or_default();
        self.scheduled_inputs.insert(
            ScheduledIdentity {
                node_id,
                acceptance: acceptance.clone(),
            },
            ScheduledVersion {
                revision_id: self.current_revision,
                generation,
            },
        );
        Ok(JobToken {
            node_id,
            revision_id: self.current_revision,
            generation,
            acceptance,
        })
    }

    pub fn accept(&mut self, result: DerivedResult) -> InsertOutcome {
        let expected_generation = self
            .generations
            .get(&result.token.node_id)
            .copied()
            .unwrap_or_default();
        let scheduled_identity = ScheduledIdentity {
            node_id: result.token.node_id,
            acceptance: result.token.acceptance.clone(),
        };
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation: expected_generation,
        };
        if result.token.revision_id != self.current_revision
            || result.token.generation != expected_generation
            || self.scheduled_inputs.get(&scheduled_identity) != Some(&current_version)
        {
            return InsertOutcome::Stale;
        }

        let key = CacheKey {
            node_id: result.token.node_id,
            revision_id: result.token.revision_id,
            generation: result.token.generation,
            acceptance: result.token.acceptance,
        };
        self.insert_cache(
            key,
            CacheEntry {
                result_fingerprint: result.result_fingerprint,
                charge_bytes: result.charge_bytes,
            },
        );
        InsertOutcome::Current
    }

    #[must_use]
    pub fn current_result_fingerprint(&self, node_id: NodeId) -> Option<&str> {
        let generation = self.generations.get(&node_id).copied().unwrap_or_default();
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation,
        };
        let mut current_identities = self
            .scheduled_inputs
            .iter()
            .filter(|(identity, version)| {
                identity.node_id == node_id && **version == current_version
            })
            .map(|(identity, _)| &identity.acceptance);
        let acceptance = current_identities.next()?;
        if current_identities.next().is_some() {
            return None;
        }
        self.current_result_fingerprint_for(node_id, acceptance)
    }

    #[must_use]
    pub fn current_result_fingerprint_for(
        &self,
        node_id: NodeId,
        acceptance: &AcceptanceIdentity,
    ) -> Option<&str> {
        let generation = self.generations.get(&node_id).copied().unwrap_or_default();
        let scheduled_identity = ScheduledIdentity {
            node_id,
            acceptance: acceptance.clone(),
        };
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation,
        };
        if self.scheduled_inputs.get(&scheduled_identity) != Some(&current_version) {
            return None;
        }
        let key = CacheKey {
            node_id,
            revision_id: self.current_revision,
            generation,
            acceptance: acceptance.clone(),
        };
        self.cache
            .get(&key)
            .map(|entry| entry.result_fingerprint.as_str())
    }

    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.cache.len(),
            used_bytes: self.cache_used_bytes,
            budget_bytes: self.cache_budget_bytes,
            evictions: self.evictions,
        }
    }

    fn insert_cache(&mut self, key: CacheKey, entry: CacheEntry) {
        if entry.charge_bytes > self.cache_budget_bytes {
            return;
        }
        if let Some(replaced) = self.cache.remove(&key) {
            self.cache_used_bytes -= replaced.charge_bytes;
            self.lru.retain(|candidate| candidate != &key);
        }
        self.cache_used_bytes += entry.charge_bytes;
        self.lru.push_back(key.clone());
        self.cache.insert(key, entry);

        while self.cache_used_bytes > self.cache_budget_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&oldest) {
                self.cache_used_bytes -= evicted.charge_bytes;
                self.evictions += 1;
            }
        }
    }
}

fn is_valid_acceptance_identity(node_id: NodeId, acceptance: &AcceptanceIdentity) -> bool {
    let root_rule_node_id = acceptance.derived_identity.root_rule_node_id;
    let segments = acceptance.derived_identity.slot_path.segments();
    acceptance.document_scope != 0
        && root_rule_node_id == node_id
        && DerivedIdentity::new(
            root_rule_node_id,
            acceptance.derived_identity.slot_path.clone(),
        )
        .is_ok()
        && SlotPath::new(segments.to_vec()).is_ok()
        && segments.iter().all(|segment| {
            segment.producer_rule_id == root_rule_node_id
                && SlotSegment::new(
                    segment.producer_rule_id,
                    &segment.output_port,
                    &segment.semantic_key,
                )
                .is_ok()
        })
        && !acceptance.input_digest.is_empty()
        && !acceptance.evaluator.is_empty()
        && !acceptance.schema.is_empty()
        && !acceptance.tolerance.is_empty()
        && !acceptance.backend.as_ref().is_some_and(String::is_empty)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NonMonotonicRevision { current: u64, proposed: u64 },
    EmptyInputDigest,
    InvalidAcceptanceIdentity,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicRevision { current, proposed } => write!(
                formatter,
                "revision {proposed} does not advance current revision {current}"
            ),
            Self::EmptyInputDigest => formatter.write_str("scheduler input digest is empty"),
            Self::InvalidAcceptanceIdentity => {
                formatter.write_str("scheduler acceptance identity is incomplete")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerFaceEvidence {
    pub ordinal: u32,
    pub geometric_fingerprint: String,
    pub lineage_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerExactResult {
    pub backend_duration: Duration,
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub request_digest: String,
    pub exact_input_digest: String,
    pub backend: String,
    pub tolerance: String,
    pub top: WorkerFaceEvidence,
    pub bottom: WorkerFaceEvidence,
    pub east: WorkerFaceEvidence,
    pub cut_west: Option<WorkerFaceEvidence>,
    pub cut_east: Option<WorkerFaceEvidence>,
    pub cut_south: Option<WorkerFaceEvidence>,
    pub cut_north: Option<WorkerFaceEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Spawn(String),
    Transport(String),
    WorkerExited,
    Cancelled,
    RequestTimedOut(Duration),
    ResponseLineTooLarge { max_bytes: usize },
    MalformedTransport(String),
    MissingCapability(String),
    Protocol(String),
    Geometry(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "worker spawn failed: {message}"),
            Self::Transport(message) => write!(formatter, "worker transport failed: {message}"),
            Self::WorkerExited => formatter.write_str("worker exited before replying"),
            Self::Cancelled => formatter.write_str("worker operation was cancelled"),
            Self::RequestTimedOut(timeout) => write!(
                formatter,
                "worker request timed out after {} ms",
                timeout.as_millis()
            ),
            Self::ResponseLineTooLarge { max_bytes } => write!(
                formatter,
                "worker response line exceeded the {max_bytes}-byte limit"
            ),
            Self::MalformedTransport(message) => {
                write!(formatter, "worker transport was malformed: {message}")
            }
            Self::MissingCapability(capability) => {
                write!(
                    formatter,
                    "worker does not support required capability {capability}"
                )
            }
            Self::Protocol(message) => write!(formatter, "worker protocol error: {message}"),
            Self::Geometry(code) => write!(formatter, "worker geometry error: {code}"),
        }
    }
}

impl std::error::Error for WorkerError {}

impl WorkerError {
    fn permits_restart(&self) -> bool {
        matches!(
            self,
            Self::Transport(_)
                | Self::WorkerExited
                | Self::RequestTimedOut(_)
                | Self::ResponseLineTooLarge { .. }
                | Self::MalformedTransport(_)
                | Self::Protocol(_)
        )
    }
}

const M3_CAPABILITY: &str = "M3_V1";
const M3_CUT_CAPABILITY: &str = "M3_CUT_V1";
const M5_NOTCH_CAPABILITY: &str = "M5_NOTCH_V1";
const DEFAULT_WORKER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_WORKER_RESPONSE_LINE_BYTES: usize = 64 * 1024;
static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);

struct WorkerWriteRequest {
    line: String,
    acknowledgment: Sender<Result<(), String>>,
}

enum WorkerResponse {
    Line(String),
    Exited,
    TooLarge,
    Malformed(String),
    Transport(String),
}

pub struct ExactWorkerClient {
    child: Child,
    write_sender: Sender<WorkerWriteRequest>,
    response_receiver: Receiver<WorkerResponse>,
}

impl ExactWorkerClient {
    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        let mut child = Command::new(executable.as_ref())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdout was not piped".to_owned()))?;
        let (write_sender, write_receiver) = mpsc::channel();
        let (response_sender, response_receiver) = mpsc::channel();
        spawn_worker_writer(stdin, write_receiver);
        spawn_worker_reader(stdout, response_sender);
        Ok(Self {
            child,
            write_sender,
            response_receiver,
        })
    }

    pub fn ping(&mut self) -> Result<(), WorkerError> {
        self.ping_with_cancellation(&NEVER_CANCELLED)
    }

    fn ping_with_cancellation(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("PING", cancelled)?;
        if response == "PONG" {
            Ok(())
        } else {
            self.fail_protocol(response)
        }
    }

    fn verify_m3_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M3_V1", cancelled)?;
        if response == "CAPS M3_V1" {
            Ok(())
        } else if response.split_whitespace().next() == Some("ERR") {
            let fields = response.split_whitespace().collect::<Vec<_>>();
            self.fail(parse_error_response(&response, &fields))
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(M3_CAPABILITY.to_owned()))
        }
    }

    fn verify_m3_cut_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M3_CUT_V1", cancelled)?;
        if response == "CAPS M3_CUT_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(M3_CUT_CAPABILITY.to_owned()))
        }
    }

    fn verify_m5_notch_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M5_NOTCH_V1", cancelled)?;
        if response == "CAPS M5_NOTCH_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M5_NOTCH_CAPABILITY.to_owned(),
            ))
        }
    }

    pub fn extrude_rectangle(&mut self, height_mm: f64) -> Result<WorkerExactResult, WorkerError> {
        let response = self.request(&format!("EXTRUDE {:016x}", height_mm.to_bits()))?;
        match parse_legacy_exact_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    pub fn extrude_rectangle_request(
        &mut self,
        request: &ExactRectangleRequest,
    ) -> Result<WorkerExactResult, WorkerError> {
        self.extrude_rectangle_request_with_cancellation(request, &NEVER_CANCELLED)
    }

    fn extrude_rectangle_request_with_cancellation(
        &mut self,
        request: &ExactRectangleRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactResult, WorkerError> {
        let (response, is_through_cut) = if let Some(cut) = &request.through_cut {
            self.verify_m3_cut_capability(cancelled)?;
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_CUT_M3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        cut.min_x_bits,
                        cut.min_y_bits,
                        cut.width_bits,
                        cut.depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                true,
            )
        } else {
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_M3_V1 {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.extrusion_feature_id.0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                false,
            )
        };
        let parsed = if is_through_cut {
            parse_m3_cut_exact_result(&response)
        } else {
            parse_m3_exact_result(&response)
        };
        match parsed {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn evaluate_beam_piece_request_with_cancellation(
        &mut self,
        request: &BeamExactPieceRequest,
        cancelled: &AtomicBool,
    ) -> Result<BeamWorkerResult, WorkerError> {
        self.verify_m5_notch_capability(cancelled)?;
        let stock = request.stock;
        let mut line = format!(
            "EVAL_NOTCHED_M5_V1 {} {} {}",
            request.document_id.0, request.piece_key, request.canonical_input_digest
        );
        push_aabb_request(&mut line, stock);
        line.push_str(&format!(" {}", request.notches.len()));
        for notch in &request.notches {
            line.push_str(&format!(
                " {} {} {}",
                notch.joint_id.0,
                notch.participant.token(),
                notch.feature_ordinal
            ));
            push_aabb_request(&mut line, notch.removed);
        }
        let response = self.request_with_cancellation(&line, cancelled)?;
        match parse_m5_exact_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    pub fn exception_probe(&mut self) -> Result<String, WorkerError> {
        let response = self.request("EXCEPTION")?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        match fields.as_slice() {
            ["ERR", code] if is_geometry_error_code(code) => Ok((*code).to_owned()),
            _ => self.fail_protocol(response),
        }
    }

    pub fn begin_killable_job(&mut self, duration: Duration) -> Result<(), WorkerError> {
        let deadline = Instant::now() + DEFAULT_WORKER_REQUEST_TIMEOUT;
        self.write_request_until(&format!("SLEEP {}", duration.as_millis()), deadline)
    }

    pub fn crash(&mut self) -> Result<(), WorkerError> {
        let deadline = Instant::now() + DEFAULT_WORKER_REQUEST_TIMEOUT;
        self.write_request_until("CRASH", deadline)?;
        match self.next_response_until(deadline)? {
            WorkerResponse::Exited => self
                .child
                .wait()
                .map(|_| ())
                .map_err(|error| WorkerError::Transport(error.to_string())),
            WorkerResponse::Line(response) => self.fail_protocol(response),
            WorkerResponse::TooLarge => self.fail(WorkerError::ResponseLineTooLarge {
                max_bytes: MAX_WORKER_RESPONSE_LINE_BYTES,
            }),
            WorkerResponse::Malformed(message) => {
                self.fail(WorkerError::MalformedTransport(message))
            }
            WorkerResponse::Transport(message) => self.fail(WorkerError::Transport(message)),
        }
    }

    pub fn cancel(mut self) -> Result<Duration, WorkerError> {
        let started = Instant::now();
        self.child
            .kill()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        self.child
            .wait()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        Ok(started.elapsed())
    }

    fn request(&mut self, request: &str) -> Result<String, WorkerError> {
        self.request_with_cancellation(request, &NEVER_CANCELLED)
    }

    fn request_with_cancellation(
        &mut self,
        request: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, WorkerError> {
        let deadline = Instant::now() + DEFAULT_WORKER_REQUEST_TIMEOUT;
        self.write_request_until_with_cancellation(request, deadline, cancelled)?;
        match self.next_response_until_with_cancellation(deadline, cancelled)? {
            WorkerResponse::Line(response) => Ok(response),
            WorkerResponse::Exited => self.fail(WorkerError::WorkerExited),
            WorkerResponse::TooLarge => self.fail(WorkerError::ResponseLineTooLarge {
                max_bytes: MAX_WORKER_RESPONSE_LINE_BYTES,
            }),
            WorkerResponse::Malformed(message) => {
                self.fail(WorkerError::MalformedTransport(message))
            }
            WorkerResponse::Transport(message) => self.fail(WorkerError::Transport(message)),
        }
    }

    fn write_request_until(&mut self, request: &str, deadline: Instant) -> Result<(), WorkerError> {
        self.write_request_until_with_cancellation(request, deadline, &NEVER_CANCELLED)
    }

    fn write_request_until_with_cancellation(
        &mut self,
        request: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.ensure_not_cancelled(cancelled)?;
        let (acknowledgment, receiver) = mpsc::channel();
        if self
            .write_sender
            .send(WorkerWriteRequest {
                line: request.to_owned(),
                acknowledgment,
            })
            .is_err()
        {
            self.ensure_not_cancelled(cancelled)?;
            return self.fail(WorkerError::MalformedTransport(
                "worker request writer disconnected".to_owned(),
            ));
        }
        loop {
            self.ensure_not_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(WorkerError::RequestTimedOut(DEFAULT_WORKER_REQUEST_TIMEOUT));
            }
            match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
                Ok(result) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return match result {
                        Ok(()) => Ok(()),
                        Err(message) => self.fail(WorkerError::Transport(message)),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return self.fail(WorkerError::MalformedTransport(
                        "worker request writer disconnected before acknowledging the write"
                            .to_owned(),
                    ));
                }
            }
        }
    }

    fn next_response_until(&mut self, deadline: Instant) -> Result<WorkerResponse, WorkerError> {
        self.next_response_until_with_cancellation(deadline, &NEVER_CANCELLED)
    }

    fn next_response_until_with_cancellation(
        &mut self,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<WorkerResponse, WorkerError> {
        loop {
            self.ensure_not_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(WorkerError::RequestTimedOut(DEFAULT_WORKER_REQUEST_TIMEOUT));
            }
            match self
                .response_receiver
                .recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL))
            {
                Ok(response) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return Ok(response);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return self.fail(WorkerError::MalformedTransport(
                        "worker response reader disconnected without a terminal event".to_owned(),
                    ));
                }
            }
        }
    }

    fn ensure_not_cancelled(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        if cancelled.load(Ordering::Acquire) {
            self.fail(WorkerError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn fail_protocol<T>(&mut self, response: String) -> Result<T, WorkerError> {
        self.fail(WorkerError::Protocol(response))
    }

    fn fail<T>(&mut self, error: WorkerError) -> Result<T, WorkerError> {
        self.terminate_worker();
        Err(error)
    }

    fn terminate_worker(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_worker_writer(mut stdin: ChildStdin, receiver: Receiver<WorkerWriteRequest>) {
    let _ = std::thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let result = writeln!(stdin, "{}", request.line)
                .and_then(|()| stdin.flush())
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            let _ = request.acknowledgment.send(result);
            if failed {
                break;
            }
        }
    });
}

fn spawn_worker_reader(stdout: ChildStdout, sender: Sender<WorkerResponse>) {
    let _ = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let response = read_worker_response(&mut reader);
            let terminal = !matches!(response, WorkerResponse::Line(_));
            if sender.send(response).is_err() || terminal {
                break;
            }
        }
    });
}

fn read_worker_response(reader: &mut impl BufRead) -> WorkerResponse {
    let mut bytes = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) => return WorkerResponse::Transport(error.to_string()),
        };
        if available.is_empty() {
            return if bytes.is_empty() {
                WorkerResponse::Exited
            } else {
                WorkerResponse::Malformed("worker response ended without a newline".to_owned())
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(consumed) > MAX_WORKER_RESPONSE_LINE_BYTES {
            return WorkerResponse::TooLarge;
        }
        bytes.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return match String::from_utf8(bytes) {
                Ok(line) => WorkerResponse::Line(line),
                Err(error) => WorkerResponse::Malformed(format!(
                    "worker response was not valid UTF-8: {error}"
                )),
            };
        }
    }
}

impl Drop for ExactWorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct ExactWorkerSupervisor {
    executable: PathBuf,
    client: ExactWorkerClient,
}

impl ExactWorkerSupervisor {
    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        Self::spawn_with_cancellation(executable, &NEVER_CANCELLED)
    }

    pub fn spawn_with_cancellation(
        executable: impl AsRef<Path>,
        cancelled: &AtomicBool,
    ) -> Result<Self, WorkerError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        let executable = executable.as_ref().to_owned();
        let client = Self::spawn_verified_client(&executable, cancelled)?;
        Ok(Self { executable, client })
    }

    fn spawn_verified_client(
        executable: &Path,
        cancelled: &AtomicBool,
    ) -> Result<ExactWorkerClient, WorkerError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        let mut client = ExactWorkerClient::spawn(executable)?;
        client.ensure_not_cancelled(cancelled)?;
        client.ping_with_cancellation(cancelled)?;
        client.verify_m3_capability(cancelled)?;
        Ok(client)
    }

    pub fn evaluate_rectangle(
        &mut self,
        request: &ExactRectangleRequest,
    ) -> Result<ExactRenderPackage, M3EvaluationError> {
        self.evaluate_rectangle_with_cancellation(request, &NEVER_CANCELLED)
    }

    pub fn evaluate_rectangle_with_cancellation(
        &mut self,
        request: &ExactRectangleRequest,
        cancelled: &AtomicBool,
    ) -> Result<ExactRenderPackage, M3EvaluationError> {
        self.client.ensure_not_cancelled(cancelled)?;
        let result = match self
            .client
            .extrude_rectangle_request_with_cancellation(request, cancelled)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, cancelled)?;
                self.client
                    .extrude_rectangle_request_with_cancellation(request, cancelled)?
            }
            Err(error) => {
                self.client.ensure_not_cancelled(cancelled)?;
                return Err(error.into());
            }
        };
        self.client.ensure_not_cancelled(cancelled)?;
        validate_m3_worker_result(request, &result)?;
        let package = build_m3_render_package(request, &result)?;
        self.client.ensure_not_cancelled(cancelled)?;
        Ok(package)
    }

    pub fn evaluate_beam_piece(
        &mut self,
        request: &BeamExactPieceRequest,
    ) -> Result<BeamExactPiecePackage, M5EvaluationError> {
        self.evaluate_beam_piece_with_cancellation(request, &NEVER_CANCELLED)
    }

    pub fn evaluate_beam_piece_with_cancellation(
        &mut self,
        request: &BeamExactPieceRequest,
        cancelled: &AtomicBool,
    ) -> Result<BeamExactPiecePackage, M5EvaluationError> {
        if !is_sha256_digest(&request.piece_key)
            || !is_sha256_digest(&request.canonical_input_digest)
        {
            return Err(BeamM5Error::InvalidWorkerEvidence.into());
        }
        self.client.ensure_not_cancelled(cancelled)?;
        let result = match self
            .client
            .evaluate_beam_piece_request_with_cancellation(request, cancelled)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, cancelled)?;
                self.client
                    .evaluate_beam_piece_request_with_cancellation(request, cancelled)?
            }
            Err(error) => return Err(error.into()),
        };
        self.client.ensure_not_cancelled(cancelled)?;
        let package = build_piece_package(request, result)?;
        self.client.ensure_not_cancelled(cancelled)?;
        Ok(package)
    }
}

fn validate_m3_worker_result(
    request: &ExactRectangleRequest,
    result: &WorkerExactResult,
) -> Result<(), ExactProductError> {
    let mut role_evidence = vec![
        (ExactFaceRole::Top, &result.top),
        (ExactFaceRole::Bottom, &result.bottom),
        (ExactFaceRole::East, &result.east),
    ];
    match (
        &request.through_cut,
        &result.cut_west,
        &result.cut_east,
        &result.cut_south,
        &result.cut_north,
    ) {
        (Some(_), Some(west), Some(east), Some(south), Some(north)) => {
            role_evidence.extend([
                (ExactFaceRole::CutWest, west),
                (ExactFaceRole::CutEast, east),
                (ExactFaceRole::CutSouth, south),
                (ExactFaceRole::CutNorth, north),
            ]);
        }
        (None, None, None, None, None) => {}
        _ => return Err(ExactProductError::InvalidWorkerEvidence),
    }

    let dimensions = request.dimensions_mm();
    let expected_bounds = [0.0, 0.0, 0.0, dimensions[0], dimensions[1], dimensions[2]];
    let (expected_volume, expected_topology) = request.through_cut.as_ref().map_or_else(
        || (dimensions.into_iter().product::<f64>(), [8, 12, 6, 1, 1]),
        |cut| {
            let cut_volume =
                f64::from_bits(cut.width_bits) * f64::from_bits(cut.depth_bits) * dimensions[2];
            (
                dimensions.into_iter().product::<f64>() - cut_volume,
                [16, 24, 10, 1, 1],
            )
        },
    );
    let volume_tolerance = 1.0e-6_f64.max(expected_volume.abs() * 1.0e-10);
    let producer_feature_id = request.producer_feature_id();
    let has_canonical_lineage = |role: ExactFaceRole, evidence: &WorkerFaceEvidence| {
        !evidence.geometric_fingerprint.is_empty()
            && evidence.lineage_digest
                == canonical_reference_lineage_digest(
                    request.document_id,
                    producer_feature_id,
                    role.semantic_role(),
                    role.source_element_id(),
                    "planar_face",
                )
    };
    let ordinals_are_distinct_and_in_range =
        role_evidence
            .iter()
            .enumerate()
            .all(|(index, (_, evidence))| {
                evidence.ordinal < result.topology_counts[2]
                    && role_evidence[..index]
                        .iter()
                        .all(|(_, prior)| prior.ordinal != evidence.ordinal)
            });

    if result.request_digest != request.canonical_input_digest
        || !is_sha256_digest(&result.request_digest)
        || !is_fnv1a64_digest(&result.exact_input_digest)
        || !is_fnv1a64_digest(&result.result_fingerprint)
        || result.backend != ketchup_exact::backend_fingerprint()
        || result.tolerance != ketchup_exact::tolerance_profile()
        || !role_evidence
            .iter()
            .all(|(role, evidence)| has_canonical_lineage(*role, evidence))
        || !ordinals_are_distinct_and_in_range
        || !result.volume_mm3.is_finite()
        || result.volume_mm3 <= 0.0
        || (result.volume_mm3 - expected_volume).abs() > volume_tolerance
        || result
            .bounds_mm
            .into_iter()
            .zip(expected_bounds)
            .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || result.topology_counts != expected_topology
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn build_m3_render_package(
    request: &ExactRectangleRequest,
    result: &WorkerExactResult,
) -> Result<ExactRenderPackage, ExactProductError> {
    let bounds = [
        [
            result.bounds_mm[0],
            result.bounds_mm[1],
            result.bounds_mm[2],
        ],
        [
            result.bounds_mm[3],
            result.bounds_mm[4],
            result.bounds_mm[5],
        ],
    ];
    let evidence = |role: ExactFaceRole, value: &WorkerFaceEvidence| {
        (
            role,
            value.lineage_digest.clone(),
            value.geometric_fingerprint.clone(),
        )
    };
    if request.through_cut.is_some() {
        let (Some(cut_west), Some(cut_east), Some(cut_south), Some(cut_north)) = (
            &result.cut_west,
            &result.cut_east,
            &result.cut_south,
            &result.cut_north,
        ) else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        build_box_render_package(
            request,
            result.exact_input_digest.clone(),
            result.result_fingerprint.clone(),
            result.backend.clone(),
            result.tolerance.clone(),
            bounds,
            [
                evidence(ExactFaceRole::Top, &result.top),
                evidence(ExactFaceRole::Bottom, &result.bottom),
                evidence(ExactFaceRole::East, &result.east),
                evidence(ExactFaceRole::CutWest, cut_west),
                evidence(ExactFaceRole::CutEast, cut_east),
                evidence(ExactFaceRole::CutSouth, cut_south),
                evidence(ExactFaceRole::CutNorth, cut_north),
            ],
        )
    } else {
        build_box_render_package(
            request,
            result.exact_input_digest.clone(),
            result.result_fingerprint.clone(),
            result.backend.clone(),
            result.tolerance.clone(),
            bounds,
            [
                evidence(ExactFaceRole::Top, &result.top),
                evidence(ExactFaceRole::Bottom, &result.bottom),
                evidence(ExactFaceRole::East, &result.east),
            ],
        )
    }
}

#[derive(Debug)]
pub enum M5EvaluationError {
    Worker(WorkerError),
    Product(BeamM5Error),
}

impl fmt::Display for M5EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker(error) => error.fmt(formatter),
            Self::Product(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for M5EvaluationError {}

impl From<WorkerError> for M5EvaluationError {
    fn from(error: WorkerError) -> Self {
        Self::Worker(error)
    }
}

impl From<BeamM5Error> for M5EvaluationError {
    fn from(error: BeamM5Error) -> Self {
        Self::Product(error)
    }
}

#[derive(Debug)]
pub enum M3EvaluationError {
    Worker(WorkerError),
    Product(ExactProductError),
}

impl fmt::Display for M3EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker(error) => error.fmt(formatter),
            Self::Product(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for M3EvaluationError {}

impl From<WorkerError> for M3EvaluationError {
    fn from(error: WorkerError) -> Self {
        Self::Worker(error)
    }
}

impl From<ExactProductError> for M3EvaluationError {
    fn from(error: ExactProductError) -> Self {
        Self::Product(error)
    }
}

fn is_geometry_error_code(code: &str) -> bool {
    [
        GeometryErrorCode::InvalidParameter,
        GeometryErrorCode::InvalidProfile,
        GeometryErrorCode::NonFiniteParameter,
        GeometryErrorCode::NoGeometricChange,
        GeometryErrorCode::DegenerateOperation,
        GeometryErrorCode::InvalidShape,
        GeometryErrorCode::BackendException,
        GeometryErrorCode::NullResult,
    ]
    .into_iter()
    .any(|candidate| candidate.as_str() == code)
}

fn push_aabb_request(line: &mut String, bounds: Aabb) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        line.push_str(&format!(" {:016x}", value.to_bits()));
    }
}

fn parse_m5_exact_result(response: &str) -> Result<BeamWorkerResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() < 19 || fields[0] != "OK_M5_V1" {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_f64 = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map(f64::from_bits)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let reference_count = fields[18]
        .parse::<usize>()
        .map_err(|_| WorkerError::Protocol(response.to_owned()))?;
    let expected_len = reference_count
        .checked_mul(6)
        .and_then(|count| count.checked_add(19))
        .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?;
    if fields.len() != expected_len {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let bounds_mm = Aabb::bounded_volume(
        [parse_f64(3)?, parse_f64(4)?, parse_f64(5)?],
        [parse_f64(6)?, parse_f64(7)?, parse_f64(8)?],
    )
    .map_err(|_| WorkerError::Protocol(response.to_owned()))?;
    let mut face_evidence = Vec::with_capacity(reference_count);
    for index in 0..reference_count {
        let offset = 19 + index * 6;
        let participant = match fields[offset + 1] {
            "a" => HalfLapParticipant::A,
            "b" => HalfLapParticipant::B,
            _ => return Err(WorkerError::Protocol(response.to_owned())),
        };
        let role = match fields[offset + 2] {
            "contact" => BeamNotchFaceRole::Contact,
            "wall.west" => BeamNotchFaceRole::WestWall,
            "wall.east" => BeamNotchFaceRole::EastWall,
            _ => return Err(WorkerError::Protocol(response.to_owned())),
        };
        face_evidence.push(BeamWorkerFaceEvidence {
            joint_id: JointId(parse_u64(offset)?),
            participant,
            role,
            face_ordinal: parse_u32(offset + 3)?,
            geometric_fingerprint: fields[offset + 4].to_owned(),
            lineage_digest: fields[offset + 5].to_owned(),
        });
    }
    Ok(BeamWorkerResult {
        result_fingerprint: fields[1].to_owned(),
        volume_mm3: parse_f64(2)?,
        bounds_mm,
        topology_counts: [
            parse_u32(9)?,
            parse_u32(10)?,
            parse_u32(11)?,
            parse_u32(12)?,
            parse_u32(13)?,
        ],
        request_digest: fields[14].to_owned(),
        exact_input_digest: fields[15].to_owned(),
        backend: fields[16].to_owned(),
        tolerance: fields[17].to_owned(),
        face_evidence,
    })
}

fn parse_error_response(response: &str, fields: &[&str]) -> WorkerError {
    match fields {
        ["ERR", code] if is_geometry_error_code(code) => WorkerError::Geometry((*code).to_owned()),
        _ => WorkerError::Protocol(response.to_owned()),
    }
}

fn parse_legacy_exact_result(response: &str) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 15 || fields[0] != "OK" {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_f64 = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map(f64::from_bits)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: parse_f64(3)?,
        bounds_mm: [
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
            parse_f64(9)?,
        ],
        topology_counts: [
            parse_u32(10)?,
            parse_u32(11)?,
            parse_u32(12)?,
            parse_u32(13)?,
            parse_u32(14)?,
        ],
        request_digest: String::new(),
        exact_input_digest: String::new(),
        backend: String::new(),
        tolerance: String::new(),
        top: WorkerFaceEvidence {
            ordinal: 0,
            geometric_fingerprint: String::new(),
            lineage_digest: String::new(),
        },
        bottom: WorkerFaceEvidence {
            ordinal: 0,
            geometric_fingerprint: String::new(),
            lineage_digest: String::new(),
        },
        east: WorkerFaceEvidence {
            ordinal: 0,
            geometric_fingerprint: String::new(),
            lineage_digest: String::new(),
        },
        cut_west: None,
        cut_east: None,
        cut_south: None,
        cut_north: None,
    })
}

fn parse_m3_exact_result(response: &str) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 28
        || fields[0] != "OK_M3_V1"
        || fields[17].is_empty()
        || fields[18].is_empty()
        || !is_sha256_digest(fields[15])
        || [2, 16, 20, 21, 23, 24, 26, 27]
            .into_iter()
            .any(|index| !is_fnv1a64_digest(fields[index]))
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_f64 = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map(f64::from_bits)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: parse_f64(3)?,
        bounds_mm: [
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
            parse_f64(9)?,
        ],
        topology_counts: [
            parse_u32(10)?,
            parse_u32(11)?,
            parse_u32(12)?,
            parse_u32(13)?,
            parse_u32(14)?,
        ],
        request_digest: fields[15].to_owned(),
        exact_input_digest: fields[16].to_owned(),
        backend: fields[17].to_owned(),
        tolerance: fields[18].to_owned(),
        top: WorkerFaceEvidence {
            ordinal: parse_u32(19)?,
            geometric_fingerprint: fields[20].to_owned(),
            lineage_digest: fields[21].to_owned(),
        },
        bottom: WorkerFaceEvidence {
            ordinal: parse_u32(22)?,
            geometric_fingerprint: fields[23].to_owned(),
            lineage_digest: fields[24].to_owned(),
        },
        east: WorkerFaceEvidence {
            ordinal: parse_u32(25)?,
            geometric_fingerprint: fields[26].to_owned(),
            lineage_digest: fields[27].to_owned(),
        },
        cut_west: None,
        cut_east: None,
        cut_south: None,
        cut_north: None,
    })
}

fn parse_m3_cut_exact_result(response: &str) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 40
        || fields[0] != "OK_M3_CUT_V1"
        || fields[17].is_empty()
        || fields[18].is_empty()
        || !is_sha256_digest(fields[15])
        || [
            2, 16, 20, 21, 23, 24, 26, 27, 29, 30, 32, 33, 35, 36, 38, 39,
        ]
        .into_iter()
        .any(|index| !is_fnv1a64_digest(fields[index]))
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_f64 = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map(f64::from_bits)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_evidence = |ordinal_index: usize| {
        Ok(WorkerFaceEvidence {
            ordinal: parse_u32(ordinal_index)?,
            geometric_fingerprint: fields[ordinal_index + 1].to_owned(),
            lineage_digest: fields[ordinal_index + 2].to_owned(),
        })
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: parse_f64(3)?,
        bounds_mm: [
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
            parse_f64(9)?,
        ],
        topology_counts: [
            parse_u32(10)?,
            parse_u32(11)?,
            parse_u32(12)?,
            parse_u32(13)?,
            parse_u32(14)?,
        ],
        request_digest: fields[15].to_owned(),
        exact_input_digest: fields[16].to_owned(),
        backend: fields[17].to_owned(),
        tolerance: fields[18].to_owned(),
        top: parse_evidence(19)?,
        bottom: parse_evidence(22)?,
        east: parse_evidence(25)?,
        cut_west: Some(parse_evidence(28)?),
        cut_east: Some(parse_evidence(31)?),
        cut_south: Some(parse_evidence(34)?),
        cut_north: Some(parse_evidence(37)?),
    })
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_fnv1a64_digest(value: &str) -> bool {
    value.len() == 24
        && value.starts_with("fnv1a64:")
        && value[8..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
