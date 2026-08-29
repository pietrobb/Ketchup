#![forbid(unsafe_code)]

pub mod assistant;
pub mod general;
pub mod plugin;
pub mod validator_runtime;

use ketchup_core::beam_m5::{
    BeamExactPiecePackage, BeamExactPieceRequest, BeamM5Error, BeamNotchFaceRole,
    BeamWorkerFaceEvidence, BeamWorkerResult, HalfLapParticipant, build_piece_package,
};
use ketchup_core::bottle_m6::{
    ExactRevolvePackage, ExactRevolveRequest, build_revolve_package, expected_volume_mm3,
};
use ketchup_core::document::{
    BooleanOperation, BottleEdgeFinishKind, DerivedIdentity, NodeId, SlotPath, SlotSegment,
    Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest, ExactLoftPackage, ExactLoftRequest, ExactPlanarOffsetPackage,
    ExactPlanarOffsetRequest, ExactProductError, ExactProfileSegment, ExactRenderPackage,
    ExactSweepPackage, ExactSweepRequest, LoftWorkerEvidence, PlanarOffsetWorkerEvidence,
    SweepWorkerEvidence, SweepWorkerFaceEvidence, build_box_render_package, build_loft_package,
    build_planar_offset_package, build_sweep_package, canonical_reference_lineage_digest,
};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{
    ImportLengthUnit, MAX_STEP_SOURCE_BYTES, StepImportEvidence, StepImportMesh,
};
use ketchup_core::prismatic::{Aabb, JointId};
use ketchup_exact::GeometryErrorCode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fmt::Write as _;
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
    pub pocket_floor: Option<WorkerFaceEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerPlanarOffsetResult {
    pub backend_duration: Duration,
    pub result_fingerprint: String,
    pub area_mm2: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub request_digest: String,
    pub exact_input_digest: String,
    pub backend: String,
    pub tolerance: String,
    pub face: WorkerFaceEvidence,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerSweepResult {
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub request_digest: String,
    pub exact_input_digest: String,
    pub backend: String,
    pub tolerance: String,
    pub faces: Vec<WorkerFaceEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerLoftResult {
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub request_digest: String,
    pub exact_input_digest: String,
    pub backend: String,
    pub tolerance: String,
    pub faces: Vec<WorkerFaceEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerRevolveResult {
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub request_digest: String,
    pub exact_input_digest: String,
    pub backend: String,
    pub tolerance: String,
    pub faces: Vec<WorkerFaceEvidence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerExactBRepGraphResult {
    pub canonical_input_digest: String,
    pub graph_digest: String,
    pub producer_feature_id: u64,
    pub result_fingerprint: String,
    pub exact_input_digest: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub backend: String,
    pub tolerance: String,
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
const M3_POCKET_CAPABILITY: &str = "M3_POCKET_V1";
const M3_UNION_CAPABILITY: &str = "M3_UNION_V1";
const P6_INTERSECT_CAPABILITY: &str = "P6_INTERSECT_V1";
const P6_SPLIT_CAPABILITY: &str = "P6_SPLIT_V1";
const P6_OFFSET_CAPABILITY: &str = "P6_OFFSET_V1";
const P6_SWEEP_CAPABILITY: &str = "P6_SWEEP_V1";
const P6_LOFT_CAPABILITY: &str = "P6_LOFT_V1";
const P3_CIRCLE_CAPABILITY: &str = "P3_CIRCLE_V1";
const P3_ARC_CAPABILITY: &str = "P3_ARC_V1";
const P3_POLYGON_CUT_CAPABILITY: &str = "P3_POLYGON_CUT_V1";
const M5_NOTCH_CAPABILITY: &str = "M5_NOTCH_V1";
const M6_REVOLVE_CAPABILITY: &str = "M6_REVOLVE_V1";
const M6_SHELL_CAPABILITY: &str = "M6_SHELL_V1";
const M14_STEP_CAPABILITY: &str = "M14_STEP_V1";
const M21_STEP_MODEL_CAPABILITY: &str = "M21_STEP_MODEL_V1";
const EXACT_BREP_GRAPH_CAPABILITY: &str = "EXACT_BREP_GRAPH_V1";
const DEFAULT_WORKER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepFeatureExportSpec {
    pub width_bits: u64,
    pub depth_bits: u64,
    pub height_bits: u64,
    pub circle: Option<StepCircleSpec>,
    pub mixed_segments: Vec<StepProfileSegment>,
    pub pocket_depth_bits: Option<u64>,
    pub boolean: Option<StepBooleanSpec>,
    pub shell: Option<StepShellSpec>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StepCircleSpec {
    pub center_x_bits: u64,
    pub center_y_bits: u64,
    pub radius_bits: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum StepProfileSegment {
    Line {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
    },
    Arc {
        start_bits: [u64; 2],
        end_bits: [u64; 2],
        center_bits: [u64; 2],
        clockwise: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepRevolveExportSpec {
    pub segments: Vec<StepProfileSegment>,
    pub axis_start_bits: [u64; 2],
    pub axis_end_bits: [u64; 2],
    pub angle_degrees_bits: u64,
}

impl From<&ExactRevolveRequest> for StepRevolveExportSpec {
    fn from(request: &ExactRevolveRequest) -> Self {
        Self {
            segments: request
                .profile_segments()
                .into_iter()
                .map(|segment| match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => StepProfileSegment::Line {
                        start_bits,
                        end_bits,
                    },
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => StepProfileSegment::Arc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    },
                })
                .collect(),
            axis_start_bits: request.axis_start_bits,
            axis_end_bits: request.axis_end_bits,
            angle_degrees_bits: request.angle_degrees_bits,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepAssemblyManifest {
    pub document_id: u64,
    pub source_revision: u64,
    pub source_digest: String,
    pub parts: Vec<StepAssemblyPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepAssemblyPart {
    pub document_id: u64,
    pub source_revision: u64,
    pub source_digest: String,
    pub expected_result_fingerprint: String,
    pub imported_result_fingerprint: String,
    pub source_sha256: String,
    pub transform_bits: [u64; 16],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepBooleanSpec {
    pub operation: String,
    pub min_x_bits: u64,
    pub min_y_bits: u64,
    pub width_bits: u64,
    pub depth_bits: u64,
    pub circle: Option<StepCircleSpec>,
    pub mixed_segments: Vec<StepProfileSegment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepShellSpec {
    pub thickness_bits: u64,
    pub finish: Option<String>,
    pub amount_bits: Option<u64>,
}

impl From<&ExactFeatureChainRequest> for StepFeatureExportSpec {
    fn from(request: &ExactFeatureChainRequest) -> Self {
        Self {
            width_bits: request.width_bits,
            depth_bits: request.depth_bits,
            height_bits: request.height_bits,
            circle: request.circle.map(|circle| StepCircleSpec {
                center_x_bits: circle.center_x_bits,
                center_y_bits: circle.center_y_bits,
                radius_bits: circle.radius_bits,
            }),
            mixed_segments: request
                .mixed_profile
                .as_ref()
                .map(|profile| {
                    profile
                        .segments
                        .iter()
                        .map(|segment| match segment {
                            ExactProfileSegment::Line {
                                start_bits,
                                end_bits,
                            } => StepProfileSegment::Line {
                                start_bits: *start_bits,
                                end_bits: *end_bits,
                            },
                            ExactProfileSegment::CircularArc {
                                start_bits,
                                end_bits,
                                center_bits,
                                clockwise,
                            } => StepProfileSegment::Arc {
                                start_bits: *start_bits,
                                end_bits: *end_bits,
                                center_bits: *center_bits,
                                clockwise: *clockwise,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default(),
            pocket_depth_bits: request.pocket_depth_bits,
            boolean: request.boolean.as_ref().map(|boolean| StepBooleanSpec {
                operation: match boolean.operation {
                    BooleanOperation::Cut => "cut",
                    BooleanOperation::Union => "union",
                    BooleanOperation::Intersect => "intersect",
                    BooleanOperation::Split => "split",
                }
                .to_owned(),
                min_x_bits: boolean.min_x_bits,
                min_y_bits: boolean.min_y_bits,
                width_bits: boolean.width_bits,
                depth_bits: boolean.depth_bits,
                circle: boolean.circle.map(|circle| StepCircleSpec {
                    center_x_bits: circle.center_x_bits,
                    center_y_bits: circle.center_y_bits,
                    radius_bits: circle.radius_bits,
                }),
                mixed_segments: boolean
                    .profile
                    .as_ref()
                    .filter(|profile| {
                        profile.is_line_arc_d_profile()
                            || profile.is_line_arc_capsule_profile()
                            || profile.is_line_arc_rounded_rectangle_profile()
                            || matches!(
                                boolean.operation,
                                BooleanOperation::Cut
                                    | BooleanOperation::Union
                                    | BooleanOperation::Intersect
                                    | BooleanOperation::Split
                            ) && profile.is_strict_convex_line_arc_profile()
                    })
                    .map(|profile| {
                        profile
                            .segments
                            .iter()
                            .map(|segment| match segment {
                                ExactProfileSegment::Line {
                                    start_bits,
                                    end_bits,
                                } => StepProfileSegment::Line {
                                    start_bits: *start_bits,
                                    end_bits: *end_bits,
                                },
                                ExactProfileSegment::CircularArc {
                                    start_bits,
                                    end_bits,
                                    center_bits,
                                    clockwise,
                                } => StepProfileSegment::Arc {
                                    start_bits: *start_bits,
                                    end_bits: *end_bits,
                                    center_bits: *center_bits,
                                    clockwise: *clockwise,
                                },
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            shell: request.shell.as_ref().map(|shell| StepShellSpec {
                thickness_bits: shell.thickness_bits,
                finish: shell.edge_finish_kind.map(|kind| match kind {
                    BottleEdgeFinishKind::Fillet => "fillet".to_owned(),
                    BottleEdgeFinishKind::Chamfer => "chamfer".to_owned(),
                }),
                amount_bits: shell.edge_finish_amount_bits,
            }),
        }
    }
}
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

    fn verify_m3_pocket_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M3_POCKET_V1", cancelled)?;
        if response == "CAPS M3_POCKET_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M3_POCKET_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p3_circle_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P3_CIRCLE_V1", cancelled)?;
        if response == "CAPS P3_CIRCLE_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P3_CIRCLE_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p3_arc_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P3_ARC_V1", cancelled)?;
        if response == "CAPS P3_ARC_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(P3_ARC_CAPABILITY.to_owned()))
        }
    }

    fn verify_p3_polygon_cut_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P3_POLYGON_CUT_V1", cancelled)?;
        if response == "CAPS P3_POLYGON_CUT_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P3_POLYGON_CUT_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_m3_union_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M3_UNION_V1", cancelled)?;
        if response == "CAPS M3_UNION_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M3_UNION_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p6_intersect_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P6_INTERSECT_V1", cancelled)?;
        if response == "CAPS P6_INTERSECT_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P6_INTERSECT_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p6_split_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P6_SPLIT_V1", cancelled)?;
        if response == "CAPS P6_SPLIT_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P6_SPLIT_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p6_offset_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P6_OFFSET_V1", cancelled)?;
        if response == "CAPS P6_OFFSET_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P6_OFFSET_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p6_sweep_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P6_SWEEP_V1", cancelled)?;
        if response == "CAPS P6_SWEEP_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P6_SWEEP_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p6_loft_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS P6_LOFT_V1", cancelled)?;
        if response == "CAPS P6_LOFT_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                P6_LOFT_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_p5_capability(
        &mut self,
        capability: &str,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let request = format!("CAPS {capability}");
        let response = self.request_with_cancellation(&request, cancelled)?;
        if response == request {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(capability.to_owned()))
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

    fn verify_m6_revolve_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M6_REVOLVE_V1", cancelled)?;
        if response == "CAPS M6_REVOLVE_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M6_REVOLVE_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_m6_shell_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M6_SHELL_V1", cancelled)?;
        if response == "CAPS M6_SHELL_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M6_SHELL_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_m14_step_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M14_STEP_V1", cancelled)?;
        if response == "CAPS M14_STEP_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M14_STEP_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_m21_step_model_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M21_STEP_MODEL_V1", cancelled)?;
        if response == "CAPS M21_STEP_MODEL_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M21_STEP_MODEL_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_exact_brep_graph_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let request = format!("CAPS {EXACT_BREP_GRAPH_CAPABILITY}");
        let response = self.request_with_cancellation(&request, cancelled)?;
        if response == request {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                EXACT_BREP_GRAPH_CAPABILITY.to_owned(),
            ))
        }
    }

    fn evaluate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactBRepGraphResult, WorkerError> {
        self.verify_exact_brep_graph_capability(cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_cancellation(
            &format!(
                "EVAL_BREP_GRAPH_V1 {} {}",
                graph.graph_digest,
                hex_encode(&bytes)
            ),
            cancelled,
        )?;
        match parse_exact_brep_graph_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn tessellate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        self.verify_exact_brep_graph_capability(cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_cancellation(
            &format!(
                "TESSELLATE_BREP_GRAPH_V1 {} {} {} {}",
                graph.graph_digest,
                hex_encode(&bytes),
                result_fingerprint,
                hex_encode(output_path.to_string_lossy().as_bytes())
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return Err(parse_error_response(&response, &fields));
        }
        if fields.len() != 7
            || fields[0] != "OK_BREP_GRAPH_MESH_V1"
            || fields[1] != graph.graph_digest
            || fields[2] != result_fingerprint
            || !is_sha256_digest(fields[5])
        {
            return self.fail_protocol(response);
        }
        let (Ok(vertex_count), Ok(triangle_count)) =
            (fields[3].parse::<u32>(), fields[4].parse::<u32>())
        else {
            return self.fail_protocol(response);
        };
        let encoded = std::fs::read(output_path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        if sha256_hex(&encoded) != fields[5] {
            return Err(WorkerError::Transport(
                "exact B-Rep graph mesh digest does not match the worker receipt".to_owned(),
            ));
        }
        let mesh = StepImportMesh::decode(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.vertices_mm.len() as u32 != vertex_count
            || mesh.triangles.len() as u32 != triangle_count
        {
            return Err(WorkerError::Transport(
                "exact B-Rep graph mesh size does not match the worker receipt".to_owned(),
            ));
        }
        Ok(mesh)
    }

    fn export_exact_brep_graph_step_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_exact_brep_graph_capability(cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_cancellation(
            &format!(
                "EXPORT_BREP_GRAPH_STEP_V1 {} {} {} {}",
                graph.graph_digest,
                hex_encode(&bytes),
                result_fingerprint,
                hex_encode(output_path.to_string_lossy().as_bytes())
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return Err(parse_error_response(&response, &fields));
        }
        if fields
            != [
                "OK_BREP_GRAPH_STEP_V1",
                graph.graph_digest.as_str(),
                result_fingerprint,
            ]
        {
            return self.fail_protocol(response);
        }
        Ok(())
    }

    fn evaluate_planar_offset_request_with_cancellation(
        &mut self,
        request: &ExactPlanarOffsetRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerPlanarOffsetResult, WorkerError> {
        self.verify_p6_offset_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "OFFSET_RECTANGLE_P6_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                request.source_bounds_bits[0],
                request.source_bounds_bits[1],
                request.source_bounds_bits[2],
                request.source_bounds_bits[3],
                request.distance_bits,
                request.document_id.0,
                request.offset_feature_id.0,
                request.canonical_input_digest,
            ),
            cancelled,
        )?;
        match parse_p6_offset_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn evaluate_sweep_request_with_cancellation(
        &mut self,
        request: &ExactSweepRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerSweepResult, WorkerError> {
        self.verify_p6_sweep_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "SWEEP_RECTANGLE_P6_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                request.profile_bounds_bits[0],
                request.profile_bounds_bits[1],
                request.profile_bounds_bits[2],
                request.profile_bounds_bits[3],
                request.path_bits[0],
                request.path_bits[1],
                request.path_bits[2],
                request.path_bits[3],
                request.document_id.0,
                request.sweep_feature_id.0,
                request.canonical_input_digest,
            ),
            cancelled,
        )?;
        match parse_p6_sweep_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn evaluate_loft_request_with_cancellation(
        &mut self,
        request: &ExactLoftRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerLoftResult, WorkerError> {
        self.verify_p6_loft_capability(cancelled)?;
        let mut line = format!(
            "LOFT_SPLINE_P6_V1 {} {} {}",
            request.document_id.0, request.loft_feature_id.0, request.canonical_input_digest,
        );
        for value in request.protocol_values() {
            write!(line, " {:016x}", value.to_bits()).unwrap();
        }
        let response = self.request_with_cancellation(&line, cancelled)?;
        match parse_p6_loft_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
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
        request: &ExactFeatureChainRequest,
    ) -> Result<WorkerExactResult, WorkerError> {
        self.extrude_rectangle_request_with_cancellation(request, &NEVER_CANCELLED)
    }

    fn extrude_rectangle_request_with_cancellation(
        &mut self,
        request: &ExactFeatureChainRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactResult, WorkerError> {
        let (response, operation) = if let Some(shell) = &request.shell {
            if let (Some(kind), Some(amount_bits)) =
                (shell.edge_finish_kind, shell.edge_finish_amount_bits)
            {
                self.verify_p5_capability("P5_FINISH_V1", cancelled)?;
                (
                    self.request_with_cancellation(
                        &format!(
                            "FINISH_BOX_P5_V1 {:016x} {:016x} {:016x} {:016x} {} {:016x} {} {} {}",
                            request.width_bits,
                            request.depth_bits,
                            request.height_bits,
                            shell.thickness_bits,
                            match kind {
                                BottleEdgeFinishKind::Fillet => "fillet",
                                BottleEdgeFinishKind::Chamfer => "chamfer",
                            },
                            amount_bits,
                            request.document_id.0,
                            request.producer_feature_id().0,
                            request.canonical_input_digest,
                        ),
                        cancelled,
                    )?,
                    6,
                )
            } else {
                self.verify_p5_capability("P5_SHELL_V1", cancelled)?;
                (
                    self.request_with_cancellation(
                        &format!(
                            "SHELL_BOX_P5_V1 {:016x} {:016x} {:016x} {:016x} {} {} {}",
                            request.width_bits,
                            request.depth_bits,
                            request.height_bits,
                            shell.thickness_bits,
                            request.document_id.0,
                            request.producer_feature_id().0,
                            request.canonical_input_digest,
                        ),
                        cancelled,
                    )?,
                    6,
                )
            }
        } else if let Some(boolean) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.circle.is_some())
        {
            self.verify_p3_circle_capability(cancelled)?;
            let circle = boolean.circle.expect("filtered circular boolean");
            let (line, operation) = if boolean.operation == BooleanOperation::Union {
                (
                    format!(
                        "EXTRUDE_CIRCULAR_UNION_P3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if boolean.operation == BooleanOperation::Intersect {
                (
                    format!(
                        "EXTRUDE_CIRCULAR_INTERSECT_P3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if boolean.operation == BooleanOperation::Split {
                (
                    format!(
                        "EXTRUDE_CIRCULAR_SPLIT_P3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if let Some(pocket_depth_bits) = request.pocket_depth_bits {
                (
                    format!(
                        "EXTRUDE_CIRCULAR_POCKET_P3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        pocket_depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    4,
                )
            } else {
                (
                    format!(
                        "EXTRUDE_CIRCULAR_CUT_P3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    4,
                )
            };
            (self.request_with_cancellation(&line, cancelled)?, operation)
        } else if let Some(boolean) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.profile.is_some())
        {
            let profile = boolean.profile.as_ref().expect("filtered polygon profile");
            self.verify_p3_polygon_cut_capability(cancelled)?;
            let (mut line, operation) = if boolean.operation == BooleanOperation::Union {
                (
                    format!(
                        "EXTRUDE_POLYGON_UNION_P3_V1 {} {:016x} {:016x} {:016x} {} {} {}",
                        profile.segments.len(),
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if boolean.operation == BooleanOperation::Intersect {
                (
                    format!(
                        "EXTRUDE_POLYGON_INTERSECT_P3_V1 {} {:016x} {:016x} {:016x} {} {} {}",
                        profile.segments.len(),
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if boolean.operation == BooleanOperation::Split {
                (
                    format!(
                        "EXTRUDE_POLYGON_SPLIT_P3_V1 {} {:016x} {:016x} {:016x} {} {} {}",
                        profile.segments.len(),
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    0,
                )
            } else if let Some(pocket_depth_bits) = request.pocket_depth_bits {
                (
                    format!(
                        "EXTRUDE_POLYGON_POCKET_P3_V1 {} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        profile.segments.len(),
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        pocket_depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    8,
                )
            } else {
                (
                    format!(
                        "EXTRUDE_POLYGON_CUT_P3_V1 {} {:016x} {:016x} {:016x} {} {} {}",
                        profile.segments.len(),
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    7,
                )
            };
            for segment in &profile.segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => write!(
                        line,
                        " L,{:016x},{:016x},{:016x},{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )
                    .expect("writing to String cannot fail"),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => write!(
                        line,
                        " A,{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        u8::from(*clockwise)
                    )
                    .expect("writing to String cannot fail"),
                }
            }
            (self.request_with_cancellation(&line, cancelled)?, operation)
        } else if let Some(mixed) = &request.mixed_profile {
            self.verify_p3_arc_capability(cancelled)?;
            let mut line = format!(
                "EXTRUDE_MIXED_P3_V1 {} {:016x} {} {} {}",
                mixed.segments.len(),
                request.height_bits,
                request.document_id.0,
                request.producer_feature_id().0,
                request.canonical_input_digest
            );
            for segment in &mixed.segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => write!(
                        line,
                        " L,{:016x},{:016x},{:016x},{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )
                    .expect("writing to String cannot fail"),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => write!(
                        line,
                        " A,{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        u8::from(*clockwise)
                    )
                    .expect("writing to String cannot fail"),
                }
            }
            (self.request_with_cancellation(&line, cancelled)?, 5)
        } else if let Some(circle) = request.circle {
            self.verify_p3_circle_capability(cancelled)?;
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_CIRCLE_P3_V1 {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        circle.center_x_bits,
                        circle.center_y_bits,
                        circle.radius_bits,
                        request.height_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                3,
            )
        } else if let Some(cut) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.operation == BooleanOperation::Cut)
        {
            if let Some(pocket_depth_bits) = request.pocket_depth_bits {
                self.verify_m3_pocket_capability(cancelled)?;
                (
                    self.request_with_cancellation(
                        &format!(
                            "EXTRUDE_POCKET_M3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                            request.width_bits,
                            request.depth_bits,
                            request.height_bits,
                            cut.min_x_bits,
                            cut.min_y_bits,
                            cut.width_bits,
                            cut.depth_bits,
                            pocket_depth_bits,
                            request.document_id.0,
                            request.producer_feature_id().0,
                            request.canonical_input_digest
                        ),
                        cancelled,
                    )?,
                    2,
                )
            } else {
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
                    1,
                )
            }
        } else if let Some(split) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.operation == BooleanOperation::Split)
        {
            self.verify_p6_split_capability(cancelled)?;
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_SPLIT_P6_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        split.min_x_bits,
                        split.min_y_bits,
                        split.width_bits,
                        split.depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                0,
            )
        } else if let Some(intersection) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.operation == BooleanOperation::Intersect)
        {
            self.verify_p6_intersect_capability(cancelled)?;
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_INTERSECT_P6_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        intersection.min_x_bits,
                        intersection.min_y_bits,
                        intersection.width_bits,
                        intersection.depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                0,
            )
        } else if let Some(union) = request
            .boolean
            .as_ref()
            .filter(|boolean| boolean.operation == BooleanOperation::Union)
        {
            self.verify_m3_union_capability(cancelled)?;
            (
                self.request_with_cancellation(
                    &format!(
                        "EXTRUDE_UNION_M3_V1 {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x} {} {} {}",
                        request.width_bits,
                        request.depth_bits,
                        request.height_bits,
                        union.min_x_bits,
                        union.min_y_bits,
                        union.width_bits,
                        union.depth_bits,
                        request.document_id.0,
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                0,
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
                        request.producer_feature_id().0,
                        request.canonical_input_digest
                    ),
                    cancelled,
                )?,
                0,
            )
        };
        let parsed = match operation {
            1 => parse_m3_cut_exact_result(&response),
            2 => parse_m3_pocket_exact_result(&response),
            4 => {
                let [width, depth, _] = request.dimensions_mm();
                let circular_boundary_pocket = request.pocket_depth_bits.is_some()
                    && request.boolean.as_ref().is_some_and(|boolean| {
                        boolean.circle.is_some_and(|circle| {
                            circle.side_overlap(width, depth).is_some()
                                || circle.corner_overlap(width, depth).is_some()
                                || circle.outside_side_overlap(width, depth).is_some()
                                || circle.center_on_side_overlap(width, depth).is_some()
                                || circle.center_on_corner_overlap(width, depth).is_some()
                                || circle.outside_corner_overlap(width, depth).is_some()
                        })
                    });
                parse_p3_circular_cut_result(&response, circular_boundary_pocket)
            }
            7 => parse_p3_polygon_cut_result(&response, false),
            8 => parse_p3_polygon_cut_result(&response, true),
            _ => parse_m3_exact_result(&response),
        };
        match parsed {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn evaluate_revolve_request_with_cancellation(
        &mut self,
        request: &ExactRevolveRequest,
        cancelled: &AtomicBool,
    ) -> Result<WorkerRevolveResult, WorkerError> {
        let mut line = if request.general {
            let response = self.request_with_cancellation("CAPS P4_REVOLVE_V1", cancelled)?;
            if response != "CAPS P4_REVOLVE_V1" {
                self.terminate_worker();
                return Err(WorkerError::MissingCapability("P4_REVOLVE_V1".to_owned()));
            }
            let segments = request.profile_segments();
            let mut line = format!(
                "REVOLVE_P4_V1 {} {} {} {:016x} {:016x} {:016x} {:016x} {:016x} {}",
                request.document_id.0,
                request.revolve_feature_id.0,
                request.canonical_input_digest,
                request.axis_start_bits[0],
                request.axis_start_bits[1],
                request.axis_end_bits[0],
                request.axis_end_bits[1],
                request.angle_degrees_bits,
                segments.len(),
            );
            for segment in &segments {
                match segment {
                    ExactProfileSegment::Line {
                        start_bits,
                        end_bits,
                    } => write!(
                        line,
                        " L,{:016x},{:016x},{:016x},{:016x}",
                        start_bits[0], start_bits[1], end_bits[0], end_bits[1]
                    )
                    .expect("writing to String cannot fail"),
                    ExactProfileSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => write!(
                        line,
                        " A,{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},{}",
                        start_bits[0],
                        start_bits[1],
                        end_bits[0],
                        end_bits[1],
                        center_bits[0],
                        center_bits[1],
                        u8::from(*clockwise),
                    )
                    .expect("writing to String cannot fail"),
                }
            }
            line
        } else if let (
            Some(finish_feature_id),
            Some(finish_kind),
            Some(amount_bits),
            Some(thickness_bits),
        ) = (
            request.edge_finish_feature_id,
            request.edge_finish_kind,
            request.edge_finish_amount_bits,
            request.thickness_bits,
        ) {
            let response = self.request_with_cancellation("CAPS M6_FINISH_V1", cancelled)?;
            if response != "CAPS M6_FINISH_V1" {
                self.terminate_worker();
                return Err(WorkerError::MissingCapability("M6_FINISH_V1".to_owned()));
            }
            format!(
                "FINISH_M6_V1 {} {} {} {:016x} {} {:016x}",
                request.document_id.0,
                finish_feature_id.0,
                request.canonical_input_digest,
                thickness_bits,
                match finish_kind {
                    ketchup_core::document::BottleEdgeFinishKind::Fillet => "fillet",
                    ketchup_core::document::BottleEdgeFinishKind::Chamfer => "chamfer",
                },
                amount_bits,
            )
        } else if let (Some(shell_feature_id), Some(thickness_bits)) =
            (request.shell_feature_id, request.thickness_bits)
        {
            self.verify_m6_shell_capability(cancelled)?;
            format!(
                "SHELL_M6_V1 {} {} {} {:016x}",
                request.document_id.0,
                shell_feature_id.0,
                request.canonical_input_digest,
                thickness_bits,
            )
        } else {
            self.verify_m6_revolve_capability(cancelled)?;
            format!(
                "REVOLVE_M6_V1 {} {} {}",
                request.document_id.0, request.revolve_feature_id.0, request.canonical_input_digest,
            )
        };
        if !request.general {
            for point in &request.points_bits {
                line.push_str(&format!(" {:016x} {:016x}", point[0], point[1]));
            }
        }
        let response = self.request_with_cancellation(&line, cancelled)?;
        match parse_m6_revolve_result(&response) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn export_revolve_step_request_with_cancellation(
        &mut self,
        request: &ExactRevolveRequest,
        expected_result_fingerprint: &str,
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_m14_step_capability(cancelled)?;
        let (kind, thickness, amount) = match (
            request.edge_finish_kind,
            request.thickness_bits,
            request.edge_finish_amount_bits,
        ) {
            (
                Some(ketchup_core::document::BottleEdgeFinishKind::Fillet),
                Some(thickness),
                Some(amount),
            ) => (
                "fillet",
                format!("{thickness:016x}"),
                format!("{amount:016x}"),
            ),
            (
                Some(ketchup_core::document::BottleEdgeFinishKind::Chamfer),
                Some(thickness),
                Some(amount),
            ) => (
                "chamfer",
                format!("{thickness:016x}"),
                format!("{amount:016x}"),
            ),
            (None, Some(thickness), None) => ("shell", format!("{thickness:016x}"), "-".to_owned()),
            (None, None, None) => ("revolve", "-".to_owned(), "-".to_owned()),
            _ => {
                return Err(WorkerError::Protocol(
                    "incomplete STEP export request".to_owned(),
                ));
            }
        };
        let mut line = format!(
            "EXPORT_STEP_M14_V1 {} {} {} {} {kind} {thickness} {amount} {}",
            request.document_id.0,
            request.producer_feature_id().0,
            request.canonical_input_digest,
            expected_result_fingerprint,
            hex_encode(path.to_string_lossy().as_bytes()),
        );
        for point in &request.points_bits {
            line.push_str(&format!(" {:016x} {:016x}", point[0], point[1]));
        }
        let response = self.request_with_cancellation(&line, cancelled)?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&"ERR") {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        if fields.as_slice()
            == [
                "OK_M14_STEP_V1",
                request.canonical_input_digest.as_str(),
                expected_result_fingerprint,
            ]
        {
            Ok(())
        } else {
            self.fail_protocol(response)
        }
    }

    fn export_general_revolve_step_request_with_cancellation(
        &mut self,
        request: &ExactRevolveRequest,
        expected_result_fingerprint: &str,
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_m21_step_model_capability(cancelled)?;
        let specification = serde_json::to_vec(&StepRevolveExportSpec::from(request))
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_cancellation(
            &format!(
                "EXPORT_REVOLVE_STEP_M21_V1 {} {} {} {} {} {}",
                request.document_id.0,
                request.producer_feature_id().0,
                request.canonical_input_digest,
                expected_result_fingerprint,
                hex_encode(path.to_string_lossy().as_bytes()),
                hex_encode(&specification),
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&"ERR") || fields.first() == Some(&"ERR_DETAIL") {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        if fields.as_slice()
            == [
                "OK_M21_REVOLVE_STEP_V1",
                request.canonical_input_digest.as_str(),
                expected_result_fingerprint,
            ]
        {
            Ok(())
        } else {
            self.fail_protocol(response)
        }
    }

    fn export_box_step_request_with_cancellation(
        &mut self,
        request: &ExactFeatureChainRequest,
        expected_result_fingerprint: &str,
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_m21_step_model_capability(cancelled)?;
        let specification = serde_json::to_vec(&StepFeatureExportSpec::from(request))
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_cancellation(
            &format!(
                "EXPORT_FEATURE_STEP_M21_V1 {} {} {} {} {} {}",
                request.document_id.0,
                request.producer_feature_id().0,
                request.canonical_input_digest,
                expected_result_fingerprint,
                hex_encode(path.to_string_lossy().as_bytes()),
                hex_encode(&specification),
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        if fields.as_slice()
            == [
                "OK_M21_BOX_STEP_V1",
                request.canonical_input_digest.as_str(),
                expected_result_fingerprint,
            ]
        {
            Ok(())
        } else {
            self.fail_protocol(response)
        }
    }

    fn inspect_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        self.verify_m21_step_model_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "INSPECT_STEP_PART_M21_V1 {source_sha256} {}",
                hex_encode(path.to_string_lossy().as_bytes())
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        let parse_bits = |index: usize| {
            fields
                .get(index)
                .and_then(|value| u64::from_str_radix(value, 16).ok())
                .map(f64::from_bits)
        };
        let evidence = (|| {
            if fields.len() != 18
                || fields[0] != "OK_M21_STEP_PART_V3"
                || fields[1] != source_sha256
                || !is_fnv1a64_digest(fields[2])
            {
                return None;
            }
            let solid_count = fields[3].parse::<u32>().ok()?;
            let volume_mm3 = parse_bits(4)?;
            let bounds_mm = [
                [parse_bits(5)?, parse_bits(6)?, parse_bits(7)?],
                [parse_bits(8)?, parse_bits(9)?, parse_bits(10)?],
            ];
            let topology_counts = [
                fields[11].parse::<u32>().ok()?,
                fields[12].parse::<u32>().ok()?,
                fields[13].parse::<u32>().ok()?,
                fields[14].parse::<u32>().ok()?,
                solid_count,
            ];
            let source_unit = match hex_decode_utf8(fields[15])?.as_str() {
                "millimetre" => ImportLengthUnit::Millimetre,
                "centimetre" => ImportLengthUnit::Centimetre,
                "metre" => ImportLengthUnit::Metre,
                "inch" => ImportLengthUnit::Inch,
                "foot" => ImportLengthUnit::Foot,
                _ => return None,
            };
            Some(StepImportEvidence {
                source_unit,
                result_fingerprint: fields[2].to_owned(),
                solid_count,
                topology_counts,
                volume_mm3,
                bounds_mm,
                backend: hex_decode_utf8(fields[16])?,
                tolerance: hex_decode_utf8(fields[17])?,
            })
        })();
        match evidence {
            Some(evidence) => Ok(evidence),
            None => self.fail_protocol(response),
        }
    }

    fn tessellate_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        self.verify_m21_step_model_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "TESSELLATE_STEP_PART_M21_V1 {source_sha256} {} {}",
                hex_encode(path.to_string_lossy().as_bytes()),
                hex_encode(output_path.to_string_lossy().as_bytes())
            ),
            cancelled,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        if fields.len() != 7
            || fields[0] != "OK_M21_STEP_MESH_V1"
            || fields[1] != source_sha256
            || fields[2] != result_fingerprint
            || !is_sha256_digest(fields[5])
        {
            return self.fail_protocol(response);
        }
        let (Ok(vertex_count), Ok(triangle_count)) =
            (fields[3].parse::<u32>(), fields[4].parse::<u32>())
        else {
            return self.fail_protocol(response);
        };
        let encoded = std::fs::read(output_path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        if sha256_hex(&encoded) != fields[5] {
            return Err(WorkerError::Transport(
                "imported STEP display mesh digest does not match the worker receipt".to_owned(),
            ));
        }
        let mesh = StepImportMesh::decode(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.vertices_mm.len() as u32 != vertex_count
            || mesh.triangles.len() as u32 != triangle_count
        {
            return Err(WorkerError::Transport(
                "imported STEP display mesh size does not match the worker receipt".to_owned(),
            ));
        }
        Ok(mesh)
    }

    fn assemble_step_model_request_with_cancellation(
        &mut self,
        manifest: &StepAssemblyManifest,
        sources: &[PathBuf],
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_m21_step_model_capability(cancelled)?;
        if manifest.parts.len() != sources.len() {
            return Err(WorkerError::Protocol(
                "STEP assembly manifest/source count mismatch".to_owned(),
            ));
        }
        let manifest_bytes = serde_json::to_vec(manifest)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let manifest_digest = sha256_hex(&manifest_bytes);
        let mut line = format!(
            "ASSEMBLE_STEP_M21_V1 {manifest_digest} {} {} {}",
            hex_encode(path.to_string_lossy().as_bytes()),
            hex_encode(&manifest_bytes),
            sources.len(),
        );
        for source in sources {
            write!(line, " {}", hex_encode(source.to_string_lossy().as_bytes()))
                .expect("writing to String cannot fail");
        }
        let response = self.request_with_cancellation(&line, cancelled)?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return match parse_error_response(&response, &fields) {
                WorkerError::Protocol(response) => self.fail_protocol(response),
                error => Err(error),
            };
        }
        if fields.len() != 4
            || fields[0] != "OK_M21_STEP_MODEL_V1"
            || fields[1] != manifest_digest
            || !is_fnv1a64_digest(fields[2])
            || !is_sha256_digest(fields[3])
        {
            return self.fail_protocol(response);
        }
        let bytes =
            std::fs::read(path).map_err(|error| WorkerError::Transport(error.to_string()))?;
        if sha256_hex(&bytes) != fields[3] {
            return self.fail_protocol("worker STEP output hash mismatch".to_owned());
        }
        Ok(())
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

    pub fn inspect_step_import_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        if std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len()
            > MAX_STEP_SOURCE_BYTES
        {
            return Err(WorkerError::Transport(
                "STEP source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.inspect_step_part_request_with_cancellation(
            path,
            source_sha256,
            cancelled,
        ) {
            Ok(evidence) => Ok(evidence),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, cancelled)?;
                self.client.inspect_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    /// Ask the isolated worker for a bounded display mesh of an imported STEP
    /// part, bound to the result fingerprint the document already committed to.
    pub fn tessellate_step_import_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        if std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len()
            > MAX_STEP_SOURCE_BYTES
        {
            return Err(WorkerError::Transport(
                "STEP source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.tessellate_step_part_request_with_cancellation(
            path,
            source_sha256,
            result_fingerprint,
            output_path,
            cancelled,
        ) {
            Ok(mesh) => Ok(mesh),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, cancelled)?;
                self.client.tessellate_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    result_fingerprint,
                    output_path,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn evaluate_planar_offset(
        &mut self,
        request: &ExactPlanarOffsetRequest,
    ) -> Result<ExactPlanarOffsetPackage, M3EvaluationError> {
        self.client.ensure_not_cancelled(&NEVER_CANCELLED)?;
        let result = match self
            .client
            .evaluate_planar_offset_request_with_cancellation(request, &NEVER_CANCELLED)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client
                    .evaluate_planar_offset_request_with_cancellation(request, &NEVER_CANCELLED)?
            }
            Err(error) => return Err(error.into()),
        };
        validate_planar_offset_worker_result(request, &result)?;
        build_planar_offset_package(
            request,
            PlanarOffsetWorkerEvidence {
                exact_input_digest: result.exact_input_digest,
                result_fingerprint: result.result_fingerprint,
                backend: result.backend,
                tolerance: result.tolerance,
                bounds_mm: [
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
                ],
                area_mm2: result.area_mm2,
                face_ordinal: result.face.ordinal,
                lineage_digest: result.face.lineage_digest,
                corroborating_geometry_fingerprint: result.face.geometric_fingerprint,
            },
        )
        .map_err(Into::into)
    }

    pub fn evaluate_sweep(
        &mut self,
        request: &ExactSweepRequest,
    ) -> Result<ExactSweepPackage, M3EvaluationError> {
        self.client.ensure_not_cancelled(&NEVER_CANCELLED)?;
        let result = match self
            .client
            .evaluate_sweep_request_with_cancellation(request, &NEVER_CANCELLED)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client
                    .evaluate_sweep_request_with_cancellation(request, &NEVER_CANCELLED)?
            }
            Err(error) => return Err(error.into()),
        };
        validate_sweep_worker_result(request, &result)?;
        build_sweep_package(
            request,
            SweepWorkerEvidence {
                exact_input_digest: result.exact_input_digest,
                result_fingerprint: result.result_fingerprint,
                backend: result.backend,
                tolerance: result.tolerance,
                bounds_mm: [
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
                ],
                volume_mm3: result.volume_mm3,
                faces: [
                    ExactFaceRole::SweepStart,
                    ExactFaceRole::SweepEnd,
                    ExactFaceRole::SweepSide0,
                    ExactFaceRole::SweepSide1,
                    ExactFaceRole::SweepSide2,
                    ExactFaceRole::SweepSide3,
                ]
                .into_iter()
                .zip(result.faces)
                .map(|(role, face)| SweepWorkerFaceEvidence {
                    role,
                    face_ordinal: face.ordinal,
                    lineage_digest: face.lineage_digest,
                    corroborating_geometry_fingerprint: face.geometric_fingerprint,
                })
                .collect(),
            },
        )
        .map_err(Into::into)
    }

    pub fn evaluate_loft(
        &mut self,
        request: &ExactLoftRequest,
    ) -> Result<ExactLoftPackage, M3EvaluationError> {
        self.client.ensure_not_cancelled(&NEVER_CANCELLED)?;
        let result = match self
            .client
            .evaluate_loft_request_with_cancellation(request, &NEVER_CANCELLED)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client
                    .evaluate_loft_request_with_cancellation(request, &NEVER_CANCELLED)?
            }
            Err(error) => return Err(error.into()),
        };
        validate_loft_worker_result(request, &result)?;
        build_loft_package(
            request,
            LoftWorkerEvidence {
                exact_input_digest: result.exact_input_digest,
                result_fingerprint: result.result_fingerprint,
                backend: result.backend,
                tolerance: result.tolerance,
                bounds_mm: [
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
                ],
                volume_mm3: result.volume_mm3,
                topology_counts: result.topology_counts,
                faces: [
                    ExactFaceRole::LoftStart,
                    ExactFaceRole::LoftEnd,
                    ExactFaceRole::LoftSide,
                ]
                .into_iter()
                .zip(result.faces)
                .map(|(role, face)| SweepWorkerFaceEvidence {
                    role,
                    face_ordinal: face.ordinal,
                    lineage_digest: face.lineage_digest,
                    corroborating_geometry_fingerprint: face.geometric_fingerprint,
                })
                .collect(),
            },
        )
        .map_err(Into::into)
    }

    pub fn evaluate_exact_brep_graph(
        &mut self,
        graph: &ExactBRepGraph,
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.client.ensure_not_cancelled(&NEVER_CANCELLED)?;
        let result = match self
            .client
            .evaluate_exact_brep_graph_with_cancellation(graph, &NEVER_CANCELLED)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client
                    .evaluate_exact_brep_graph_with_cancellation(graph, &NEVER_CANCELLED)?
            }
            Err(error) => return Err(error),
        };
        if result.canonical_input_digest != graph.canonical_input_digest
            || result.graph_digest != graph.graph_digest
            || result.producer_feature_id != graph.producer_feature_id
            || result.result_fingerprint.is_empty()
            || result.exact_input_digest.is_empty()
            || !result.volume_mm3.is_finite()
            || result.volume_mm3 <= 0.0
            || result.bounds_mm.iter().any(|value| !value.is_finite())
            || result.topology_counts.contains(&0)
            || result.backend.is_empty()
            || result.tolerance.is_empty()
        {
            return Err(WorkerError::Protocol(
                "exact B-Rep graph result does not match its request".to_owned(),
            ));
        }
        let mesh_file = tempfile::Builder::new()
            .prefix(".ketchup-brep-graph-mesh-")
            .suffix(".bin")
            .tempfile()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let mesh = match self.client.tessellate_exact_brep_graph_with_cancellation(
            graph,
            &result.result_fingerprint,
            mesh_file.path(),
            &NEVER_CANCELLED,
        ) {
            Ok(mesh) => mesh,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client.tessellate_exact_brep_graph_with_cancellation(
                    graph,
                    &result.result_fingerprint,
                    mesh_file.path(),
                    &NEVER_CANCELLED,
                )?
            }
            Err(error) => return Err(error),
        };
        ExactBRepGraphPackage::from_worker_evidence(
            graph,
            ExactBRepGraphWorkerEvidence {
                exact_input_digest: result.exact_input_digest,
                result_fingerprint: result.result_fingerprint,
                volume_mm3: result.volume_mm3,
                topology_counts: result.topology_counts,
                bounds_mm: [
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
                ],
                backend: result.backend,
                tolerance: result.tolerance,
            },
            &mesh,
        )
        .map_err(|error| WorkerError::Protocol(error.to_string()))
    }

    pub fn evaluate_rectangle(
        &mut self,
        request: &ExactFeatureChainRequest,
    ) -> Result<ExactRenderPackage, M3EvaluationError> {
        self.evaluate_rectangle_with_cancellation(request, &NEVER_CANCELLED)
    }

    pub fn evaluate_rectangle_with_cancellation(
        &mut self,
        request: &ExactFeatureChainRequest,
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

    pub fn evaluate_revolve(
        &mut self,
        request: &ExactRevolveRequest,
    ) -> Result<ExactRevolvePackage, M6EvaluationError> {
        self.evaluate_revolve_with_cancellation(request, &NEVER_CANCELLED)
    }

    pub fn evaluate_revolve_with_cancellation(
        &mut self,
        request: &ExactRevolveRequest,
        cancelled: &AtomicBool,
    ) -> Result<ExactRevolvePackage, M6EvaluationError> {
        self.client.ensure_not_cancelled(cancelled)?;
        let result = match self
            .client
            .evaluate_revolve_request_with_cancellation(request, cancelled)
        {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, cancelled)?;
                self.client
                    .evaluate_revolve_request_with_cancellation(request, cancelled)?
            }
            Err(error) => return Err(error.into()),
        };
        self.client.ensure_not_cancelled(cancelled)?;
        validate_m6_worker_result(request, &result)?;
        let package = build_m6_revolve_package(request, &result)?;
        self.client.ensure_not_cancelled(cancelled)?;
        Ok(package)
    }

    pub fn export_revolve_step(
        &mut self,
        snapshot: &Snapshot,
        request: &ExactRevolveRequest,
        expected: &ExactRevolvePackage,
        path: &Path,
    ) -> Result<(), M6EvaluationError> {
        if !expected.is_current(snapshot)
            || ExactRevolveRequest::from_snapshot(snapshot, request.definition_id).as_ref()
                != Ok(request)
        {
            return Err(ExactProductError::InvalidWorkerEvidence.into());
        }
        expected.validate_for_request(request)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".ketchup-step-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let temporary = temporary.into_temp_path();
        let export = self.client.export_revolve_step_request_with_cancellation(
            request,
            &expected.identity.result_fingerprint,
            &temporary,
            &NEVER_CANCELLED,
        );
        match export {
            Ok(()) => {}
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client.export_revolve_step_request_with_cancellation(
                    request,
                    &expected.identity.result_fingerprint,
                    &temporary,
                    &NEVER_CANCELLED,
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        temporary
            .persist(path)
            .map_err(|error| WorkerError::Transport(error.error.to_string()))?;
        Ok(())
    }

    pub fn export_exact_brep_graph_step(
        &mut self,
        snapshot: &Snapshot,
        expected: &ExactBRepGraphPackage,
        path: &Path,
    ) -> Result<(), M6EvaluationError> {
        if !expected.is_current(snapshot) {
            return Err(ExactProductError::StaleResult.into());
        }
        let graph = ExactBRepGraph::from_snapshot(
            snapshot,
            expected.identity.definition_id,
            expected.identity.producer_feature_id,
        )
        .map_err(|_| ExactProductError::InvalidWorkerEvidence)?;
        if graph != expected.graph {
            return Err(ExactProductError::InvalidWorkerEvidence.into());
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".ketchup-brep-graph-step-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let temporary = temporary.into_temp_path();
        let export = self.client.export_exact_brep_graph_step_with_cancellation(
            &graph,
            &expected.identity.result_fingerprint,
            &temporary,
            &NEVER_CANCELLED,
        );
        match export {
            Ok(()) => {}
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client.export_exact_brep_graph_step_with_cancellation(
                    &graph,
                    &expected.identity.result_fingerprint,
                    &temporary,
                    &NEVER_CANCELLED,
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        temporary
            .persist(path)
            .map_err(|error| WorkerError::Transport(error.error.to_string()))?;
        Ok(())
    }

    pub fn export_current_model_step(
        &mut self,
        snapshot: &Snapshot,
        occurrences: &[(ExactBodyPackage, Transform)],
        path: &Path,
    ) -> Result<(), M6EvaluationError> {
        if occurrences.is_empty() {
            return Err(ExactProductError::EmptyModelExport.into());
        }
        for (package, _) in occurrences {
            if !package.is_current(snapshot) {
                return Err(ExactProductError::StaleResult.into());
            }
            match package {
                ExactBodyPackage::Rectangle(expected) => {
                    let request = ExactFeatureChainRequest::from_snapshot(
                        snapshot,
                        expected.identity.definition_id,
                    )?;
                    expected.validate_for_request(&request)?;
                }
                ExactBodyPackage::Revolve(expected) => {
                    let request = ExactRevolveRequest::from_snapshot(
                        snapshot,
                        expected.identity.definition_id,
                    )?;
                    expected.validate_for_request(&request)?;
                }
                ExactBodyPackage::Graph(expected) => {
                    if ExactBRepGraph::from_snapshot(
                        snapshot,
                        expected.identity.definition_id,
                        expected.identity.producer_feature_id,
                    )
                    .as_ref()
                        != Ok(&expected.graph)
                    {
                        return Err(ExactProductError::InvalidWorkerEvidence.into());
                    }
                }
                ExactBodyPackage::Imported(expected) => {
                    if sha256_hex(&expected.source_bytes)
                        != expected
                            .source_sha256
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    {
                        return Err(ExactProductError::InvalidWorkerEvidence.into());
                    }
                }
            }
        }

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let directory = tempfile::Builder::new()
            .prefix(".ketchup-step-model-")
            .tempdir_in(parent)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let mut sources = Vec::with_capacity(occurrences.len());
        let mut manifest = StepAssemblyManifest {
            document_id: snapshot.document_id().0,
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            parts: Vec::with_capacity(occurrences.len()),
        };
        for (index, (package, transform)) in occurrences.iter().enumerate() {
            let source = directory.path().join(format!("part-{index}.step"));
            let result = match package {
                ExactBodyPackage::Rectangle(expected) => {
                    let request = ExactFeatureChainRequest::from_snapshot(
                        snapshot,
                        expected.identity.definition_id,
                    )?;
                    self.client.export_box_step_request_with_cancellation(
                        &request,
                        &expected.identity.result_fingerprint,
                        &source,
                        &NEVER_CANCELLED,
                    )
                }
                ExactBodyPackage::Revolve(expected) => {
                    let request = ExactRevolveRequest::from_snapshot(
                        snapshot,
                        expected.identity.definition_id,
                    )?;
                    if request.general {
                        self.client
                            .export_general_revolve_step_request_with_cancellation(
                                &request,
                                &expected.identity.result_fingerprint,
                                &source,
                                &NEVER_CANCELLED,
                            )
                    } else {
                        self.client.export_revolve_step_request_with_cancellation(
                            &request,
                            &expected.identity.result_fingerprint,
                            &source,
                            &NEVER_CANCELLED,
                        )
                    }
                }
                ExactBodyPackage::Graph(expected) => {
                    self.client.export_exact_brep_graph_step_with_cancellation(
                        &expected.graph,
                        &expected.identity.result_fingerprint,
                        &source,
                        &NEVER_CANCELLED,
                    )
                }
                ExactBodyPackage::Imported(expected) => {
                    std::fs::write(&source, &expected.source_bytes)
                        .map_err(|error| WorkerError::Transport(error.to_string()))
                }
            };
            if let Err(error) = result {
                if error.permits_restart() {
                    self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                    match package {
                        ExactBodyPackage::Rectangle(expected) => {
                            let request = ExactFeatureChainRequest::from_snapshot(
                                snapshot,
                                expected.identity.definition_id,
                            )?;
                            self.client.export_box_step_request_with_cancellation(
                                &request,
                                &expected.identity.result_fingerprint,
                                &source,
                                &NEVER_CANCELLED,
                            )?;
                        }
                        ExactBodyPackage::Revolve(expected) => {
                            let request = ExactRevolveRequest::from_snapshot(
                                snapshot,
                                expected.identity.definition_id,
                            )?;
                            if request.general {
                                self.client
                                    .export_general_revolve_step_request_with_cancellation(
                                        &request,
                                        &expected.identity.result_fingerprint,
                                        &source,
                                        &NEVER_CANCELLED,
                                    )?;
                            } else {
                                self.client.export_revolve_step_request_with_cancellation(
                                    &request,
                                    &expected.identity.result_fingerprint,
                                    &source,
                                    &NEVER_CANCELLED,
                                )?;
                            }
                        }
                        ExactBodyPackage::Graph(expected) => {
                            self.client.export_exact_brep_graph_step_with_cancellation(
                                &expected.graph,
                                &expected.identity.result_fingerprint,
                                &source,
                                &NEVER_CANCELLED,
                            )?;
                        }
                        ExactBodyPackage::Imported(expected) => {
                            std::fs::write(&source, &expected.source_bytes)
                                .map_err(|error| WorkerError::Transport(error.to_string()))?;
                        }
                    }
                } else {
                    return Err(error.into());
                }
            }
            let source_bytes = std::fs::read(&source)
                .map_err(|error| WorkerError::Transport(error.to_string()))?;
            let source_sha256 = sha256_hex(&source_bytes);
            let imported_result_fingerprint = self
                .client
                .inspect_step_part_request_with_cancellation(
                    &source,
                    &source_sha256,
                    &NEVER_CANCELLED,
                )?
                .result_fingerprint;
            manifest.parts.push(StepAssemblyPart {
                document_id: snapshot.document_id().0,
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                expected_result_fingerprint: package.result_key().result_fingerprint,
                imported_result_fingerprint,
                source_sha256,
                transform_bits: transform.matrix().map(f64::to_bits),
            });
            sources.push(source);
        }
        let temporary = tempfile::Builder::new()
            .prefix(".ketchup-step-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let temporary = temporary.into_temp_path();
        let assembly = self.client.assemble_step_model_request_with_cancellation(
            &manifest,
            &sources,
            &temporary,
            &NEVER_CANCELLED,
        );
        match assembly {
            Ok(()) => {}
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(&self.executable, &NEVER_CANCELLED)?;
                self.client.assemble_step_model_request_with_cancellation(
                    &manifest,
                    &sources,
                    &temporary,
                    &NEVER_CANCELLED,
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        temporary
            .persist(path)
            .map_err(|error| WorkerError::Transport(error.error.to_string()))?;
        Ok(())
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

fn validate_m6_worker_result(
    request: &ExactRevolveRequest,
    result: &WorkerRevolveResult,
) -> Result<(), ExactProductError> {
    if request.general {
        let expected_roles = request.face_roles();
        let evidence_valid = result.faces.len() == expected_roles.len()
            && result
                .faces
                .iter()
                .zip(expected_roles.iter().copied())
                .enumerate()
                .all(|(index, (evidence, role))| {
                    evidence.ordinal < result.topology_counts[2]
                        && result.faces[..index]
                            .iter()
                            .all(|prior| prior.ordinal != evidence.ordinal)
                        && !evidence.geometric_fingerprint.is_empty()
                        && evidence.lineage_digest
                            == canonical_reference_lineage_digest(
                                request.document_id,
                                request.producer_feature_id(),
                                role.semantic_role(),
                                role.source_element_id(),
                                role.expected_type(),
                            )
                });
        let bounds_valid = result.bounds_mm.into_iter().all(f64::is_finite)
            && result.bounds_mm[0] < result.bounds_mm[3]
            && result.bounds_mm[1] < result.bounds_mm[4]
            && result.bounds_mm[2] < result.bounds_mm[5];
        if result.request_digest != request.canonical_input_digest
            || !is_sha256_digest(&result.request_digest)
            || !is_fnv1a64_digest(&result.exact_input_digest)
            || !is_fnv1a64_digest(&result.result_fingerprint)
            || result.backend != ketchup_exact::backend_fingerprint()
            || result.tolerance != ketchup_exact::tolerance_profile()
            || !evidence_valid
            || !bounds_valid
            || !result.volume_mm3.is_finite()
            || result.volume_mm3 <= 0.0
            || result.topology_counts[2] < expected_roles.len() as u32
            || result.topology_counts[3] != 1
            || result.topology_counts[4] != 1
        {
            return Err(ExactProductError::InvalidWorkerEvidence);
        }
        return Ok(());
    }
    let points = request.points_mm();
    let max_radius = points.iter().map(|point| point[0]).fold(0.0_f64, f64::max);
    let expected_bounds = [
        -max_radius,
        -max_radius,
        points[0][1],
        max_radius,
        max_radius,
        points[5][1],
    ];
    let expected_volume = expected_volume_mm3(request);
    let expected_roles = request.face_roles();
    let volume_valid = expected_volume.is_some_and(|expected| {
        let tolerance = 1.0e-6_f64.max(expected.abs() * 1.0e-10);
        (result.volume_mm3 - expected).abs() <= tolerance
    }) || request.edge_finish_feature_id.is_some()
        && result.volume_mm3.is_finite()
        && result.volume_mm3 > 0.0;
    let topology_face_count_valid = if request.edge_finish_feature_id.is_some() {
        result.topology_counts[2] >= expected_roles.len() as u32
    } else {
        result.topology_counts[2] == expected_roles.len() as u32
    };
    let evidence_valid = result.faces.len() == expected_roles.len()
        && result
            .faces
            .iter()
            .zip(expected_roles.iter().copied())
            .enumerate()
            .all(|(index, (evidence, role))| {
                evidence.ordinal < result.topology_counts[2]
                    && result.faces[..index]
                        .iter()
                        .all(|prior| prior.ordinal != evidence.ordinal)
                    && !evidence.geometric_fingerprint.is_empty()
                    && evidence.lineage_digest
                        == canonical_reference_lineage_digest(
                            request.document_id,
                            request.producer_feature_id(),
                            role.semantic_role(),
                            role.source_element_id(),
                            role.expected_type(),
                        )
            });
    if result.request_digest != request.canonical_input_digest
        || !is_sha256_digest(&result.request_digest)
        || !is_fnv1a64_digest(&result.exact_input_digest)
        || !is_fnv1a64_digest(&result.result_fingerprint)
        || result.backend != ketchup_exact::backend_fingerprint()
        || result.tolerance != ketchup_exact::tolerance_profile()
        || !evidence_valid
        || !volume_valid
        || result
            .bounds_mm
            .into_iter()
            .zip(expected_bounds)
            .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || !topology_face_count_valid
        || result.topology_counts[3] != 1
        || result.topology_counts[4] != 1
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn build_m6_revolve_package(
    request: &ExactRevolveRequest,
    result: &WorkerRevolveResult,
) -> Result<ExactRevolvePackage, ExactProductError> {
    let face_evidence = request
        .face_roles()
        .iter()
        .copied()
        .zip(&result.faces)
        .map(|(role, evidence)| {
            (
                role,
                evidence.lineage_digest.clone(),
                evidence.geometric_fingerprint.clone(),
            )
        })
        .collect();
    build_revolve_package(
        request,
        result.exact_input_digest.clone(),
        result.result_fingerprint.clone(),
        result.backend.clone(),
        result.tolerance.clone(),
        [
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
        ],
        face_evidence,
    )
}

fn validate_loft_worker_result(
    request: &ExactLoftRequest,
    result: &WorkerLoftResult,
) -> Result<(), ExactProductError> {
    let roles = [
        ExactFaceRole::LoftStart,
        ExactFaceRole::LoftEnd,
        ExactFaceRole::LoftSide,
    ];
    let first_z = f64::from_bits(request.sections.first().unwrap().elevation_bits);
    let last_z = f64::from_bits(request.sections.last().unwrap().elevation_bits);
    let contains_inputs = request.sections.iter().all(|section| {
        let z = f64::from_bits(section.elevation_bits);
        section.control_point_bits.iter().all(|point| {
            let [x, y] = point.map(f64::from_bits);
            x >= result.bounds_mm[0] - 1.0e-6
                && x <= result.bounds_mm[3] + 1.0e-6
                && y >= result.bounds_mm[1] - 1.0e-6
                && y <= result.bounds_mm[4] + 1.0e-6
                && z >= result.bounds_mm[2] - 1.0e-6
                && z <= result.bounds_mm[5] + 1.0e-6
        })
    });
    if result.request_digest != request.canonical_input_digest
        || !is_sha256_digest(&result.request_digest)
        || !is_fnv1a64_digest(&result.exact_input_digest)
        || !is_fnv1a64_digest(&result.result_fingerprint)
        || result.backend != ketchup_exact::backend_fingerprint()
        || result.tolerance != ketchup_exact::tolerance_profile()
        || !result.volume_mm3.is_finite()
        || result.volume_mm3 <= 0.0
        || result.bounds_mm.into_iter().any(|value| !value.is_finite())
        || (result.bounds_mm[2] - first_z).abs() > 1.0e-6
        || (result.bounds_mm[5] - last_z).abs() > 1.0e-6
        || !contains_inputs
        || result.topology_counts != [2, 3, 3, 1, 1]
        || result.faces.len() != roles.len()
        || result.faces.iter().zip(roles).any(|(face, role)| {
            face.ordinal >= 3
                || face.geometric_fingerprint.is_empty()
                || face.lineage_digest
                    != canonical_reference_lineage_digest(
                        request.document_id,
                        request.loft_feature_id,
                        role.semantic_role(),
                        role.source_element_id(),
                        role.expected_type(),
                    )
        })
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn validate_sweep_worker_result(
    request: &ExactSweepRequest,
    result: &WorkerSweepResult,
) -> Result<(), ExactProductError> {
    let [min, max] = request.expected_bounds_mm();
    let expected_bounds = [min[0], min[1], min[2], max[0], max[1], max[2]];
    let roles = [
        ExactFaceRole::SweepStart,
        ExactFaceRole::SweepEnd,
        ExactFaceRole::SweepSide0,
        ExactFaceRole::SweepSide1,
        ExactFaceRole::SweepSide2,
        ExactFaceRole::SweepSide3,
    ];
    if result.request_digest != request.canonical_input_digest
        || !is_sha256_digest(&result.request_digest)
        || !is_fnv1a64_digest(&result.exact_input_digest)
        || !is_fnv1a64_digest(&result.result_fingerprint)
        || result.backend != ketchup_exact::backend_fingerprint()
        || result.tolerance != ketchup_exact::tolerance_profile()
        || !result.volume_mm3.is_finite()
        || (result.volume_mm3 - request.expected_volume_mm3()).abs() > 1.0e-6
        || result
            .bounds_mm
            .into_iter()
            .zip(expected_bounds)
            .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || result.topology_counts != [8, 12, 6, 1, 1]
        || result.faces.len() != roles.len()
        || result.faces.iter().zip(roles).any(|(face, role)| {
            face.ordinal >= 6
                || face.geometric_fingerprint.is_empty()
                || face.lineage_digest
                    != canonical_reference_lineage_digest(
                        request.document_id,
                        request.sweep_feature_id,
                        role.semantic_role(),
                        role.source_element_id(),
                        role.expected_type(),
                    )
        })
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn validate_planar_offset_worker_result(
    request: &ExactPlanarOffsetRequest,
    result: &WorkerPlanarOffsetResult,
) -> Result<(), ExactProductError> {
    let [min, max] = request.expected_bounds_mm();
    let expected_bounds = [min[0], min[1], min[2], max[0], max[1], max[2]];
    let expected_lineage = canonical_reference_lineage_digest(
        request.document_id,
        request.offset_feature_id,
        ExactFaceRole::PlanarOffsetFace.semantic_role(),
        ExactFaceRole::PlanarOffsetFace.source_element_id(),
        ExactFaceRole::PlanarOffsetFace.expected_type(),
    );
    if result.request_digest != request.canonical_input_digest
        || !is_sha256_digest(&result.request_digest)
        || !is_fnv1a64_digest(&result.exact_input_digest)
        || !is_fnv1a64_digest(&result.result_fingerprint)
        || result.backend != ketchup_exact::backend_fingerprint()
        || result.tolerance != ketchup_exact::tolerance_profile()
        || !result.area_mm2.is_finite()
        || (result.area_mm2 - request.expected_area_mm2()).abs() > 1.0e-6
        || result
            .bounds_mm
            .into_iter()
            .zip(expected_bounds)
            .any(|(actual, expected)| !actual.is_finite() || (actual - expected).abs() > 1.0e-6)
        || result.topology_counts != [4, 4, 1, 0, 0]
        || result.face.ordinal != 0
        || result.face.geometric_fingerprint.is_empty()
        || result.face.lineage_digest != expected_lineage
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn validate_m3_worker_result(
    request: &ExactFeatureChainRequest,
    result: &WorkerExactResult,
) -> Result<(), ExactProductError> {
    let dimensions = request.dimensions_mm();
    let side_role = if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && (boolean.circle.is_some_and(|circle| {
                (circle.side_overlap(dimensions[0], dimensions[1]).is_some()
                    || circle
                        .corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .outside_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .center_on_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .center_on_corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .outside_corner_overlap(dimensions[0], dimensions[1])
                        .is_some())
                    && f64::from_bits(circle.center_x_bits) + f64::from_bits(circle.radius_bits)
                        > dimensions[0] + 1.0e-6
            }) || request.pocket_depth_bits.is_none()
                && boolean.profile.as_ref().is_some_and(|profile| {
                    profile
                        .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                        .is_some()
                        && f64::from_bits(profile.bounds_bits[0]) > 0.0
                }))
    }) {
        ExactFaceRole::West
    } else if request.circle.is_some()
        || request.boolean.as_ref().is_some_and(|boolean| {
            matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) && boolean.circle.is_some()
                || boolean.operation == BooleanOperation::Split
                    && boolean.circle.is_some_and(|circle| {
                        circle.side_overlap(dimensions[0], dimensions[1]).is_some()
                            || circle
                                .corner_overlap(dimensions[0], dimensions[1])
                                .is_some()
                            || circle
                                .outside_side_overlap(dimensions[0], dimensions[1])
                                .is_some()
                            || circle
                                .center_on_side_overlap(dimensions[0], dimensions[1])
                                .is_some()
                            || circle
                                .center_on_corner_overlap(dimensions[0], dimensions[1])
                                .is_some()
                            || circle
                                .outside_corner_overlap(dimensions[0], dimensions[1])
                                .is_some()
                    })
        })
    {
        ExactFaceRole::CircleSide
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.profile.as_ref().is_some_and(|profile| {
            matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect
            ) && (profile.has_only_line_segments()
                || boolean.operation == BooleanOperation::Union
                    && profile
                        .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                || boolean.operation == BooleanOperation::Intersect
                    && profile
                        .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                        .is_some())
                || boolean.operation == BooleanOperation::Split
                    && profile
                        .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                        .is_some()
        })
    }) || request
        .mixed_profile
        .as_ref()
        .is_some_and(|profile| profile.has_only_line_segments())
    {
        ExactFaceRole::LinearSide
    } else if request.mixed_profile.is_some()
        || request.boolean.as_ref().is_some_and(|boolean| {
            matches!(
                boolean.operation,
                BooleanOperation::Union | BooleanOperation::Intersect | BooleanOperation::Split
            ) && boolean.profile.as_ref().is_some_and(|profile| {
                profile.is_line_arc_d_profile()
                    || profile.is_line_arc_capsule_profile()
                    || profile.is_line_arc_rounded_rectangle_profile()
                    || matches!(
                        boolean.operation,
                        BooleanOperation::Union
                            | BooleanOperation::Intersect
                            | BooleanOperation::Split
                    ) && profile.is_strict_convex_line_arc_profile()
            })
        })
    {
        ExactFaceRole::ArcSide
    } else {
        ExactFaceRole::East
    };
    let circular_boundary_pocket = request.pocket_depth_bits.is_some()
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.operation == BooleanOperation::Cut
                && boolean.circle.is_some_and(|circle| {
                    circle.side_overlap(dimensions[0], dimensions[1]).is_some()
                        || circle
                            .corner_overlap(dimensions[0], dimensions[1])
                            .is_some()
                        || circle
                            .outside_side_overlap(dimensions[0], dimensions[1])
                            .is_some()
                        || circle
                            .center_on_side_overlap(dimensions[0], dimensions[1])
                            .is_some()
                        || circle
                            .center_on_corner_overlap(dimensions[0], dimensions[1])
                            .is_some()
                        || circle
                            .outside_corner_overlap(dimensions[0], dimensions[1])
                            .is_some()
                })
        });
    let mut role_evidence = if request.shell.is_some() {
        vec![
            (ExactFaceRole::BoxShellRim, &result.top),
            (ExactFaceRole::BoxShellOuterBottom, &result.bottom),
            (ExactFaceRole::BoxShellOuterEast, &result.east),
        ]
    } else if circular_boundary_pocket {
        vec![(ExactFaceRole::Top, &result.top), (side_role, &result.east)]
    } else {
        vec![
            (ExactFaceRole::Top, &result.top),
            (ExactFaceRole::Bottom, &result.bottom),
            (side_role, &result.east),
        ]
    };
    let polygon_cut_wall_role = request
        .boolean
        .as_ref()
        .filter(|boolean| boolean.operation == BooleanOperation::Cut)
        .and_then(|boolean| boolean.profile.as_ref())
        .filter(|profile| {
            profile
                .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                    dimensions[0],
                    dimensions[1],
                )
                .is_some()
        })
        .map_or(ExactFaceRole::CutLinear, |_| ExactFaceRole::CutArc);
    if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut && boolean.profile.is_some()
    }) {
        match (
            request.pocket_depth_bits.is_some(),
            &result.cut_west,
            &result.cut_east,
            &result.cut_south,
            &result.cut_north,
            &result.pocket_floor,
        ) {
            (false, Some(wall), None, None, None, None) => {
                role_evidence.push((polygon_cut_wall_role, wall));
            }
            (true, Some(wall), None, None, None, Some(floor)) => {
                role_evidence.extend([
                    (polygon_cut_wall_role, wall),
                    (ExactFaceRole::PocketFloor, floor),
                ]);
            }
            _ => return Err(ExactProductError::InvalidWorkerEvidence),
        }
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut && boolean.circle.is_some()
    }) {
        match (
            request.pocket_depth_bits.is_some(),
            &result.cut_west,
            &result.cut_east,
            &result.cut_south,
            &result.cut_north,
            &result.pocket_floor,
        ) {
            (false, Some(circle), None, None, None, None) => {
                role_evidence.push((ExactFaceRole::CutCircle, circle));
            }
            (true, Some(circle), None, None, None, Some(floor)) if circular_boundary_pocket => {
                role_evidence.extend([
                    (ExactFaceRole::CutCircle, circle),
                    (ExactFaceRole::PocketFloor, floor),
                ]);
            }
            (true, Some(circle), None, None, None, None) if !circular_boundary_pocket => {
                role_evidence.push((ExactFaceRole::CutCircle, circle));
            }
            _ => return Err(ExactProductError::InvalidWorkerEvidence),
        }
    } else {
        match (
            request
                .boolean
                .as_ref()
                .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut),
            request.pocket_depth_bits.is_some(),
            &result.cut_west,
            &result.cut_east,
            &result.cut_south,
            &result.cut_north,
            &result.pocket_floor,
        ) {
            (true, false, Some(west), Some(east), Some(south), Some(north), None) => {
                role_evidence.extend([
                    (ExactFaceRole::CutWest, west),
                    (ExactFaceRole::CutEast, east),
                    (ExactFaceRole::CutSouth, south),
                    (ExactFaceRole::CutNorth, north),
                ]);
            }
            (true, true, Some(west), Some(east), Some(south), Some(north), Some(floor)) => {
                role_evidence.extend([
                    (ExactFaceRole::PocketFloor, floor),
                    (ExactFaceRole::PocketWest, west),
                    (ExactFaceRole::PocketEast, east),
                    (ExactFaceRole::PocketSouth, south),
                    (ExactFaceRole::PocketNorth, north),
                ]);
            }
            (false, false, None, None, None, None, None) => {}
            _ => return Err(ExactProductError::InvalidWorkerEvidence),
        }
    }

    let [expected_min, expected_max] = request.expected_bounds_mm();
    let expected_bounds = [
        expected_min[0],
        expected_min[1],
        expected_min[2],
        expected_max[0],
        expected_max[1],
        expected_max[2],
    ];
    let circular_profile = request
        .circle
        .or_else(|| request.boolean.as_ref().and_then(|boolean| boolean.circle));
    let split = request
        .boolean
        .as_ref()
        .is_some_and(|boolean| boolean.operation == BooleanOperation::Split);
    let (expected_volume, expected_topology) = if let Some(shell) = &request.shell {
        let thickness = f64::from_bits(shell.thickness_bits);
        let shell_volume = dimensions.into_iter().product::<f64>()
            - (dimensions[0] - 2.0 * thickness)
                * (dimensions[1] - 2.0 * thickness)
                * (dimensions[2] - thickness);
        (
            if shell.edge_finish_feature_id.is_some() {
                result.volume_mm3
            } else {
                shell_volume
            },
            result.topology_counts,
        )
    } else if let Some(circle) = circular_profile {
        let radius = f64::from_bits(circle.radius_bits);
        let cylinder_height = request
            .pocket_depth_bits
            .map_or(dimensions[2], f64::from_bits);
        let side_overlap = request.boolean.as_ref().and_then(|boolean| {
            matches!(
                boolean.operation,
                BooleanOperation::Cut
                    | BooleanOperation::Union
                    | BooleanOperation::Intersect
                    | BooleanOperation::Split
            )
            .then(|| {
                circle
                    .side_overlap(dimensions[0], dimensions[1])
                    .or_else(|| circle.corner_overlap(dimensions[0], dimensions[1]))
                    .or_else(|| {
                        matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        )
                        .then(|| circle.outside_side_overlap(dimensions[0], dimensions[1]))
                        .flatten()
                    })
                    .or_else(|| {
                        matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        )
                        .then(|| circle.center_on_side_overlap(dimensions[0], dimensions[1]))
                        .flatten()
                    })
                    .or_else(|| {
                        matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        )
                        .then(|| circle.center_on_corner_overlap(dimensions[0], dimensions[1]))
                        .flatten()
                    })
                    .or_else(|| {
                        matches!(
                            boolean.operation,
                            BooleanOperation::Cut
                                | BooleanOperation::Union
                                | BooleanOperation::Intersect
                                | BooleanOperation::Split
                        )
                        .then(|| circle.outside_corner_overlap(dimensions[0], dimensions[1]))
                        .flatten()
                    })
            })
            .flatten()
        });
        let circle_area = std::f64::consts::PI * radius * radius;
        let overlap_area = side_overlap.map_or(circle_area, |value| value.0);
        let base_volume = dimensions.into_iter().product::<f64>();
        (
            if request.boolean.as_ref().is_some_and(|boolean| {
                boolean.operation == BooleanOperation::Split && boolean.circle.is_some()
            }) {
                base_volume
            } else if request.circle.is_some() {
                circle_area * cylinder_height
            } else if request.boolean.as_ref().is_some_and(|boolean| {
                boolean.operation == BooleanOperation::Union && boolean.circle.is_some()
            }) {
                side_overlap.map_or(circle_area * cylinder_height, |_| {
                    base_volume + (circle_area - overlap_area) * cylinder_height
                })
            } else if request.boolean.as_ref().is_some_and(|boolean| {
                boolean.operation == BooleanOperation::Intersect && boolean.circle.is_some()
            }) {
                overlap_area * cylinder_height
            } else {
                base_volume - overlap_area * cylinder_height
            },
            if request.boolean.as_ref().is_some_and(|boolean| {
                boolean.operation == BooleanOperation::Cut
                    && (circle
                        .corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
                        || circle
                            .center_on_corner_overlap(dimensions[0], dimensions[1])
                            .is_some()
                        || circle
                            .outside_corner_overlap(dimensions[0], dimensions[1])
                            .is_some())
            }) {
                if request.pocket_depth_bits.is_some() {
                    [12, 18, 8, 1, 1]
                } else {
                    [10, 15, 7, 1, 1]
                }
            } else if side_overlap.is_some()
                && request
                    .boolean
                    .as_ref()
                    .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut)
            {
                [12, 18, 8, 1, 1]
            } else {
                result.topology_counts
            },
        )
    } else if let Some(boolean) = request
        .boolean
        .as_ref()
        .filter(|boolean| boolean.profile.is_some())
    {
        let profile = boolean.profile.as_ref().expect("filtered polygon profile");
        let segment_count = profile.segments.len() as u32;
        if boolean.operation == BooleanOperation::Split {
            (
                dimensions.into_iter().product::<f64>(),
                if profile
                    .d_profile_arc_only_clipped_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
                    || profile
                        .capsule_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                {
                    [16, 26, 13, 2, 2]
                } else if profile
                    .capsule_corner_overlap(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [14, 23, 12, 2, 2]
                } else if profile
                    .rounded_rectangle_chord_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [20, 32, 15, 2, 2]
                } else if profile
                    .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [18, 29, 14, 2, 2]
                } else if profile
                    .strict_convex_line_arc_clipped_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [20, 32, 15, 2, 2]
                } else if profile
                    .rounded_rectangle_corner_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [16, 26, 13, 2, 2]
                } else if profile
                    .rounded_rectangle_arc_clipped_corner_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
                {
                    [14, 23, 12, 2, 2]
                } else if profile
                    .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                        dimensions[0],
                        dimensions[1],
                    )
                    .is_some()
                {
                    [12, 20, 11, 2, 2]
                } else {
                    result.topology_counts
                },
            )
        } else if matches!(
            boolean.operation,
            BooleanOperation::Union | BooleanOperation::Intersect
        ) {
            if boolean.operation == BooleanOperation::Union
                && let Some((overlap_area, topology)) = profile
                    .d_profile_arc_only_clipped_side_overlap(dimensions[0], dimensions[1])
                    .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    .or_else(|| {
                        profile
                            .capsule_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [16, 24, 10, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .capsule_corner_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [16, 24, 10, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_chord_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [20, 30, 12, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [16, 24, 10, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_arc_only_clipped_side_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_side_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                            .map(|area| (area, [20, 30, 12, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_corner_overlap_area(dimensions[0], dimensions[1])
                            .map(|area| (area, [22, 33, 13, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_arc_clipped_corner_overlap_area(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|area| (area, [24, 36, 14, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|area| (area, [26, 39, 15, 1, 1]))
                    })
            {
                (
                    dimensions.into_iter().product::<f64>()
                        + (f64::from_bits(profile.area_bits) - overlap_area) * dimensions[2],
                    topology,
                )
            } else if boolean.operation == BooleanOperation::Intersect
                && let Some((overlap_area, topology)) = profile
                    .d_profile_arc_only_clipped_side_overlap(dimensions[0], dimensions[1])
                    .map(|(area, _)| (area, [8, 12, 6, 1, 1]))
                    .or_else(|| {
                        profile
                            .capsule_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [8, 12, 6, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .capsule_corner_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [8, 12, 6, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_chord_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                            .map(|(area, _)| (area, [10, 15, 7, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_arc_only_clipped_side_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [14, 21, 9, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_side_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_south_east_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_south_west_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_north_east_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .strict_convex_line_arc_clipped_north_west_corner_overlap(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|(area, _)| (area, [12, 18, 8, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                            .map(|area| (area, [8, 12, 6, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_corner_overlap_area(dimensions[0], dimensions[1])
                            .map(|area| (area, [10, 15, 7, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_arc_clipped_corner_overlap_area(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|area| (area, [8, 12, 6, 1, 1]))
                    })
                    .or_else(|| {
                        profile
                            .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                                dimensions[0],
                                dimensions[1],
                            )
                            .map(|area| (area, [6, 9, 5, 1, 1]))
                    })
            {
                (overlap_area * dimensions[2], topology)
            } else {
                (
                    f64::from_bits(profile.area_bits) * dimensions[2],
                    [
                        segment_count * 2,
                        segment_count * 3,
                        segment_count + 2,
                        1,
                        1,
                    ],
                )
            }
        } else {
            let cut_height = request
                .pocket_depth_bits
                .map_or(dimensions[2], f64::from_bits);
            let side_clipped_overlap = profile
                .d_profile_arc_only_clipped_side_overlap(dimensions[0], dimensions[1])
                .map(|(area, _)| (area, [16, 24, 10, 1, 1]))
                .or_else(|| {
                    profile
                        .capsule_side_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| (area, [16, 24, 10, 1, 1]))
                })
                .or_else(|| {
                    profile
                        .capsule_corner_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [14, 21, 9, 1, 1]
                            } else {
                                [12, 18, 8, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .rounded_rectangle_chord_side_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| (area, [20, 30, 12, 1, 1]))
                })
                .or_else(|| {
                    profile
                        .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                        .map(|area| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [12, 18, 8, 1, 1]
                            } else {
                                [8, 12, 6, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .rounded_rectangle_corner_overlap_area(dimensions[0], dimensions[1])
                        .map(|area| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [16, 24, 10, 1, 1]
                            } else {
                                [14, 21, 9, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .rounded_rectangle_arc_clipped_corner_overlap_area(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|area| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [14, 21, 9, 1, 1]
                            } else {
                                [12, 18, 8, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|area| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [12, 18, 8, 1, 1]
                            } else {
                                [10, 15, 7, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| (area, [18, 27, 11, 1, 1]))
                })
                .or_else(|| {
                    profile
                        .strict_convex_arc_only_clipped_side_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| (area, [22, 33, 13, 1, 1]))
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_arc_clipped_side_overlap(dimensions[0], dimensions[1])
                        .map(|(area, _)| (area, [20, 30, 12, 1, 1]))
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_arc_clipped_south_east_corner_overlap(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|(area, _)| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [18, 27, 11, 1, 1]
                            } else {
                                [16, 24, 10, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_arc_clipped_south_west_corner_overlap(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|(area, _)| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [18, 27, 11, 1, 1]
                            } else {
                                [16, 24, 10, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_arc_clipped_north_east_corner_overlap(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|(area, _)| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [18, 27, 11, 1, 1]
                            } else {
                                [16, 24, 10, 1, 1]
                            };
                            (area, topology)
                        })
                })
                .or_else(|| {
                    profile
                        .strict_convex_line_arc_clipped_north_west_corner_overlap(
                            dimensions[0],
                            dimensions[1],
                        )
                        .map(|(area, _)| {
                            let topology = if request.pocket_depth_bits.is_some() {
                                [18, 27, 11, 1, 1]
                            } else {
                                [16, 24, 10, 1, 1]
                            };
                            (area, topology)
                        })
                });
            (
                dimensions.into_iter().product::<f64>()
                    - side_clipped_overlap
                        .map_or_else(|| f64::from_bits(profile.area_bits), |(area, _)| area)
                        * cut_height,
                side_clipped_overlap.map_or_else(
                    || {
                        [
                            (segment_count + 4) * 2,
                            (segment_count + 4) * 3,
                            segment_count + 6 + u32::from(request.pocket_depth_bits.is_some()),
                            1,
                            1,
                        ]
                    },
                    |(_, topology)| topology,
                ),
            )
        }
    } else if let Some(mixed) = &request.mixed_profile {
        let segment_count = mixed.segments.len() as u32;
        (
            f64::from_bits(mixed.area_bits) * dimensions[2],
            [
                segment_count * 2,
                segment_count * 3,
                segment_count + 2,
                1,
                1,
            ],
        )
    } else {
        match request.boolean.as_ref().map(|boolean| boolean.operation) {
            Some(BooleanOperation::Cut) => {
                let cut = request.boolean.as_ref().expect("matched Boolean cut");
                let cut_height = request
                    .pocket_depth_bits
                    .map_or(dimensions[2], f64::from_bits);
                let cut_volume =
                    f64::from_bits(cut.width_bits) * f64::from_bits(cut.depth_bits) * cut_height;
                (
                    dimensions.into_iter().product::<f64>() - cut_volume,
                    if request.pocket_depth_bits.is_some() {
                        [16, 24, 11, 1, 1]
                    } else {
                        [16, 24, 10, 1, 1]
                    },
                )
            }
            Some(BooleanOperation::Union) => (
                (expected_max[0] - expected_min[0])
                    * (expected_max[1] - expected_min[1])
                    * (expected_max[2] - expected_min[2]),
                [8, 12, 6, 1, 1],
            ),
            Some(BooleanOperation::Intersect) => (
                (expected_max[0] - expected_min[0])
                    * (expected_max[1] - expected_min[1])
                    * (expected_max[2] - expected_min[2]),
                [8, 12, 6, 1, 1],
            ),
            Some(BooleanOperation::Split) => (
                dimensions.into_iter().product::<f64>(),
                result.topology_counts,
            ),
            None => (dimensions.into_iter().product::<f64>(), [8, 12, 6, 1, 1]),
        }
    };
    let volume_tolerance = if request.boolean.as_ref().is_some_and(|boolean| {
        matches!(
            boolean.operation,
            BooleanOperation::Cut | BooleanOperation::Intersect
        ) && boolean.circle.is_some_and(|circle| {
            circle
                .outside_corner_overlap(dimensions[0], dimensions[1])
                .is_some()
        })
    }) {
        2.0e-3
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .capsule_corner_overlap(dimensions[0], dimensions[1])
                    .is_some()
            })
    }) {
        5.0e-2
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Cut
            && boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_arc_clipped_corner_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
            })
    }) {
        3.0e-3
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.profile.as_ref().is_some_and(|profile| {
            profile.is_line_arc_capsule_profile()
                || profile.is_line_arc_rounded_rectangle_profile()
                || profile.is_strict_convex_line_arc_profile()
        })
    }) {
        2.0e-3
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean
            .profile
            .as_ref()
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    }) {
        1.0e-3
    } else if request
        .boolean
        .as_ref()
        .is_some_and(|boolean| boolean.profile.is_some())
    {
        1.0e-6_f64.max(expected_volume.abs() * 1.0e-9)
    } else {
        1.0e-6_f64.max(expected_volume.abs() * 1.0e-10)
    };
    let producer_feature_id = request.producer_feature_id();
    let has_canonical_lineage = |role: ExactFaceRole, evidence: &WorkerFaceEvidence| {
        !evidence.geometric_fingerprint.is_empty()
            && evidence.lineage_digest
                == canonical_reference_lineage_digest(
                    request.document_id,
                    producer_feature_id,
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
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
    let circular_split = split
        && request
            .boolean
            .as_ref()
            .is_some_and(|boolean| boolean.circle.is_some());
    let corner_overlap_split = split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_corner_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let arc_clipped_corner_overlap_split = split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_arc_clipped_corner_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let two_axis_arc_clipped_corner_overlap_split = split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.profile.as_ref().is_some_and(|profile| {
                profile
                    .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                        dimensions[0],
                        dimensions[1],
                    )
                    .is_some()
            })
        });
    let circular_side_overlap_split = circular_split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.circle.is_some_and(|circle| {
                circle.side_overlap(dimensions[0], dimensions[1]).is_some()
                    || circle
                        .corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .outside_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .center_on_side_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .center_on_corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
                    || circle
                        .outside_corner_overlap(dimensions[0], dimensions[1])
                        .is_some()
            })
        });
    let outside_circular_side_overlap_split = circular_split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.circle.is_some_and(|circle| {
                circle
                    .outside_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let center_on_side_circular_split = circular_split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.circle.is_some_and(|circle| {
                circle
                    .center_on_side_overlap(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let center_on_corner_circular_split = circular_split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.circle.is_some_and(|circle| {
                circle
                    .center_on_corner_overlap(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let outside_corner_circular_split = circular_split
        && request.boolean.as_ref().is_some_and(|boolean| {
            boolean.circle.is_some_and(|circle| {
                circle
                    .outside_corner_overlap(dimensions[0], dimensions[1])
                    .is_some()
            })
        });
    let split_topology_valid = !split
        || if outside_corner_circular_split {
            result.topology_counts == [12, 20, 11, 2, 2]
        } else if outside_circular_side_overlap_split {
            matches!(
                result.topology_counts,
                [12, 20, 11, 2, 2] | [14, 23, 12, 2, 2]
            )
        } else if center_on_corner_circular_split {
            result.topology_counts == [12, 20, 11, 2, 2]
        } else if center_on_side_circular_split {
            let circle = request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.circle)
                .expect("classified center-on-side circular Split");
            result.topology_counts
                == if f64::from_bits(circle.center_x_bits).abs() <= 1.0e-6 {
                    [14, 23, 12, 2, 2]
                } else {
                    [12, 20, 11, 2, 2]
                }
        } else if circular_side_overlap_split {
            let circle = request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.circle)
                .expect("classified circular Split");
            let east_overlap = f64::from_bits(circle.center_x_bits)
                + f64::from_bits(circle.radius_bits)
                > dimensions[0] + 1.0e-6;
            result.topology_counts
                == if east_overlap {
                    [12, 20, 11, 2, 2]
                } else {
                    [14, 23, 12, 2, 2]
                }
        } else if circular_split {
            result.topology_counts == [10, 15, 9, 2, 2]
        } else if corner_overlap_split {
            result.topology_counts == [16, 26, 13, 2, 2]
        } else if arc_clipped_corner_overlap_split {
            result.topology_counts == [14, 23, 12, 2, 2]
        } else if two_axis_arc_clipped_corner_overlap_split {
            result.topology_counts == [12, 20, 11, 2, 2]
        } else {
            result.topology_counts[0] >= 12
                && result.topology_counts[1] >= 18
                && result.topology_counts[2] >= 10
                && result.topology_counts[3] >= 2
                && result.topology_counts[3] == result.topology_counts[4]
        };
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
        || !split_topology_valid
        || !split && result.topology_counts != expected_topology
        || request.shell.is_some()
            && (result.topology_counts[2] < role_evidence.len() as u32
                || result.topology_counts[3] != 1
                || result.topology_counts[4] != 1)
        || circular_profile.is_some()
            && !split
            && (result.topology_counts[3] != 1
                || result.topology_counts[4] != 1
                || result.topology_counts[2] < role_evidence.len() as u32)
    {
        return Err(ExactProductError::InvalidWorkerEvidence);
    }
    Ok(())
}

fn build_m3_render_package(
    request: &ExactFeatureChainRequest,
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
    if request.shell.is_some() {
        build_box_render_package(
            request,
            result.exact_input_digest.clone(),
            result.result_fingerprint.clone(),
            result.backend.clone(),
            result.tolerance.clone(),
            bounds,
            [
                evidence(ExactFaceRole::BoxShellRim, &result.top),
                evidence(ExactFaceRole::BoxShellOuterBottom, &result.bottom),
                evidence(ExactFaceRole::BoxShellOuterEast, &result.east),
            ],
        )
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        matches!(
            boolean.operation,
            BooleanOperation::Union | BooleanOperation::Intersect
        ) && boolean.profile.is_some()
    }) {
        let dimensions = request.dimensions_mm();
        let side_role = if request.boolean.as_ref().is_some_and(|boolean| {
            boolean.profile.as_ref().is_some_and(|profile| {
                profile.has_only_line_segments()
                    || boolean.operation == BooleanOperation::Union
                        && profile
                            .strict_convex_line_clipped_side_overlap(dimensions[0], dimensions[1])
                            .is_some()
                    || boolean.operation == BooleanOperation::Intersect
                        && profile
                            .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                            .is_some()
            })
        }) {
            ExactFaceRole::LinearSide
        } else {
            ExactFaceRole::ArcSide
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
                evidence(side_role, &result.east),
            ],
        )
    } else if request.boolean.as_ref().is_some_and(|boolean| {
        boolean.operation == BooleanOperation::Split && boolean.profile.is_some()
    }) {
        let dimensions = request.dimensions_mm();
        let side_role = if request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| {
                profile
                    .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                    .is_some()
            }) {
            ExactFaceRole::LinearSide
        } else if request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| {
                profile.is_line_arc_d_profile()
                    || profile.is_line_arc_capsule_profile()
                    || profile.is_line_arc_rounded_rectangle_profile()
                    || profile.is_strict_convex_line_arc_profile()
            })
        {
            ExactFaceRole::ArcSide
        } else {
            ExactFaceRole::East
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
                evidence(side_role, &result.east),
            ],
        )
    } else if request
        .boolean
        .as_ref()
        .is_some_and(|boolean| boolean.profile.is_some())
    {
        let Some(wall) = &result.cut_west else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        let dimensions = request.dimensions_mm();
        let two_axis_arc_clipped_corner_cut = request.boolean.as_ref().is_some_and(|boolean| {
            boolean.operation == BooleanOperation::Cut
                && boolean.profile.as_ref().is_some_and(|profile| {
                    profile
                        .rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(
                            dimensions[0],
                            dimensions[1],
                        )
                        .is_some()
                })
        });
        let side_role = if request.pocket_depth_bits.is_none()
            && request.boolean.as_ref().is_some_and(|boolean| {
                boolean.operation == BooleanOperation::Cut
                    && boolean.profile.as_ref().is_some_and(|profile| {
                        profile
                            .rounded_rectangle_side_overlap_area(dimensions[0], dimensions[1])
                            .is_some()
                            && f64::from_bits(profile.bounds_bits[0]) > 0.0
                    })
            }) {
            ExactFaceRole::West
        } else {
            ExactFaceRole::East
        };
        if let Some(floor) = &result.pocket_floor {
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
                    evidence(side_role, &result.east),
                    evidence(
                        if two_axis_arc_clipped_corner_cut {
                            ExactFaceRole::CutArc
                        } else {
                            ExactFaceRole::CutLinear
                        },
                        wall,
                    ),
                    evidence(ExactFaceRole::PocketFloor, floor),
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
                    evidence(side_role, &result.east),
                    evidence(
                        if two_axis_arc_clipped_corner_cut {
                            ExactFaceRole::CutArc
                        } else {
                            ExactFaceRole::CutLinear
                        },
                        wall,
                    ),
                ],
            )
        }
    } else if let Some(boolean) = request
        .boolean
        .as_ref()
        .filter(|boolean| boolean.circle.is_some())
    {
        if boolean.operation == BooleanOperation::Split {
            let [width, depth, _] = request.dimensions_mm();
            let side_role = if boolean.circle.is_some_and(|circle| {
                circle.side_overlap(width, depth).is_some()
                    || circle.corner_overlap(width, depth).is_some()
                    || circle.outside_side_overlap(width, depth).is_some()
                    || circle.center_on_side_overlap(width, depth).is_some()
                    || circle.center_on_corner_overlap(width, depth).is_some()
                    || circle.outside_corner_overlap(width, depth).is_some()
            }) {
                ExactFaceRole::CircleSide
            } else {
                ExactFaceRole::East
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
                    evidence(side_role, &result.east),
                ],
            )
        } else if matches!(
            boolean.operation,
            BooleanOperation::Union | BooleanOperation::Intersect
        ) {
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
                    evidence(ExactFaceRole::CircleSide, &result.east),
                ],
            )
        } else {
            let Some(circle_wall) = &result.cut_west else {
                return Err(ExactProductError::InvalidWorkerEvidence);
            };
            let [width, depth, _] = request.dimensions_mm();
            let side_role = if boolean.circle.is_some_and(|circle| {
                (circle.side_overlap(width, depth).is_some()
                    || circle.corner_overlap(width, depth).is_some()
                    || circle.outside_side_overlap(width, depth).is_some()
                    || circle.center_on_side_overlap(width, depth).is_some()
                    || circle.center_on_corner_overlap(width, depth).is_some()
                    || circle.outside_corner_overlap(width, depth).is_some())
                    && f64::from_bits(circle.center_x_bits) + f64::from_bits(circle.radius_bits)
                        > width + 1.0e-6
            }) {
                ExactFaceRole::West
            } else {
                ExactFaceRole::East
            };
            if let Some(floor) = &result.pocket_floor {
                build_box_render_package(
                    request,
                    result.exact_input_digest.clone(),
                    result.result_fingerprint.clone(),
                    result.backend.clone(),
                    result.tolerance.clone(),
                    bounds,
                    [
                        evidence(ExactFaceRole::Top, &result.top),
                        evidence(side_role, &result.east),
                        evidence(ExactFaceRole::CutCircle, circle_wall),
                        evidence(ExactFaceRole::PocketFloor, floor),
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
                        evidence(side_role, &result.east),
                        evidence(ExactFaceRole::CutCircle, circle_wall),
                    ],
                )
            }
        }
    } else if request.circle.is_some() {
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
                evidence(ExactFaceRole::CircleSide, &result.east),
            ],
        )
    } else if let Some(profile) = &request.mixed_profile {
        let side_role = if profile.has_only_line_segments() {
            ExactFaceRole::LinearSide
        } else {
            ExactFaceRole::ArcSide
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
                evidence(side_role, &result.east),
            ],
        )
    } else if request
        .boolean
        .as_ref()
        .is_some_and(|boolean| boolean.operation == BooleanOperation::Cut)
    {
        let (Some(cut_west), Some(cut_east), Some(cut_south), Some(cut_north)) = (
            &result.cut_west,
            &result.cut_east,
            &result.cut_south,
            &result.cut_north,
        ) else {
            return Err(ExactProductError::InvalidWorkerEvidence);
        };
        if let Some(floor) = &result.pocket_floor {
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
                    evidence(ExactFaceRole::PocketFloor, floor),
                    evidence(ExactFaceRole::PocketWest, cut_west),
                    evidence(ExactFaceRole::PocketEast, cut_east),
                    evidence(ExactFaceRole::PocketSouth, cut_south),
                    evidence(ExactFaceRole::PocketNorth, cut_north),
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
                    evidence(ExactFaceRole::CutWest, cut_west),
                    evidence(ExactFaceRole::CutEast, cut_east),
                    evidence(ExactFaceRole::CutSouth, cut_south),
                    evidence(ExactFaceRole::CutNorth, cut_north),
                ],
            )
        }
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
pub enum M6EvaluationError {
    Worker(WorkerError),
    Product(ExactProductError),
}

impl fmt::Display for M6EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker(error) => error.fmt(formatter),
            Self::Product(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for M6EvaluationError {}

impl From<WorkerError> for M6EvaluationError {
    fn from(error: WorkerError) -> Self {
        Self::Worker(error)
    }
}

impl From<ExactProductError> for M6EvaluationError {
    fn from(error: ExactProductError) -> Self {
        Self::Product(error)
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

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode_utf8(value: &str) -> Option<String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

fn push_aabb_request(line: &mut String, bounds: Aabb) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        line.push_str(&format!(" {:016x}", value.to_bits()));
    }
}

fn parse_exact_brep_graph_result(
    response: &str,
) -> Result<WorkerExactBRepGraphResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 20
        || fields[0] != "OK_BREP_GRAPH_V1"
        || !is_sha256_digest(fields[1])
        || !is_sha256_digest(fields[2])
        || !is_fnv1a64_digest(fields[4])
        || !is_fnv1a64_digest(fields[5])
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
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
    Ok(WorkerExactBRepGraphResult {
        canonical_input_digest: fields[1].to_owned(),
        graph_digest: fields[2].to_owned(),
        producer_feature_id: fields[3]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        result_fingerprint: fields[4].to_owned(),
        exact_input_digest: fields[5].to_owned(),
        volume_mm3: parse_f64(6)?,
        bounds_mm: [
            parse_f64(7)?,
            parse_f64(8)?,
            parse_f64(9)?,
            parse_f64(10)?,
            parse_f64(11)?,
            parse_f64(12)?,
        ],
        topology_counts: [
            parse_u32(13)?,
            parse_u32(14)?,
            parse_u32(15)?,
            parse_u32(16)?,
            parse_u32(17)?,
        ],
        backend: hex_decode_utf8(fields[18])
            .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?,
        tolerance: hex_decode_utf8(fields[19])
            .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?,
    })
}

fn parse_p6_loft_result(response: &str) -> Result<WorkerLoftResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 27
        || fields[0] != "OK_P6_LOFT_V1"
        || !is_fnv1a64_digest(fields[1])
        || !is_sha256_digest(fields[14])
        || !is_fnv1a64_digest(fields[15])
        || fields[16].is_empty()
        || fields[17].is_empty()
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
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
    let mut faces = Vec::with_capacity(3);
    for index in 0..3 {
        let offset = 18 + index * 3;
        if !is_fnv1a64_digest(fields[offset + 1]) || !is_fnv1a64_digest(fields[offset + 2]) {
            return Err(WorkerError::Protocol(response.to_owned()));
        }
        faces.push(WorkerFaceEvidence {
            ordinal: parse_u32(offset)?,
            geometric_fingerprint: fields[offset + 1].to_owned(),
            lineage_digest: fields[offset + 2].to_owned(),
        });
    }
    Ok(WorkerLoftResult {
        result_fingerprint: fields[1].to_owned(),
        volume_mm3: parse_f64(2)?,
        bounds_mm: [
            parse_f64(3)?,
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
        ],
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
        faces,
    })
}

fn parse_p6_sweep_result(response: &str) -> Result<WorkerSweepResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 36
        || fields[0] != "OK_P6_SWEEP_V1"
        || !is_fnv1a64_digest(fields[1])
        || !is_sha256_digest(fields[14])
        || !is_fnv1a64_digest(fields[15])
        || fields[16].is_empty()
        || fields[17].is_empty()
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
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
    let mut faces = Vec::with_capacity(6);
    for index in 0..6 {
        let offset = 18 + index * 3;
        if !is_fnv1a64_digest(fields[offset + 1]) || !is_fnv1a64_digest(fields[offset + 2]) {
            return Err(WorkerError::Protocol(response.to_owned()));
        }
        faces.push(WorkerFaceEvidence {
            ordinal: parse_u32(offset)?,
            geometric_fingerprint: fields[offset + 1].to_owned(),
            lineage_digest: fields[offset + 2].to_owned(),
        });
    }
    Ok(WorkerSweepResult {
        result_fingerprint: fields[1].to_owned(),
        volume_mm3: parse_f64(2)?,
        bounds_mm: [
            parse_f64(3)?,
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
        ],
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
        faces,
    })
}

fn parse_p6_offset_result(response: &str) -> Result<WorkerPlanarOffsetResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 22
        || fields[0] != "OK_P6_OFFSET_V1"
        || fields[1].parse::<u128>().is_err()
        || !is_fnv1a64_digest(fields[2])
        || !is_sha256_digest(fields[15])
        || !is_fnv1a64_digest(fields[16])
        || fields[17].is_empty()
        || fields[18].is_empty()
        || !is_fnv1a64_digest(fields[20])
        || !is_fnv1a64_digest(fields[21])
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
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
    Ok(WorkerPlanarOffsetResult {
        backend_duration: Duration::from_nanos(
            fields[1]
                .parse::<u64>()
                .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        ),
        result_fingerprint: fields[2].to_owned(),
        area_mm2: parse_f64(3)?,
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
        face: WorkerFaceEvidence {
            ordinal: parse_u32(19)?,
            geometric_fingerprint: fields[20].to_owned(),
            lineage_digest: fields[21].to_owned(),
        },
    })
}

fn parse_m6_revolve_result(response: &str) -> Result<WorkerRevolveResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    let (face_count, evidence_offset) = match fields.first().copied() {
        Some("OK_M6_REVOLVE_V1") => (5, 19),
        Some("OK_M6_SHELL_V1") | Some("OK_M6_FINISH_V1") => (9, 19),
        Some("OK_P4_REVOLVE_V1") => {
            let count = fields
                .get(19)
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?;
            if !matches!(count, 2 | 4) {
                return Err(WorkerError::Protocol(response.to_owned()));
            }
            (count, 20)
        }
        _ => return Err(WorkerError::Protocol(response.to_owned())),
    };
    let expected_len = evidence_offset + face_count * 3;
    if fields.len() != expected_len
        || fields[1].parse::<u128>().is_err()
        || !is_fnv1a64_digest(fields[2])
        || !is_sha256_digest(fields[15])
        || !is_fnv1a64_digest(fields[16])
        || fields[17].is_empty()
        || fields[18].is_empty()
        || (0..face_count).any(|index| {
            let offset = evidence_offset + index * 3;
            !is_fnv1a64_digest(fields[offset + 1]) || !is_fnv1a64_digest(fields[offset + 2])
        })
    {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
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
    let mut faces = Vec::with_capacity(face_count);
    for index in 0..face_count {
        let offset = evidence_offset + index * 3;
        faces.push(WorkerFaceEvidence {
            ordinal: parse_u32(offset)?,
            geometric_fingerprint: fields[offset + 1].to_owned(),
            lineage_digest: fields[offset + 2].to_owned(),
        });
    }
    Ok(WorkerRevolveResult {
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
        faces,
    })
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
        [
            "ERR_DETAIL",
            code,
            diagnostic,
            operation,
            input_digest,
            backend,
        ] if is_geometry_error_code(code)
            && (is_sha256_digest(input_digest) || is_fnv1a64_digest(input_digest)) =>
        {
            match (
                hex_decode_utf8(diagnostic),
                hex_decode_utf8(operation),
                hex_decode_utf8(backend),
            ) {
                (Some(diagnostic), Some(operation), Some(backend)) => {
                    WorkerError::Geometry(format!(
                        "{code}; operation={operation}; diagnostic={diagnostic}; input_digest={input_digest}; backend={backend}"
                    ))
                }
                _ => WorkerError::Protocol(response.to_owned()),
            }
        }
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
        pocket_floor: None,
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
        pocket_floor: None,
    })
}

fn parse_p3_circular_cut_result(
    response: &str,
    pocket: bool,
) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 31 || fields[0] != "OK_P3_CIRCULAR_CUT_V1" {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_hex = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let evidence = |index: usize| {
        Ok(WorkerFaceEvidence {
            ordinal: parse_u32(index)?,
            geometric_fingerprint: fields[index + 1].to_owned(),
            lineage_digest: fields[index + 2].to_owned(),
        })
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: f64::from_bits(parse_hex(3)?),
        bounds_mm: [4, 5, 6, 7, 8, 9]
            .map(|index| parse_hex(index).map(f64::from_bits))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        topology_counts: [10, 11, 12, 13, 14]
            .map(parse_u32)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        request_digest: fields[15].to_owned(),
        exact_input_digest: fields[16].to_owned(),
        backend: fields[17].to_owned(),
        tolerance: fields[18].to_owned(),
        top: evidence(19)?,
        bottom: evidence(22)?,
        east: evidence(25)?,
        cut_west: Some(evidence(28)?),
        cut_east: None,
        cut_south: None,
        cut_north: None,
        pocket_floor: pocket.then(|| evidence(22)).transpose()?,
    })
}

fn parse_p3_polygon_cut_result(
    response: &str,
    pocket: bool,
) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    let mut digest_indexes = vec![2, 16, 20, 21, 23, 24, 26, 27, 29, 30];
    if pocket {
        digest_indexes.extend([32, 33]);
    }
    if fields.len() != if pocket { 34 } else { 31 }
        || fields[0]
            != if pocket {
                "OK_P3_POLYGON_POCKET_V1"
            } else {
                "OK_P3_POLYGON_CUT_V1"
            }
        || fields[17].is_empty()
        || fields[18].is_empty()
        || !is_sha256_digest(fields[15])
        || digest_indexes
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
    let evidence = |index: usize| {
        Ok(WorkerFaceEvidence {
            ordinal: parse_u32(index)?,
            geometric_fingerprint: fields[index + 1].to_owned(),
            lineage_digest: fields[index + 2].to_owned(),
        })
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: parse_f64(3)?,
        bounds_mm: [4, 5, 6, 7, 8, 9]
            .map(parse_f64)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        topology_counts: [10, 11, 12, 13, 14]
            .map(parse_u32)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))?,
        request_digest: fields[15].to_owned(),
        exact_input_digest: fields[16].to_owned(),
        backend: fields[17].to_owned(),
        tolerance: fields[18].to_owned(),
        top: evidence(19)?,
        bottom: evidence(22)?,
        east: evidence(25)?,
        cut_west: Some(evidence(28)?),
        cut_east: None,
        cut_south: None,
        cut_north: None,
        pocket_floor: pocket.then(|| evidence(31)).transpose()?,
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
        pocket_floor: None,
    })
}

fn parse_m3_pocket_exact_result(response: &str) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(parse_error_response(response, &fields));
    }
    if fields.len() != 43
        || fields[0] != "OK_M3_POCKET_V1"
        || fields[17].is_empty()
        || fields[18].is_empty()
        || !is_sha256_digest(fields[15])
        || [
            2, 16, 20, 21, 23, 24, 26, 27, 29, 30, 32, 33, 35, 36, 38, 39, 41, 42,
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
        pocket_floor: Some(parse_evidence(28)?),
        cut_west: Some(parse_evidence(31)?),
        cut_east: Some(parse_evidence(34)?),
        cut_south: Some(parse_evidence(37)?),
        cut_north: Some(parse_evidence(40)?),
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
