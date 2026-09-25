#![forbid(unsafe_code)]

pub mod assistant;
pub mod exact_worker;
pub mod general;
pub mod pair_query;
pub mod plugin;
pub mod response_transport;
pub use pair_query::{ExactPairCandidate, ExactPairQueryResult, ExactPairRelation};
use response_transport::read_worker_response;
pub mod validator_runtime;

use ketchup_core::cam::{
    CAM_SIMULATION_SCHEMA_V1, CamCollisionEvidence, CamCollisionParticipant, CamCollisionTarget,
    CamFixture, CamMotionKind, CamMotionPath, CamPlan, CamSimulationEvidence, CamToolpath,
};
use ketchup_core::document::{
    BodyKind, DerivedIdentity, GroupId, InstancePath, InstancePathStep, LocalGroupKey,
    LocalOccurrenceKey, NodeId, SceneOccurrence, SlotPath, SlotSegment, Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V6, EXACT_BREP_GRAPH_SCHEMA_V7, EXACT_BREP_GRAPH_SCHEMA_V8,
    EXACT_BREP_GRAPH_SCHEMA_V9, EXACT_BREP_GRAPH_SCHEMA_V10, EXACT_BREP_GRAPH_SCHEMA_V11,
    EXACT_BREP_GRAPH_SCHEMA_V12, EXACT_BREP_GRAPH_SCHEMA_V13, EXACT_BREP_GRAPH_SCHEMA_V14,
    EXACT_BREP_GRAPH_SCHEMA_V15, EXACT_BREP_GRAPH_SCHEMA_V16, EXACT_BREP_GRAPH_SCHEMA_V17,
    EXACT_BREP_GRAPH_SCHEMA_V18, EXACT_BREP_GRAPH_SCHEMA_V19, EXACT_BREP_GRAPH_SCHEMA_V20,
    EXACT_BREP_GRAPH_SCHEMA_V21, EXACT_BREP_GRAPH_SCHEMA_V22, EXACT_BREP_GRAPH_SCHEMA_V23,
    ExactBRepGraph, ExactBRepOperation,
};
use ketchup_core::exact_product::{
    ExactBRepGraphEdgeEvidence, ExactBRepGraphFaceEvidence, ExactBRepGraphPackage,
    ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactProductError,
};
use ketchup_core::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaElement, FeaElementKind, FeaLoad, FeaMaterial, FeaModel,
    FeaNode,
};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{
    IgesImportEvidence, IgesXdeImportEvidence, IgesXdeNodeEvidence, IgesXdePartEvidence,
    ImportLengthUnit, MAX_STEP_MESH_TRIANGLES, MAX_STEP_MESH_VERTICES, MAX_STEP_SOURCE_BYTES,
    STEP_MESH_MAGIC, StepImportEvidence, StepImportMesh, StepXdeImportEvidence,
    StepXdeNodeEvidence, StepXdePartEvidence,
};
use ketchup_exact::GeometryErrorCode;
#[cfg(windows)]
use process_wrap::std::JobObject;
use process_wrap::std::{ChildWrapper, CommandWrap};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkerExactBRepGraphFaceEvidence {
    pub semantic_role: String,
    pub source_element_id: String,
    pub face_ordinal: u32,
    pub surface_kind: String,
    pub geometric_fingerprint: String,
    pub centroid_mm: [f64; 3],
    pub unit_normal: [f64; 3],
    pub axis_origin_mm: Option<[f64; 3]>,
    pub unit_axis_direction: Option<[f64; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkerExactBRepGraphEdgeEvidence {
    pub edge_ordinal: u32,
    pub curve_kind: String,
    pub length_mm: f64,
    pub centroid_mm: [f64; 3],
    pub bounds_mm: [[f64; 3]; 2],
    pub closed: bool,
    pub circle_radius_mm: Option<f64>,
    pub axis_origin_mm: Option<[f64; 3]>,
    pub unit_axis_direction: Option<[f64; 3]>,
    pub adjacent_face_ordinals: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerExactBRepGraphResult {
    pub canonical_input_digest: String,
    pub graph_digest: String,
    pub producer_feature_id: u64,
    pub result_fingerprint: String,
    pub exact_input_digest: String,
    pub volume_mm3: f64,
    pub area_mm2: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
    pub wire_count: Option<u32>,
    pub backend: String,
    pub tolerance: String,
    pub faces: Vec<WorkerExactBRepGraphFaceEvidence>,
    pub edges: Vec<WorkerExactBRepGraphEdgeEvidence>,
}

pub const EXACT_VOLUME_MESH_WIRE_SCHEMA_V1: &str = "ketchup.exact-volume-mesh-wire.v1";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactVolumeMeshWireOptions {
    pub surface_deflection_mm: f64,
    pub angular_deflection_rad: f64,
    pub max_tetrahedra: u32,
    pub max_relative_volume_error: f64,
    pub min_tetrahedron_quality: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExactVolumeBoundaryTriangle {
    pub vertex_indices: [u32; 3],
    pub face_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExactVolumeMesh {
    pub schema: String,
    pub graph_digest: String,
    pub source_result_fingerprint: String,
    pub request_digest: String,
    pub mesh_fingerprint: String,
    pub vertices_mm: Vec<[f64; 3]>,
    pub tetrahedra: Vec<[u32; 4]>,
    pub boundary_triangles: Vec<WorkerExactVolumeBoundaryTriangle>,
    pub exact_volume_mm3: f64,
    pub tetrahedral_volume_mm3: f64,
    pub relative_volume_error: f64,
    pub minimum_signed_volume_mm3: f64,
    pub minimum_quality: f64,
    pub maximum_edge_ratio: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBRepVolumeMeshPackage {
    pub source: ExactBRepGraphPackage,
    pub options: ExactVolumeMeshWireOptions,
    pub mesh: WorkerExactVolumeMesh,
}

impl ExactBRepVolumeMeshPackage {
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.source.is_current(snapshot)
            && self.mesh.graph_digest == self.source.graph.graph_digest
            && self.mesh.source_result_fingerprint == self.source.identity.result_fingerprint
    }

    pub fn occurrence_bound_model(
        &self,
        snapshot: &Snapshot,
        setup: &ExactFeaSetup,
    ) -> Result<OccurrenceBoundFeaModel, ExactFeaSetupError> {
        if !self.is_current(snapshot) {
            return Err(ExactFeaSetupError::StaleGeometry);
        }
        if snapshot
            .resolve_instance_path(&setup.instance_path)
            .map_err(|_| ExactFeaSetupError::OccurrenceMismatch)?
            .definition_id
            != self.source.identity.definition_id
        {
            return Err(ExactFeaSetupError::OccurrenceMismatch);
        }
        if setup.case_id.trim().is_empty()
            || setup.constrained_face_ordinals.is_empty()
            || setup.face_tractions.is_empty()
        {
            return Err(ExactFeaSetupError::IncompleteSetup);
        }
        let boundary_face_ordinals = self
            .mesh
            .boundary_triangles
            .iter()
            .map(|triangle| triangle.face_ordinal)
            .collect::<BTreeSet<_>>();
        if setup
            .constrained_face_ordinals
            .iter()
            .any(|ordinal| !boundary_face_ordinals.contains(ordinal))
            || setup
                .face_tractions
                .iter()
                .any(|traction| !boundary_face_ordinals.contains(&traction.face_ordinal))
            || setup
                .face_tractions
                .iter()
                .flat_map(|traction| traction.traction_local_n_per_mm2)
                .any(|value| !value.is_finite())
        {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let constrained_faces = setup
            .constrained_face_ordinals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if constrained_faces.len() != setup.constrained_face_ordinals.len() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let traction_faces = setup
            .face_tractions
            .iter()
            .map(|traction| traction.face_ordinal)
            .collect::<BTreeSet<_>>();
        if traction_faces.len() != setup.face_tractions.len() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }

        let constrained_nodes = self
            .mesh
            .boundary_triangles
            .iter()
            .filter(|triangle| constrained_faces.contains(&triangle.face_ordinal))
            .flat_map(|triangle| triangle.vertex_indices)
            .collect::<BTreeSet<_>>();
        if constrained_nodes.is_empty() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let loads = setup
            .face_tractions
            .iter()
            .flat_map(|traction| {
                self.mesh
                    .boundary_triangles
                    .iter()
                    .filter(move |triangle| triangle.face_ordinal == traction.face_ordinal)
                    .map(move |triangle| FeaLoad::SurfaceTraction {
                        nodes: triangle.vertex_indices.map(|index| index as usize),
                        traction_n_per_mm2: traction.traction_local_n_per_mm2,
                    })
            })
            .collect::<Vec<_>>();
        if loads.is_empty() {
            return Err(ExactFeaSetupError::InvalidBoundarySelection);
        }
        let model = FeaModel {
            schema: FEA_MODEL_SCHEMA_V1.to_owned(),
            case_id: setup.case_id.clone(),
            nodes: self
                .mesh
                .vertices_mm
                .iter()
                .copied()
                .map(|position_mm| FeaNode { position_mm })
                .collect(),
            materials: vec![setup.material.clone()],
            elements: self
                .mesh
                .tetrahedra
                .iter()
                .enumerate()
                .map(|(index, nodes)| FeaElement {
                    id: index as u64 + 1,
                    kind: FeaElementKind::LinearTetrahedron4 {
                        nodes: nodes.map(|node| node as usize),
                        material_id: setup.material.id,
                    },
                })
                .collect(),
            constraints: constrained_nodes
                .into_iter()
                .map(|node| FeaConstraint {
                    node: node as usize,
                    displacement_mm: [Some(0.0); 3],
                })
                .collect(),
            loads,
        };
        Ok(OccurrenceBoundFeaModel {
            instance_path: setup.instance_path.clone(),
            source_revision: self.source.identity.source_revision,
            source_digest: self.source.identity.source_digest.clone(),
            graph_digest: self.mesh.graph_digest.clone(),
            mesh_fingerprint: self.mesh.mesh_fingerprint.clone(),
            model,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaFaceTraction {
    pub face_ordinal: u32,
    pub traction_local_n_per_mm2: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactFeaSetup {
    pub case_id: String,
    pub instance_path: InstancePath,
    pub material: FeaMaterial,
    pub constrained_face_ordinals: Vec<u32>,
    pub face_tractions: Vec<ExactFeaFaceTraction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceBoundFeaModel {
    pub instance_path: InstancePath,
    pub source_revision: u64,
    pub source_digest: String,
    pub graph_digest: String,
    pub mesh_fingerprint: String,
    pub model: FeaModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactFeaSetupError {
    StaleGeometry,
    OccurrenceMismatch,
    IncompleteSetup,
    InvalidBoundarySelection,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireRequest {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub target_exact_graph_digest: String,
    pub stock_bounds_mm: [[f64; 3]; 2],
    pub setup_to_world: [f64; 16],
    pub cutter_radius_mm: f64,
    pub cutter_length_mm: f64,
    pub tool_length_mm: f64,
    pub holder_radius_mm: f64,
    pub holder_offset_mm: f64,
    pub holder_length_mm: f64,
    pub motions: Vec<CamSimulationWireMotion>,
    pub fixtures: Vec<CamSimulationWireFixture>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireMotion {
    pub kind: u8,
    pub path_kind: u8,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub center_mm: [f64; 3],
    pub clockwise: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireFixture {
    pub id: u64,
    pub bounds_mm: [[f64; 3]; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireEvidence {
    pub schema: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub target_exact_graph_digest: String,
    pub motion_count: usize,
    pub cutting_motion_count: usize,
    pub fixture_count: usize,
    pub stock_before_mm3: f64,
    pub stock_after_mm3: f64,
    pub removed_stock_mm3: f64,
    pub residual_stock_mm3: f64,
    pub gouge_mm3: f64,
    pub collisions: Vec<CamSimulationWireCollision>,
    pub backend: String,
    pub tolerance: String,
    pub result_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CamSimulationWireCollision {
    pub motion_index: usize,
    pub motion_kind: u8,
    pub participant: u8,
    pub target: u8,
    pub fixture_id: Option<u64>,
    pub common_volume_mm3: f64,
    pub contact_area_mm2: f64,
    pub distance_mm: f64,
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

const M21_STEP_MODEL_CAPABILITY: &str = "M21_STEP_MODEL_V1";
const M21_STEP_XDE_CAPABILITY: &str = "M21_STEP_XDE_V1";
const M21_IGES_CAPABILITY: &str = "M21_IGES_V1";
const EXACT_BREP_GRAPH_CAPABILITY_V6: &str = "EXACT_BREP_GRAPH_V6";
const EXACT_BREP_GRAPH_CAPABILITY_V7: &str = "EXACT_BREP_GRAPH_V7";
const EXACT_BREP_GRAPH_CAPABILITY_V8: &str = "EXACT_BREP_GRAPH_V8";
const EXACT_BREP_GRAPH_CAPABILITY_V9: &str = "EXACT_BREP_GRAPH_V9";
const EXACT_BREP_GRAPH_CAPABILITY_V10: &str = "EXACT_BREP_GRAPH_V10";
const EXACT_BREP_GRAPH_CAPABILITY_V11: &str = "EXACT_BREP_GRAPH_V11";
const EXACT_BREP_GRAPH_CAPABILITY_V12: &str = "EXACT_BREP_GRAPH_V12";
const EXACT_BREP_GRAPH_CAPABILITY_V13: &str = "EXACT_BREP_GRAPH_V13";
const EXACT_BREP_GRAPH_CAPABILITY_V14: &str = "EXACT_BREP_GRAPH_V14";
const EXACT_BREP_GRAPH_CAPABILITY_V15: &str = "EXACT_BREP_GRAPH_V15";
const EXACT_BREP_GRAPH_CAPABILITY_V16: &str = "EXACT_BREP_GRAPH_V16";
const EXACT_BREP_GRAPH_CAPABILITY_V17: &str = "EXACT_BREP_GRAPH_V17";
const EXACT_BREP_GRAPH_CAPABILITY_V18: &str = "EXACT_BREP_GRAPH_V18";
const EXACT_BREP_GRAPH_CAPABILITY_V19: &str = "EXACT_BREP_GRAPH_V19";
const EXACT_BREP_GRAPH_CAPABILITY_V20: &str = "EXACT_BREP_GRAPH_V20";
const EXACT_BREP_GRAPH_CAPABILITY_V21: &str = "EXACT_BREP_GRAPH_V21";
const EXACT_BREP_GRAPH_CAPABILITY_V22: &str = "EXACT_BREP_GRAPH_V22";
const EXACT_BREP_GRAPH_CAPABILITY_V23: &str = "EXACT_BREP_GRAPH_V23";
const CAM_SIMULATION_CAPABILITY_V1: &str = "CAM_SIMULATION_V1";
const MAX_CAM_SIMULATION_MOTIONS: usize = 4_096;
const MAX_CAM_SIMULATION_FIXTURES: usize = 64;
const MAX_CAM_SIMULATION_PAIR_CHECKS: usize = 16_384;
pub const MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES: usize = 64;
pub const MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_WORKER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
// General graph operations may rebuild many sequential exact Boolean features.
// Keep this finite and separate from handshake/simple-call latency limits.
const EXACT_BREP_GRAPH_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepXdeWorkerEvidence {
    pub source_sha256: String,
    pub source_byte_len: u64,
    pub source_unit: String,
    pub parts: Vec<StepXdeWorkerPart>,
    pub nodes: Vec<StepXdeWorkerNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepXdeWorkerPart {
    pub index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub result_fingerprint: String,
    pub body_kind: String,
    pub solid_count: u32,
    pub topology_counts: [u32; 5],
    pub area_mm2: f64,
    pub volume_mm3: f64,
    pub bounds_mm: [[f64; 3]; 2],
    pub backend: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepXdeWorkerNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepAssemblyManifest {
    pub schema: String,
    pub document_id: u64,
    pub source_revision: u64,
    pub source_digest: String,
    pub parts: Vec<StepAssemblyPart>,
    pub nodes: Vec<StepAssemblyNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepAssemblyPart {
    pub document_id: u64,
    pub source_revision: u64,
    pub source_digest: String,
    pub definition_id: u64,
    pub producer_feature_id: u64,
    pub name: String,
    pub expected_result_fingerprint: String,
    pub imported_result_fingerprint: String,
    pub source_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepAssemblyNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub color: Option<[u8; 3]>,
    pub transform_bits: [u64; 16],
}

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_WORKER_RESPONSE_LINE_BYTES: usize = 64 * 1024;
const MAX_EXACT_WORKER_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXACT_VOLUME_MESH_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);

fn read_bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
    limit_message: &str,
) -> Result<Vec<u8>, WorkerError> {
    let file = fs::File::open(path).map_err(|error| WorkerError::Transport(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| WorkerError::Transport(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        return Err(WorkerError::Transport(limit_message.to_owned()));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| WorkerError::Transport(error.to_string()))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(WorkerError::Transport(limit_message.to_owned()));
    }
    Ok(bytes)
}

fn read_step_import_mesh_output(
    output_path: &Path,
    vertex_count: u32,
    triangle_count: u32,
) -> Result<Vec<u8>, WorkerError> {
    if vertex_count == 0
        || triangle_count == 0
        || vertex_count > MAX_STEP_MESH_VERTICES
        || triangle_count > MAX_STEP_MESH_TRIANGLES
    {
        return Err(WorkerError::Protocol(
            "exact worker mesh receipt exceeds its bounded envelope".to_owned(),
        ));
    }
    let expected_bytes = STEP_MESH_MAGIC.len() as u64
        + 8
        + u64::from(vertex_count) * 24
        + u64::from(triangle_count) * 16;
    let file =
        fs::File::open(output_path).map_err(|error| WorkerError::Transport(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| WorkerError::Transport(error.to_string()))?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(WorkerError::Transport(
            "exact worker mesh output does not match its bounded receipt".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(expected_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| WorkerError::Transport(error.to_string()))?;
    if bytes.len() as u64 != expected_bytes {
        return Err(WorkerError::Transport(
            "exact worker mesh output does not match its bounded receipt".to_owned(),
        ));
    }
    Ok(bytes)
}

fn open_guarded_exact_worker(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

struct GuardedExactWorker {
    executable: PathBuf,
    sha256: String,
    _file: fs::File,
}

fn guarded_exact_worker_identity(executable: &Path) -> Result<GuardedExactWorker, WorkerError> {
    if !executable.is_absolute() {
        return Err(WorkerError::Spawn(
            "exact worker executable must be absolute".to_owned(),
        ));
    }
    let executable = executable
        .canonicalize()
        .map_err(|error| WorkerError::Spawn(error.to_string()))?;
    let file = open_guarded_exact_worker(&executable)
        .map_err(|error| WorkerError::Spawn(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| WorkerError::Spawn(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_EXACT_WORKER_EXECUTABLE_BYTES {
        return Err(WorkerError::Spawn(
            "exact worker executable is not a bounded file".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_EXACT_WORKER_EXECUTABLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| WorkerError::Spawn(error.to_string()))?;
    if bytes.len() as u64 > MAX_EXACT_WORKER_EXECUTABLE_BYTES {
        return Err(WorkerError::Spawn(
            "exact worker executable is not a bounded file".to_owned(),
        ));
    }
    Ok(GuardedExactWorker {
        executable,
        sha256: sha256_hex(&bytes),
        _file: file,
    })
}

fn exact_worker_identity(executable: &Path) -> Result<(PathBuf, String), WorkerError> {
    let guarded = guarded_exact_worker_identity(executable)?;
    Ok((guarded.executable, guarded.sha256))
}

fn verify_exact_worker_identity(
    executable: &Path,
    expected_sha256: &str,
) -> Result<GuardedExactWorker, WorkerError> {
    let guarded = guarded_exact_worker_identity(executable)?;
    if guarded.sha256 != expected_sha256 {
        return Err(WorkerError::Spawn(
            "exact worker executable identity changed".to_owned(),
        ));
    }
    Ok(guarded)
}

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

fn spawn_exact_worker_command(command: Command) -> io::Result<Box<dyn ChildWrapper>> {
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    command.wrap(JobObject);
    command.spawn()
}

pub struct ExactWorkerClient {
    child: Box<dyn ChildWrapper>,
    write_sender: Sender<WorkerWriteRequest>,
    response_receiver: Receiver<WorkerResponse>,
    _temp_directory: tempfile::TempDir,
}

impl ExactWorkerClient {
    fn read_bounded_output(
        &mut self,
        path: &Path,
        maximum_bytes: u64,
        limit_message: &str,
    ) -> Result<Vec<u8>, WorkerError> {
        match read_bounded_regular_file(path, maximum_bytes, limit_message) {
            Ok(bytes) => Ok(bytes),
            Err(error) => self.fail(error),
        }
    }

    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        let executable = guarded_exact_worker_identity(executable.as_ref())?;
        Self::spawn_guarded(executable)
    }

    fn spawn_guarded(executable: GuardedExactWorker) -> Result<Self, WorkerError> {
        let working_directory = executable
            .executable
            .parent()
            .expect("canonical exact worker executable has a parent");
        let temp_directory =
            tempfile::tempdir().map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let mut command = Command::new(&executable.executable);
        command
            .current_dir(working_directory)
            .env_clear()
            .env("TEMP", temp_directory.path())
            .env("TMP", temp_directory.path())
            .env("TMPDIR", temp_directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = spawn_exact_worker_command(command)
            .map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let stdin = child
            .stdin()
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdout was not piped".to_owned()))?;
        let (write_sender, write_receiver) = mpsc::channel();
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        spawn_worker_writer(stdin, write_receiver);
        spawn_worker_reader(stdout, response_sender);
        Ok(Self {
            child,
            write_sender,
            response_receiver,
            _temp_directory: temp_directory,
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

    fn verify_capability(
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

    fn verify_m21_step_xde_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M21_STEP_XDE_V1", cancelled)?;
        if response == "CAPS M21_STEP_XDE_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M21_STEP_XDE_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_m21_iges_capability(&mut self, cancelled: &AtomicBool) -> Result<(), WorkerError> {
        let response = self.request_with_cancellation("CAPS M21_IGES_V1", cancelled)?;
        if response == "CAPS M21_IGES_V1" {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                M21_IGES_CAPABILITY.to_owned(),
            ))
        }
    }

    fn verify_exact_brep_graph_capability(
        &mut self,
        graph: &ExactBRepGraph,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let capability = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6 => EXACT_BREP_GRAPH_CAPABILITY_V6,
            EXACT_BREP_GRAPH_SCHEMA_V7 => EXACT_BREP_GRAPH_CAPABILITY_V7,
            EXACT_BREP_GRAPH_SCHEMA_V8 => EXACT_BREP_GRAPH_CAPABILITY_V8,
            EXACT_BREP_GRAPH_SCHEMA_V9 => EXACT_BREP_GRAPH_CAPABILITY_V9,
            EXACT_BREP_GRAPH_SCHEMA_V10 => EXACT_BREP_GRAPH_CAPABILITY_V10,
            EXACT_BREP_GRAPH_SCHEMA_V11 => EXACT_BREP_GRAPH_CAPABILITY_V11,
            EXACT_BREP_GRAPH_SCHEMA_V12 => EXACT_BREP_GRAPH_CAPABILITY_V12,
            EXACT_BREP_GRAPH_SCHEMA_V13 => EXACT_BREP_GRAPH_CAPABILITY_V13,
            EXACT_BREP_GRAPH_SCHEMA_V14 => EXACT_BREP_GRAPH_CAPABILITY_V14,
            EXACT_BREP_GRAPH_SCHEMA_V15 => EXACT_BREP_GRAPH_CAPABILITY_V15,
            EXACT_BREP_GRAPH_SCHEMA_V16 => EXACT_BREP_GRAPH_CAPABILITY_V16,
            EXACT_BREP_GRAPH_SCHEMA_V17 => EXACT_BREP_GRAPH_CAPABILITY_V17,
            EXACT_BREP_GRAPH_SCHEMA_V18 => EXACT_BREP_GRAPH_CAPABILITY_V18,
            EXACT_BREP_GRAPH_SCHEMA_V19 => EXACT_BREP_GRAPH_CAPABILITY_V19,
            EXACT_BREP_GRAPH_SCHEMA_V20 => EXACT_BREP_GRAPH_CAPABILITY_V20,
            EXACT_BREP_GRAPH_SCHEMA_V21 => EXACT_BREP_GRAPH_CAPABILITY_V21,
            EXACT_BREP_GRAPH_SCHEMA_V22 => EXACT_BREP_GRAPH_CAPABILITY_V22,
            EXACT_BREP_GRAPH_SCHEMA_V23 => EXACT_BREP_GRAPH_CAPABILITY_V23,
            schema => {
                return Err(WorkerError::Protocol(format!(
                    "unsupported graph schema {schema}"
                )));
            }
        };
        let request = format!("CAPS {capability}");
        let response = self.request_with_cancellation(&request, cancelled)?;
        if response == request {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(capability.to_owned()))
        }
    }

    fn verify_cam_simulation_capability(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let request = format!("CAPS {CAM_SIMULATION_CAPABILITY_V1}");
        let response = self.request_with_cancellation(&request, cancelled)?;
        if response == request {
            Ok(())
        } else {
            self.terminate_worker();
            Err(WorkerError::MissingCapability(
                CAM_SIMULATION_CAPABILITY_V1.to_owned(),
            ))
        }
    }

    fn simulate_cam_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        request: &CamSimulationWireRequest,
        cancelled: &AtomicBool,
    ) -> Result<CamSimulationWireEvidence, WorkerError> {
        self.verify_cam_simulation_capability(cancelled)?;
        self.verify_exact_brep_graph_capability(graph, cancelled)?;
        let graph_bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let request_bytes = serde_json::to_vec(request)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let response = self.request_with_timeout(
            &format!(
                "SIMULATE_CAM_V1 {} {} {}",
                graph.graph_digest,
                hex_encode(&graph_bytes),
                hex_encode(&request_bytes)
            ),
            cancelled,
            EXACT_BREP_GRAPH_REQUEST_TIMEOUT,
        )?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return Err(parse_error_response(&response, &fields));
        }
        if fields.len() != 3
            || fields[0] != "OK_CAM_SIMULATION_V1"
            || fields[1] != graph.graph_digest
        {
            return self.fail_protocol(response);
        }
        let encoded = hex_decode_utf8(fields[2])
            .ok_or_else(|| WorkerError::Protocol("CAM evidence is not hexadecimal UTF-8".into()))?;
        serde_json::from_str(&encoded).map_err(|_| WorkerError::Protocol(response))
    }

    fn evaluate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactBRepGraphResult, WorkerError> {
        self.verify_exact_brep_graph_capability(graph, cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let operation = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6 => "EVAL_BREP_GRAPH_V6",
            EXACT_BREP_GRAPH_SCHEMA_V7 => "EVAL_BREP_GRAPH_V7",
            EXACT_BREP_GRAPH_SCHEMA_V8 => "EVAL_BREP_GRAPH_V8",
            EXACT_BREP_GRAPH_SCHEMA_V9 => "EVAL_BREP_GRAPH_V9",
            EXACT_BREP_GRAPH_SCHEMA_V10 => "EVAL_BREP_GRAPH_V10",
            EXACT_BREP_GRAPH_SCHEMA_V11 => "EVAL_BREP_GRAPH_V11",
            EXACT_BREP_GRAPH_SCHEMA_V12 => "EVAL_BREP_GRAPH_V12",
            EXACT_BREP_GRAPH_SCHEMA_V13 => "EVAL_BREP_GRAPH_V13",
            EXACT_BREP_GRAPH_SCHEMA_V14 => "EVAL_BREP_GRAPH_V14",
            EXACT_BREP_GRAPH_SCHEMA_V15 => "EVAL_BREP_GRAPH_V15",
            EXACT_BREP_GRAPH_SCHEMA_V16 => "EVAL_BREP_GRAPH_V16",
            EXACT_BREP_GRAPH_SCHEMA_V17 => "EVAL_BREP_GRAPH_V17",
            EXACT_BREP_GRAPH_SCHEMA_V18 => "EVAL_BREP_GRAPH_V18",
            EXACT_BREP_GRAPH_SCHEMA_V19 => "EVAL_BREP_GRAPH_V19",
            EXACT_BREP_GRAPH_SCHEMA_V20 => "EVAL_BREP_GRAPH_V20",
            EXACT_BREP_GRAPH_SCHEMA_V21 => "EVAL_BREP_GRAPH_V21",
            EXACT_BREP_GRAPH_SCHEMA_V22 => "EVAL_BREP_GRAPH_V22",
            EXACT_BREP_GRAPH_SCHEMA_V23 => "EVAL_BREP_GRAPH_V23",
            _ => unreachable!("capability validation rejects unsupported graph schemas"),
        };
        let mut request = format!("{operation} {} {}", graph.graph_digest, hex_encode(&bytes));
        append_exact_brep_graph_sources(&mut request, imported_sources);
        let response =
            self.request_with_timeout(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)?;
        let expected_protocol = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6 | EXACT_BREP_GRAPH_SCHEMA_V7 => "OK_BREP_GRAPH_V6",
            EXACT_BREP_GRAPH_SCHEMA_V8 => "OK_BREP_GRAPH_V8",
            EXACT_BREP_GRAPH_SCHEMA_V9 => "OK_BREP_GRAPH_V9",
            EXACT_BREP_GRAPH_SCHEMA_V10 => "OK_BREP_GRAPH_V10",
            EXACT_BREP_GRAPH_SCHEMA_V11 => "OK_BREP_GRAPH_V11",
            EXACT_BREP_GRAPH_SCHEMA_V12 => "OK_BREP_GRAPH_V12",
            EXACT_BREP_GRAPH_SCHEMA_V13 => "OK_BREP_GRAPH_V13",
            EXACT_BREP_GRAPH_SCHEMA_V14 => "OK_BREP_GRAPH_V14",
            EXACT_BREP_GRAPH_SCHEMA_V15 => "OK_BREP_GRAPH_V15",
            EXACT_BREP_GRAPH_SCHEMA_V16 => "OK_BREP_GRAPH_V16",
            EXACT_BREP_GRAPH_SCHEMA_V17 => "OK_BREP_GRAPH_V17",
            EXACT_BREP_GRAPH_SCHEMA_V18 => "OK_BREP_GRAPH_V18",
            EXACT_BREP_GRAPH_SCHEMA_V19 => "OK_BREP_GRAPH_V19",
            EXACT_BREP_GRAPH_SCHEMA_V20 => "OK_BREP_GRAPH_V20",
            EXACT_BREP_GRAPH_SCHEMA_V21 => "OK_BREP_GRAPH_V21",
            EXACT_BREP_GRAPH_SCHEMA_V22 => "OK_BREP_GRAPH_V22",
            EXACT_BREP_GRAPH_SCHEMA_V23 => "OK_BREP_GRAPH_V23",
            _ => unreachable!("capability validation rejects unsupported graph schemas"),
        };
        match parse_exact_brep_graph_result(&response, expected_protocol) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn tessellate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        self.verify_exact_brep_graph_capability(graph, cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let operation = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6 => "TESSELLATE_BREP_GRAPH_V6",
            EXACT_BREP_GRAPH_SCHEMA_V7 => "TESSELLATE_BREP_GRAPH_V7",
            EXACT_BREP_GRAPH_SCHEMA_V8 => "TESSELLATE_BREP_GRAPH_V8",
            EXACT_BREP_GRAPH_SCHEMA_V9 => "TESSELLATE_BREP_GRAPH_V9",
            EXACT_BREP_GRAPH_SCHEMA_V10 => "TESSELLATE_BREP_GRAPH_V10",
            EXACT_BREP_GRAPH_SCHEMA_V11 => "TESSELLATE_BREP_GRAPH_V11",
            EXACT_BREP_GRAPH_SCHEMA_V12 => "TESSELLATE_BREP_GRAPH_V12",
            EXACT_BREP_GRAPH_SCHEMA_V13 => "TESSELLATE_BREP_GRAPH_V13",
            EXACT_BREP_GRAPH_SCHEMA_V14 => "TESSELLATE_BREP_GRAPH_V14",
            EXACT_BREP_GRAPH_SCHEMA_V15 => "TESSELLATE_BREP_GRAPH_V15",
            EXACT_BREP_GRAPH_SCHEMA_V16 => "TESSELLATE_BREP_GRAPH_V16",
            EXACT_BREP_GRAPH_SCHEMA_V17 => "TESSELLATE_BREP_GRAPH_V17",
            EXACT_BREP_GRAPH_SCHEMA_V18 => "TESSELLATE_BREP_GRAPH_V18",
            EXACT_BREP_GRAPH_SCHEMA_V19 => "TESSELLATE_BREP_GRAPH_V19",
            EXACT_BREP_GRAPH_SCHEMA_V20 => "TESSELLATE_BREP_GRAPH_V20",
            EXACT_BREP_GRAPH_SCHEMA_V21 => "TESSELLATE_BREP_GRAPH_V21",
            EXACT_BREP_GRAPH_SCHEMA_V22 => "TESSELLATE_BREP_GRAPH_V22",
            EXACT_BREP_GRAPH_SCHEMA_V23 => "TESSELLATE_BREP_GRAPH_V23",
            _ => unreachable!("capability validation rejects unsupported graph schemas"),
        };
        let mut request = format!(
            "{operation} {} {} {} {}",
            graph.graph_digest,
            hex_encode(&bytes),
            result_fingerprint,
            hex_encode(output_path.to_string_lossy().as_bytes())
        );
        append_exact_brep_graph_sources(&mut request, imported_sources);
        let response =
            self.request_with_timeout(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)?;
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
        let encoded = read_step_import_mesh_output(output_path, vertex_count, triangle_count)?;
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

    fn volume_mesh_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        options: ExactVolumeMeshWireOptions,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactVolumeMesh, WorkerError> {
        self.verify_exact_brep_graph_capability(graph, cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let operation = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6 => "VOLUME_MESH_BREP_GRAPH_V6",
            EXACT_BREP_GRAPH_SCHEMA_V7 => "VOLUME_MESH_BREP_GRAPH_V7",
            EXACT_BREP_GRAPH_SCHEMA_V8 => "VOLUME_MESH_BREP_GRAPH_V8",
            EXACT_BREP_GRAPH_SCHEMA_V9 => "VOLUME_MESH_BREP_GRAPH_V9",
            EXACT_BREP_GRAPH_SCHEMA_V10 => "VOLUME_MESH_BREP_GRAPH_V10",
            EXACT_BREP_GRAPH_SCHEMA_V11 => "VOLUME_MESH_BREP_GRAPH_V11",
            EXACT_BREP_GRAPH_SCHEMA_V12 => "VOLUME_MESH_BREP_GRAPH_V12",
            EXACT_BREP_GRAPH_SCHEMA_V13 => "VOLUME_MESH_BREP_GRAPH_V13",
            EXACT_BREP_GRAPH_SCHEMA_V14 => "VOLUME_MESH_BREP_GRAPH_V14",
            EXACT_BREP_GRAPH_SCHEMA_V15 => "VOLUME_MESH_BREP_GRAPH_V15",
            EXACT_BREP_GRAPH_SCHEMA_V16 => "VOLUME_MESH_BREP_GRAPH_V16",
            EXACT_BREP_GRAPH_SCHEMA_V17 => "VOLUME_MESH_BREP_GRAPH_V17",
            EXACT_BREP_GRAPH_SCHEMA_V18 => "VOLUME_MESH_BREP_GRAPH_V18",
            EXACT_BREP_GRAPH_SCHEMA_V19 => "VOLUME_MESH_BREP_GRAPH_V19",
            EXACT_BREP_GRAPH_SCHEMA_V20 => "VOLUME_MESH_BREP_GRAPH_V20",
            EXACT_BREP_GRAPH_SCHEMA_V21 => "VOLUME_MESH_BREP_GRAPH_V21",
            EXACT_BREP_GRAPH_SCHEMA_V22 => "VOLUME_MESH_BREP_GRAPH_V22",
            EXACT_BREP_GRAPH_SCHEMA_V23 => "VOLUME_MESH_BREP_GRAPH_V23",
            _ => unreachable!("capability validation rejects unsupported graph schemas"),
        };
        let max_tetrahedra = options.max_tetrahedra;
        let options = serde_json::to_vec(&options)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let mut request = format!(
            "{operation} {} {} {} {} {}",
            graph.graph_digest,
            hex_encode(&bytes),
            result_fingerprint,
            hex_encode(output_path.to_string_lossy().as_bytes()),
            hex_encode(&options),
        );
        append_exact_brep_graph_sources(&mut request, imported_sources);
        let response =
            self.request_with_timeout(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)?;
        let fields = response.split_whitespace().collect::<Vec<_>>();
        if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
            return Err(parse_error_response(&response, &fields));
        }
        if fields.len() != 8
            || fields[0] != "OK_BREP_GRAPH_VOLUME_MESH_V1"
            || fields[1] != graph.graph_digest
            || fields[2] != result_fingerprint
            || !is_sha256_digest(fields[6])
        {
            return self.fail_protocol(response);
        }
        let (Ok(vertex_count), Ok(tetrahedron_count), Ok(boundary_count)) = (
            fields[3].parse::<u32>(),
            fields[4].parse::<u32>(),
            fields[5].parse::<u32>(),
        ) else {
            return self.fail_protocol(response);
        };
        let maximum_vertices = max_tetrahedra.saturating_mul(4);
        let maximum_boundary_triangles = max_tetrahedra.saturating_mul(4);
        if vertex_count < 4
            || tetrahedron_count == 0
            || tetrahedron_count > max_tetrahedra
            || boundary_count == 0
            || vertex_count > maximum_vertices
            || boundary_count > maximum_boundary_triangles
        {
            return self.fail_protocol(response);
        }
        let encoded = self.read_bounded_output(
            output_path,
            MAX_EXACT_VOLUME_MESH_OUTPUT_BYTES,
            "exact worker volume-mesh output exceeds the bounded 64 MiB envelope",
        )?;
        if sha256_hex(&encoded) != fields[6] {
            return Err(WorkerError::Transport(
                "exact volume-mesh digest does not match the worker receipt".to_owned(),
            ));
        }
        let mesh = serde_json::from_slice::<WorkerExactVolumeMesh>(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.schema != EXACT_VOLUME_MESH_WIRE_SCHEMA_V1
            || mesh.graph_digest != graph.graph_digest
            || mesh.source_result_fingerprint != result_fingerprint
            || mesh.mesh_fingerprint != fields[7]
            || mesh.vertices_mm.len() as u32 != vertex_count
            || mesh.tetrahedra.len() as u32 != tetrahedron_count
            || mesh.boundary_triangles.len() as u32 != boundary_count
        {
            return Err(WorkerError::Transport(
                "exact volume mesh does not match the worker receipt or request identity"
                    .to_owned(),
            ));
        }
        Ok(mesh)
    }

    fn export_exact_brep_graph_step_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_exact_brep_graph_capability(graph, cancelled)?;
        let bytes = graph
            .to_bytes()
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let (operation, expected_protocol) = match graph.schema.as_str() {
            EXACT_BREP_GRAPH_SCHEMA_V6
            | EXACT_BREP_GRAPH_SCHEMA_V7
            | EXACT_BREP_GRAPH_SCHEMA_V8
            | EXACT_BREP_GRAPH_SCHEMA_V9
            | EXACT_BREP_GRAPH_SCHEMA_V10
            | EXACT_BREP_GRAPH_SCHEMA_V11 => ("EXPORT_BREP_GRAPH_STEP_V2", "OK_BREP_GRAPH_STEP_V2"),
            EXACT_BREP_GRAPH_SCHEMA_V12 => ("EXPORT_BREP_GRAPH_STEP_V3", "OK_BREP_GRAPH_STEP_V3"),
            EXACT_BREP_GRAPH_SCHEMA_V13
            | EXACT_BREP_GRAPH_SCHEMA_V14
            | EXACT_BREP_GRAPH_SCHEMA_V15
            | EXACT_BREP_GRAPH_SCHEMA_V16
            | EXACT_BREP_GRAPH_SCHEMA_V17
            | EXACT_BREP_GRAPH_SCHEMA_V18
            | EXACT_BREP_GRAPH_SCHEMA_V19
            | EXACT_BREP_GRAPH_SCHEMA_V20
            | EXACT_BREP_GRAPH_SCHEMA_V21
            | EXACT_BREP_GRAPH_SCHEMA_V22
            | EXACT_BREP_GRAPH_SCHEMA_V23 => ("EXPORT_BREP_GRAPH_STEP_V4", "OK_BREP_GRAPH_STEP_V4"),
            _ => unreachable!("capability validation rejects unsupported graph schemas"),
        };
        let mut request = format!(
            "{operation} {} {} {} {}",
            graph.graph_digest,
            hex_encode(&bytes),
            result_fingerprint,
            hex_encode(output_path.to_string_lossy().as_bytes())
        );
        append_exact_brep_graph_sources(&mut request, imported_sources);
        let response =
            self.request_with_timeout(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)?;
        match parse_exact_brep_graph_step_acknowledgment(
            &response,
            expected_protocol,
            &graph.graph_digest,
            result_fingerprint,
        ) {
            Err(WorkerError::Protocol(response)) => self.fail_protocol(response),
            result => result,
        }
    }

    fn inspect_step_xde_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepXdeImportEvidence, WorkerError> {
        self.verify_m21_step_xde_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "INSPECT_STEP_XDE_M21_V1 {source_sha256} {}",
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
        if fields.len() != 4
            || fields[0] != "OK_M21_STEP_XDE_V2"
            || fields[1] != source_sha256
            || !is_sha256_digest(fields[2])
        {
            return self.fail_protocol(response);
        }
        let encoded = hex_decode_utf8(fields[3]).ok_or_else(|| {
            WorkerError::Protocol("STEP XDE evidence is not valid hexadecimal UTF-8".to_owned())
        })?;
        if sha256_hex(encoded.as_bytes()) != fields[2] {
            return Err(WorkerError::Transport(
                "STEP XDE evidence digest does not match the worker receipt".to_owned(),
            ));
        }
        let evidence: StepXdeWorkerEvidence = serde_json::from_str(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if evidence.source_sha256 != source_sha256
            || evidence
                .parts
                .iter()
                .any(|part| parse_import_body_kind(&part.body_kind).is_none())
        {
            return self.fail_protocol(response);
        }
        let source_sha256 = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid STEP source SHA-256".to_owned()))?;
        let source_byte_len = evidence.source_byte_len;
        let source_unit = match evidence.source_unit.as_str() {
            "millimetre" => ImportLengthUnit::Millimetre,
            "centimetre" => ImportLengthUnit::Centimetre,
            "metre" => ImportLengthUnit::Metre,
            "inch" => ImportLengthUnit::Inch,
            "foot" => ImportLengthUnit::Foot,
            _ => return self.fail_protocol(response),
        };
        let parts = evidence
            .parts
            .into_iter()
            .map(|part| StepXdePartEvidence {
                index: part.index,
                name: part.name,
                name_from_source: part.name_from_source,
                color: part.color,
                exact: StepImportEvidence {
                    source_sha256,
                    source_byte_len,
                    source_unit,
                    result_fingerprint: part.result_fingerprint,
                    body_kind: parse_import_body_kind(&part.body_kind)
                        .expect("worker body kind was validated"),
                    solid_count: part.solid_count,
                    topology_counts: part.topology_counts,
                    area_mm2: part.area_mm2,
                    volume_mm3: part.volume_mm3,
                    bounds_mm: part.bounds_mm,
                    backend: part.backend,
                    tolerance: part.tolerance,
                },
            })
            .collect();
        let nodes = evidence
            .nodes
            .into_iter()
            .map(|node| {
                let transform = Transform::from_matrix(node.transform)
                    .ok()
                    .filter(|transform| transform.rigid_inverse().is_some())?;
                Some(StepXdeNodeEvidence {
                    id: node.id,
                    parent_id: node.parent_id,
                    part_index: node.part_index,
                    name: node.name,
                    name_from_source: node.name_from_source,
                    color: node.color,
                    transform,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| WorkerError::Protocol("STEP XDE transform is not rigid".to_owned()))?;
        Ok(StepXdeImportEvidence {
            source_sha256,
            source_byte_len: evidence.source_byte_len,
            parts,
            nodes,
        })
    }

    fn inspect_iges_xde_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<IgesXdeImportEvidence, WorkerError> {
        self.verify_m21_iges_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "INSPECT_IGES_XDE_M21_V1 {source_sha256} {}",
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
        if fields.len() != 4
            || fields[0] != "OK_M21_IGES_XDE_V2"
            || fields[1] != source_sha256
            || !is_sha256_digest(fields[2])
        {
            return self.fail_protocol(response);
        }
        let encoded = hex_decode_utf8(fields[3]).ok_or_else(|| {
            WorkerError::Protocol("IGES XDE evidence is not valid hexadecimal UTF-8".to_owned())
        })?;
        if sha256_hex(encoded.as_bytes()) != fields[2] {
            return Err(WorkerError::Transport(
                "IGES XDE evidence digest does not match the worker receipt".to_owned(),
            ));
        }
        let evidence: StepXdeWorkerEvidence = serde_json::from_str(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if evidence.source_sha256 != source_sha256
            || evidence
                .parts
                .iter()
                .any(|part| parse_import_body_kind(&part.body_kind).is_none())
        {
            return self.fail_protocol(response);
        }
        let source_sha256 = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid IGES source SHA-256".to_owned()))?;
        let source_byte_len = evidence.source_byte_len;
        let source_unit = match evidence.source_unit.as_str() {
            "millimetre" => ImportLengthUnit::Millimetre,
            "centimetre" => ImportLengthUnit::Centimetre,
            "metre" => ImportLengthUnit::Metre,
            "inch" => ImportLengthUnit::Inch,
            "foot" => ImportLengthUnit::Foot,
            _ => return self.fail_protocol(response),
        };
        let parts = evidence
            .parts
            .into_iter()
            .map(|part| IgesXdePartEvidence {
                index: part.index,
                name: part.name,
                name_from_source: part.name_from_source,
                color: part.color,
                exact: StepImportEvidence {
                    source_sha256,
                    source_byte_len,
                    source_unit,
                    result_fingerprint: part.result_fingerprint,
                    body_kind: parse_import_body_kind(&part.body_kind)
                        .expect("worker body kind was validated"),
                    solid_count: part.solid_count,
                    topology_counts: part.topology_counts,
                    area_mm2: part.area_mm2,
                    volume_mm3: part.volume_mm3,
                    bounds_mm: part.bounds_mm,
                    backend: part.backend,
                    tolerance: part.tolerance,
                },
            })
            .collect();
        let nodes = evidence
            .nodes
            .into_iter()
            .map(|node| {
                if node.parent_id.is_some() {
                    return None;
                }
                let part_index = node.part_index?;
                let transform = Transform::from_matrix(node.transform)
                    .ok()
                    .filter(|transform| *transform == Transform::identity())?;
                Some(IgesXdeNodeEvidence {
                    id: node.id,
                    part_index,
                    name: node.name,
                    name_from_source: node.name_from_source,
                    color: node.color,
                    transform,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                WorkerError::Protocol(
                    "IGES XDE evidence is not a flat identity-root model".to_owned(),
                )
            })?;
        Ok(IgesXdeImportEvidence {
            source_sha256,
            source_byte_len: evidence.source_byte_len,
            parts,
            nodes,
        })
    }

    fn inspect_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        let (operation, response_schema, suffix) = if let Some(index) = part_index {
            self.verify_m21_step_xde_capability(cancelled)?;
            (
                "INSPECT_STEP_XDE_PART_M21_V1",
                "OK_M21_STEP_XDE_PART_V2",
                format!(" {index}"),
            )
        } else {
            self.verify_m21_step_model_capability(cancelled)?;
            (
                "INSPECT_STEP_PART_M21_V1",
                "OK_M21_STEP_PART_V4",
                String::new(),
            )
        };
        let response = self.request_with_cancellation(
            &format!(
                "{operation} {source_sha256} {}{suffix}",
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
        let source_sha256_bytes = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid import source SHA-256".to_owned()))?;
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        let evidence = (|| {
            if fields.len() != 20
                || fields[0] != response_schema
                || fields[1] != source_sha256
                || !is_fnv1a64_digest(fields[2])
            {
                return None;
            }
            let body_kind = parse_import_body_kind(fields[3])?;
            let solid_count = fields[4].parse::<u32>().ok()?;
            let area_mm2 = parse_bits(5)?;
            let volume_mm3 = parse_bits(6)?;
            let bounds_mm = [
                [parse_bits(7)?, parse_bits(8)?, parse_bits(9)?],
                [parse_bits(10)?, parse_bits(11)?, parse_bits(12)?],
            ];
            let topology_counts = [
                fields[13].parse::<u32>().ok()?,
                fields[14].parse::<u32>().ok()?,
                fields[15].parse::<u32>().ok()?,
                fields[16].parse::<u32>().ok()?,
                solid_count,
            ];
            let source_unit = match hex_decode_utf8(fields[17])?.as_str() {
                "millimetre" => ImportLengthUnit::Millimetre,
                "centimetre" => ImportLengthUnit::Centimetre,
                "metre" => ImportLengthUnit::Metre,
                "inch" => ImportLengthUnit::Inch,
                "foot" => ImportLengthUnit::Foot,
                _ => return None,
            };
            Some(StepImportEvidence {
                source_sha256: source_sha256_bytes,
                source_byte_len,
                source_unit,
                result_fingerprint: fields[2].to_owned(),
                body_kind,
                solid_count,
                topology_counts,
                area_mm2,
                volume_mm3,
                bounds_mm,
                backend: hex_decode_utf8(fields[18])?,
                tolerance: hex_decode_utf8(fields[19])?,
            })
        })();
        match evidence {
            Some(evidence) => Ok(evidence),
            None => self.fail_protocol(response),
        }
    }

    fn inspect_iges_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        self.verify_m21_iges_capability(cancelled)?;
        let (operation, response_schema, suffix) = if let Some(index) = part_index {
            (
                "INSPECT_IGES_XDE_PART_M21_V1",
                "OK_M21_IGES_XDE_PART_V2",
                format!(" {index}"),
            )
        } else {
            (
                "INSPECT_IGES_PART_M21_V1",
                "OK_M21_IGES_PART_V2",
                String::new(),
            )
        };
        let response = self.request_with_cancellation(
            &format!(
                "{operation} {source_sha256} {}{suffix}",
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
        let source_sha256_bytes = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid import source SHA-256".to_owned()))?;
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        let evidence = (|| {
            if fields.len() != 20
                || fields[0] != response_schema
                || fields[1] != source_sha256
                || !is_fnv1a64_digest(fields[2])
            {
                return None;
            }
            let body_kind = parse_import_body_kind(fields[3])?;
            let solid_count = fields[4].parse::<u32>().ok()?;
            let source_unit = match hex_decode_utf8(fields[17])?.as_str() {
                "millimetre" => ImportLengthUnit::Millimetre,
                "centimetre" => ImportLengthUnit::Centimetre,
                "metre" => ImportLengthUnit::Metre,
                "inch" => ImportLengthUnit::Inch,
                "foot" => ImportLengthUnit::Foot,
                _ => return None,
            };
            Some(StepImportEvidence {
                source_sha256: source_sha256_bytes,
                source_byte_len,
                source_unit,
                result_fingerprint: fields[2].to_owned(),
                body_kind,
                solid_count,
                topology_counts: [
                    fields[13].parse::<u32>().ok()?,
                    fields[14].parse::<u32>().ok()?,
                    fields[15].parse::<u32>().ok()?,
                    fields[16].parse::<u32>().ok()?,
                    solid_count,
                ],
                area_mm2: parse_bits(5)?,
                volume_mm3: parse_bits(6)?,
                bounds_mm: [
                    [parse_bits(7)?, parse_bits(8)?, parse_bits(9)?],
                    [parse_bits(10)?, parse_bits(11)?, parse_bits(12)?],
                ],
                backend: hex_decode_utf8(fields[18])?,
                tolerance: hex_decode_utf8(fields[19])?,
            })
        })();
        match evidence {
            Some(evidence) => Ok(evidence),
            None => self.fail_protocol(response),
        }
    }

    fn tessellate_iges_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        self.verify_m21_iges_capability(cancelled)?;
        let (operation, response_schema, suffix) = if let Some(index) = part_index {
            (
                "TESSELLATE_IGES_XDE_PART_M21_V1",
                "OK_M21_IGES_XDE_MESH_V1",
                format!(" {index}"),
            )
        } else {
            (
                "TESSELLATE_IGES_PART_M21_V1",
                "OK_M21_IGES_MESH_V1",
                String::new(),
            )
        };
        let response = self.request_with_cancellation(
            &format!(
                "{operation} {source_sha256} {}{suffix} {}",
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
            || fields[0] != response_schema
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
        let encoded = read_step_import_mesh_output(output_path, vertex_count, triangle_count)?;
        if sha256_hex(&encoded) != fields[5] {
            return Err(WorkerError::Transport(
                "imported IGES display mesh digest does not match the worker receipt".to_owned(),
            ));
        }
        let mesh = StepImportMesh::decode(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.vertices_mm.len() as u32 != vertex_count
            || mesh.triangles.len() as u32 != triangle_count
        {
            return Err(WorkerError::Transport(
                "imported IGES display mesh size does not match the worker receipt".to_owned(),
            ));
        }
        Ok(mesh)
    }

    fn convert_step_to_iges_request_with_cancellation(
        &mut self,
        source_path: &Path,
        source_sha256: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, WorkerError> {
        self.verify_m21_iges_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "CONVERT_STEP_XDE_TO_IGES_M21_V1 {source_sha256} {} {}",
                hex_encode(source_path.to_string_lossy().as_bytes()),
                hex_encode(output_path.to_string_lossy().as_bytes()),
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
        if fields.len() != 3
            || fields[0] != "OK_M21_IGES_EXPORT_V1"
            || fields[1] != source_sha256
            || !is_fnv1a64_digest(fields[2])
        {
            return self.fail_protocol(response);
        }
        self.read_bounded_output(
            output_path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker IGES output exceeds the bounded 32 MiB envelope",
        )
    }

    fn export_step_xde_part_request_with_cancellation(
        &mut self,
        source_path: &Path,
        source_sha256: &str,
        part_index: u32,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        self.verify_m21_step_xde_capability(cancelled)?;
        let response = self.request_with_cancellation(
            &format!(
                "EXPORT_STEP_XDE_PART_M21_V1 {source_sha256} {} {part_index} {result_fingerprint} {}",
                hex_encode(source_path.to_string_lossy().as_bytes()),
                hex_encode(output_path.to_string_lossy().as_bytes()),
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
        if fields.len() != 4
            || fields[0] != "OK_M21_STEP_XDE_EXPORT_V1"
            || fields[1] != source_sha256
            || fields[2] != result_fingerprint
            || !is_sha256_digest(fields[3])
        {
            return self.fail_protocol(response);
        }
        let bytes = self.read_bounded_output(
            output_path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker STEP output exceeds the bounded 32 MiB envelope",
        )?;
        if sha256_hex(&bytes) != fields[3] {
            return self.fail_protocol("worker STEP XDE part output hash mismatch".to_owned());
        }
        Ok(())
    }

    fn tessellate_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        let (operation, response_schema, suffix) = if let Some(index) = part_index {
            self.verify_m21_step_xde_capability(cancelled)?;
            (
                "TESSELLATE_STEP_XDE_PART_M21_V1",
                "OK_M21_STEP_XDE_MESH_V1",
                format!(" {index}"),
            )
        } else {
            self.verify_m21_step_model_capability(cancelled)?;
            (
                "TESSELLATE_STEP_PART_M21_V1",
                "OK_M21_STEP_MESH_V1",
                String::new(),
            )
        };
        let response = self.request_with_cancellation(
            &format!(
                "{operation} {source_sha256} {}{suffix} {}",
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
            || fields[0] != response_schema
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
        let encoded = read_step_import_mesh_output(output_path, vertex_count, triangle_count)?;
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
    ) -> Result<Vec<u8>, WorkerError> {
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
        let bytes = self.read_bounded_output(
            path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker STEP output exceeds the bounded 32 MiB envelope",
        )?;
        if sha256_hex(&bytes) != fields[3] {
            return self.fail_protocol("worker STEP output hash mismatch".to_owned());
        }
        Ok(bytes)
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
        self.request_with_timeout(request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &mut self,
        request: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<String, WorkerError> {
        // Writing and receiving share one budget; cancellation still polls at 10 ms.
        let deadline = Instant::now() + timeout;
        self.write_request_until_with_cancellation(request, deadline, cancelled, timeout)?;
        match self.next_response_until_with_cancellation(deadline, cancelled, timeout)? {
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
        self.write_request_until_with_cancellation(
            request,
            deadline,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
    }

    fn write_request_until_with_cancellation(
        &mut self,
        request: &str,
        deadline: Instant,
        cancelled: &AtomicBool,
        timeout: Duration,
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
                return self.fail(WorkerError::RequestTimedOut(timeout));
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
        self.next_response_until_with_cancellation(
            deadline,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
    }

    fn next_response_until_with_cancellation(
        &mut self,
        deadline: Instant,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<WorkerResponse, WorkerError> {
        loop {
            self.ensure_not_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(WorkerError::RequestTimedOut(timeout));
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

fn spawn_worker_reader(stdout: ChildStdout, sender: mpsc::SyncSender<WorkerResponse>) {
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

fn read_worker_response_line(reader: &mut impl BufRead) -> WorkerResponse {
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
    executable_sha256: String,
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
        let (executable, executable_sha256) = exact_worker_identity(executable.as_ref())?;
        let client = Self::spawn_verified_client(&executable, &executable_sha256, cancelled)?;
        Ok(Self {
            executable,
            executable_sha256,
            client,
        })
    }

    /// Canonical worker executable this supervisor was verified against.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Whether the worker process is alive and its executable is unchanged.
    pub fn is_reusable(&mut self) -> bool {
        matches!(self.client.child.try_wait(), Ok(None))
            && verify_exact_worker_identity(&self.executable, &self.executable_sha256).is_ok()
    }

    fn spawn_verified_client(
        executable: &Path,
        executable_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<ExactWorkerClient, WorkerError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(WorkerError::Cancelled);
        }
        let executable = verify_exact_worker_identity(executable, executable_sha256)?;
        let mut client = ExactWorkerClient::spawn_guarded(executable)?;
        client.ensure_not_cancelled(cancelled)?;
        client.ping_with_cancellation(cancelled)?;
        Ok(client)
    }

    pub fn inspect_step_xde_import_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepXdeImportEvidence, WorkerError> {
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        if source_byte_len > MAX_STEP_SOURCE_BYTES {
            return Err(WorkerError::Transport(
                "STEP source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        let evidence = match self.client.inspect_step_xde_request_with_cancellation(
            path,
            source_sha256,
            cancelled,
        ) {
            Ok(evidence) => evidence,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_step_xde_request_with_cancellation(
                    path,
                    source_sha256,
                    cancelled,
                )?
            }
            Err(error) => return Err(error),
        };
        if evidence.source_byte_len != source_byte_len {
            return Err(WorkerError::Transport(
                "STEP XDE evidence byte length does not match the sealed source".to_owned(),
            ));
        }
        Ok(evidence)
    }

    pub fn inspect_step_xde_part_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: u32,
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
            Some(part_index),
            cancelled,
        ) {
            Ok(evidence) => Ok(evidence),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    Some(part_index),
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
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
            None,
            cancelled,
        ) {
            Ok(evidence) => Ok(evidence),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    None,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn inspect_iges_xde_import_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<IgesXdeImportEvidence, WorkerError> {
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        if source_byte_len > MAX_STEP_SOURCE_BYTES {
            return Err(WorkerError::Transport(
                "IGES source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        let evidence = match self.client.inspect_iges_xde_request_with_cancellation(
            path,
            source_sha256,
            cancelled,
        ) {
            Ok(evidence) => evidence,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_iges_xde_request_with_cancellation(
                    path,
                    source_sha256,
                    cancelled,
                )?
            }
            Err(error) => return Err(error),
        };
        if evidence.source_byte_len != source_byte_len {
            return Err(WorkerError::Transport(
                "IGES XDE evidence byte length does not match the sealed source".to_owned(),
            ));
        }
        Ok(evidence)
    }

    pub fn inspect_iges_xde_part_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: u32,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        if std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len()
            > MAX_STEP_SOURCE_BYTES
        {
            return Err(WorkerError::Transport(
                "IGES source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.inspect_iges_part_request_with_cancellation(
            path,
            source_sha256,
            Some(part_index),
            cancelled,
        ) {
            Ok(evidence) => Ok(evidence),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_iges_part_request_with_cancellation(
                    path,
                    source_sha256,
                    Some(part_index),
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn inspect_iges_import_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<IgesImportEvidence, WorkerError> {
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        if source_byte_len > MAX_STEP_SOURCE_BYTES {
            return Err(WorkerError::Transport(
                "IGES source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        let evidence = match self.client.inspect_iges_part_request_with_cancellation(
            path,
            source_sha256,
            None,
            cancelled,
        ) {
            Ok(evidence) => evidence,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.inspect_iges_part_request_with_cancellation(
                    path,
                    source_sha256,
                    None,
                    cancelled,
                )?
            }
            Err(error) => return Err(error),
        };
        let source_sha256 = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid IGES source SHA-256".to_owned()))?;
        Ok(IgesImportEvidence {
            source_sha256,
            source_byte_len,
            source_unit: evidence.source_unit,
            result_fingerprint: evidence.result_fingerprint,
            body_kind: evidence.body_kind,
            solid_count: evidence.solid_count,
            topology_counts: evidence.topology_counts,
            area_mm2: evidence.area_mm2,
            volume_mm3: evidence.volume_mm3,
            bounds_mm: evidence.bounds_mm,
            backend: evidence.backend,
            tolerance: evidence.tolerance,
        })
    }

    pub fn tessellate_iges_xde_part_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: u32,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        if std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len()
            > MAX_STEP_SOURCE_BYTES
        {
            return Err(WorkerError::Transport(
                "IGES source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.tessellate_iges_part_request_with_cancellation(
            path,
            source_sha256,
            result_fingerprint,
            output_path,
            Some(part_index),
            cancelled,
        ) {
            Ok(mesh) => Ok(mesh),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.tessellate_iges_part_request_with_cancellation(
                    path,
                    source_sha256,
                    result_fingerprint,
                    output_path,
                    Some(part_index),
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn tessellate_iges_import_with_cancellation(
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
                "IGES source exceeds the bounded 32 MiB envelope".to_owned(),
            ));
        }
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.tessellate_iges_part_request_with_cancellation(
            path,
            source_sha256,
            result_fingerprint,
            output_path,
            None,
            cancelled,
        ) {
            Ok(mesh) => Ok(mesh),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.tessellate_iges_part_request_with_cancellation(
                    path,
                    source_sha256,
                    result_fingerprint,
                    output_path,
                    None,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn convert_step_to_iges_with_cancellation(
        &mut self,
        source_path: &Path,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, WorkerError> {
        let source = read_bounded_regular_file(
            source_path,
            MAX_STEP_SOURCE_BYTES,
            "STEP staging source exceeds the bounded 32 MiB envelope",
        )?;
        let source_sha256 = sha256_hex(&source);
        self.client.ensure_not_cancelled(cancelled)?;
        match self.client.convert_step_to_iges_request_with_cancellation(
            source_path,
            &source_sha256,
            output_path,
            cancelled,
        ) {
            Ok(bytes) => Ok(bytes),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.convert_step_to_iges_request_with_cancellation(
                    source_path,
                    &source_sha256,
                    output_path,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn tessellate_step_xde_part_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: u32,
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
            Some(part_index),
            cancelled,
        ) {
            Ok(mesh) => Ok(mesh),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.tessellate_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    result_fingerprint,
                    output_path,
                    Some(part_index),
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
            None,
            cancelled,
        ) {
            Ok(mesh) => Ok(mesh),
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.tessellate_step_part_request_with_cancellation(
                    path,
                    source_sha256,
                    result_fingerprint,
                    output_path,
                    None,
                    cancelled,
                )
            }
            Err(error) => Err(error),
        }
    }

    pub fn simulate_cam(
        &mut self,
        snapshot: &Snapshot,
        plan: &CamPlan,
        toolpath: &CamToolpath,
        fixtures: &[CamFixture],
        cancelled: &AtomicBool,
    ) -> Result<CamSimulationEvidence, WorkerError> {
        toolpath
            .validate(snapshot, plan)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if fixtures.len() > MAX_CAM_SIMULATION_FIXTURES
            || toolpath.motions.len() > MAX_CAM_SIMULATION_MOTIONS
            || toolpath
                .motions
                .len()
                .checked_mul(fixtures.len().saturating_mul(2).saturating_add(2))
                .is_none_or(|checks| checks > MAX_CAM_SIMULATION_PAIR_CHECKS)
        {
            return Err(WorkerError::Protocol(
                "CAM simulation exceeds its bounded resource envelope".to_owned(),
            ));
        }
        let mut fixture_ids = BTreeSet::new();
        for fixture in fixtures {
            fixture
                .validate()
                .map_err(|error| WorkerError::Protocol(error.to_string()))?;
            if !fixture_ids.insert(fixture.id) {
                return Err(WorkerError::Protocol(
                    "CAM fixture identifiers must be unique".to_owned(),
                ));
            }
        }
        let graph = ExactBRepGraph::from_snapshot(
            snapshot,
            plan.target().definition_id,
            plan.target().feature_id,
        )
        .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if graph.graph_digest != plan.target().exact_graph_digest
            || graph
                .nodes
                .iter()
                .any(|node| matches!(node.operation, ExactBRepOperation::ImportedExact { .. }))
        {
            return Err(WorkerError::Protocol(
                "CAM simulation target is stale or requires unavailable imported sources"
                    .to_owned(),
            ));
        }
        let setup = plan.setup();
        let z_axis = [
            setup.x_axis[1] * setup.y_axis[2] - setup.x_axis[2] * setup.y_axis[1],
            setup.x_axis[2] * setup.y_axis[0] - setup.x_axis[0] * setup.y_axis[2],
            setup.x_axis[0] * setup.y_axis[1] - setup.x_axis[1] * setup.y_axis[0],
        ];
        let request = CamSimulationWireRequest {
            schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
            plan_digest: plan.stable_digest(),
            toolpath_digest: toolpath.toolpath_digest.clone(),
            target_exact_graph_digest: graph.graph_digest.clone(),
            stock_bounds_mm: [plan.stock().minimum_mm, plan.stock().maximum_mm],
            setup_to_world: [
                setup.x_axis[0],
                setup.y_axis[0],
                z_axis[0],
                setup.origin_mm[0],
                setup.x_axis[1],
                setup.y_axis[1],
                z_axis[1],
                setup.origin_mm[1],
                setup.x_axis[2],
                setup.y_axis[2],
                z_axis[2],
                setup.origin_mm[2],
                0.0,
                0.0,
                0.0,
                1.0,
            ],
            cutter_radius_mm: plan.tool().diameter_mm * 0.5,
            cutter_length_mm: plan.tool().flute_length_mm,
            tool_length_mm: plan.tool().overall_length_mm,
            holder_radius_mm: plan.tool().holder_diameter_mm * 0.5,
            holder_offset_mm: plan.tool().overall_length_mm,
            holder_length_mm: plan.tool().holder_length_mm,
            motions: toolpath
                .motions
                .iter()
                .map(|motion| {
                    let (path_kind, start_mm, end_mm, center_mm, clockwise) = match motion.path {
                        CamMotionPath::Line { start_mm, end_mm } => {
                            (0, start_mm, end_mm, [0.0; 3], false)
                        }
                        CamMotionPath::Arc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => (1, start_mm, end_mm, center_mm, clockwise),
                    };
                    CamSimulationWireMotion {
                        kind: match motion.kind {
                            CamMotionKind::Rapid => 0,
                            CamMotionKind::Plunge => 1,
                            CamMotionKind::Cut => 2,
                            CamMotionKind::Retract => 3,
                        },
                        path_kind,
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    }
                })
                .collect(),
            fixtures: fixtures
                .iter()
                .map(|fixture| CamSimulationWireFixture {
                    id: fixture.id,
                    bounds_mm: [fixture.minimum_mm, fixture.maximum_mm],
                })
                .collect(),
        };
        self.client.ensure_not_cancelled(cancelled)?;
        let evidence = match self
            .client
            .simulate_cam_with_cancellation(&graph, &request, cancelled)
        {
            Ok(evidence) => evidence,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client
                    .simulate_cam_with_cancellation(&graph, &request, cancelled)?
            }
            Err(error) => return Err(error),
        };
        validate_cam_simulation_wire_evidence(&request, &evidence)?;
        let mut evidence = cam_simulation_evidence(evidence)?;
        evidence.result_fingerprint = evidence.stable_fingerprint();
        Ok(evidence)
    }

    pub fn evaluate_exact_brep_graph_volume_mesh(
        &mut self,
        graph: &ExactBRepGraph,
        options: ExactVolumeMeshWireOptions,
    ) -> Result<ExactBRepVolumeMeshPackage, WorkerError> {
        self.evaluate_exact_brep_graph_volume_mesh_with_cancellation(
            graph,
            options,
            &NEVER_CANCELLED,
        )
    }

    pub fn evaluate_exact_brep_graph_volume_mesh_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        options: ExactVolumeMeshWireOptions,
        cancelled: &AtomicBool,
    ) -> Result<ExactBRepVolumeMeshPackage, WorkerError> {
        self.client.ensure_not_cancelled(cancelled)?;
        let source = self.evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
            graph,
            &[],
            cancelled,
        )?;
        let output_file = tempfile::Builder::new()
            .prefix(".ketchup-brep-volume-mesh-")
            .suffix(".json")
            .tempfile()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        let mesh = match self.client.volume_mesh_exact_brep_graph_with_cancellation(
            graph,
            &source.identity.result_fingerprint,
            options,
            output_file.path(),
            &[],
            cancelled,
        ) {
            Ok(mesh) => mesh,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.volume_mesh_exact_brep_graph_with_cancellation(
                    graph,
                    &source.identity.result_fingerprint,
                    options,
                    output_file.path(),
                    &[],
                    cancelled,
                )?
            }
            Err(error) => return Err(error),
        };
        self.client.ensure_not_cancelled(cancelled)?;
        let vertex_count = mesh.vertices_mm.len();
        let source_volume_scale = source.volume_mm3.abs().max(1.0);
        if mesh.schema != EXACT_VOLUME_MESH_WIRE_SCHEMA_V1
            || mesh.graph_digest != graph.graph_digest
            || mesh.source_result_fingerprint != source.identity.result_fingerprint
            || mesh.vertices_mm.len() < 4
            || mesh
                .vertices_mm
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || mesh.tetrahedra.is_empty()
            || mesh.tetrahedra.len() > options.max_tetrahedra as usize
            || mesh.tetrahedra.iter().any(|tetrahedron| {
                tetrahedron
                    .iter()
                    .any(|index| *index as usize >= vertex_count)
                    || {
                        let mut unique = *tetrahedron;
                        unique.sort_unstable();
                        unique.windows(2).any(|pair| pair[0] == pair[1])
                    }
            })
            || mesh.boundary_triangles.is_empty()
            || mesh.boundary_triangles.iter().any(|triangle| {
                triangle.face_ordinal >= source.topology_counts[2]
                    || triangle
                        .vertex_indices
                        .iter()
                        .any(|index| *index as usize >= vertex_count)
            })
            || !mesh.exact_volume_mm3.is_finite()
            || (mesh.exact_volume_mm3 - source.volume_mm3).abs() > source_volume_scale * 1.0e-10
            || !mesh.tetrahedral_volume_mm3.is_finite()
            || mesh.tetrahedral_volume_mm3 <= 0.0
            || !mesh.relative_volume_error.is_finite()
            || mesh.relative_volume_error > options.max_relative_volume_error
            || !mesh.minimum_signed_volume_mm3.is_finite()
            || mesh.minimum_signed_volume_mm3 <= 0.0
            || !mesh.minimum_quality.is_finite()
            || mesh.minimum_quality < options.min_tetrahedron_quality
            || !mesh.maximum_edge_ratio.is_finite()
            || mesh.maximum_edge_ratio < 1.0
            || mesh.request_digest.is_empty()
            || mesh.mesh_fingerprint.is_empty()
        {
            return Err(WorkerError::Protocol(
                "exact volume mesh does not satisfy its graph-bound request".to_owned(),
            ));
        }
        Ok(ExactBRepVolumeMeshPackage {
            source,
            options,
            mesh,
        })
    }

    pub fn evaluate_exact_brep_graph(
        &mut self,
        graph: &ExactBRepGraph,
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.evaluate_exact_brep_graph_with_imported_sources(graph, &[])
    }

    pub fn evaluate_exact_brep_graph_with_imported_source(
        &mut self,
        graph: &ExactBRepGraph,
        source: &[u8],
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.evaluate_exact_brep_graph_with_imported_sources(graph, &[source])
    }

    pub fn evaluate_exact_brep_graph_with_imported_sources(
        &mut self,
        graph: &ExactBRepGraph,
        sources: &[&[u8]],
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
            graph,
            sources,
            &NEVER_CANCELLED,
        )
    }

    pub fn evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        sources: &[&[u8]],
        cancelled: &AtomicBool,
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.client.ensure_not_cancelled(cancelled)?;
        let mut expected = BTreeMap::<String, u64>::new();
        let mut source_order = Vec::new();
        for node in &graph.nodes {
            let ExactBRepOperation::ImportedExact {
                source_sha256,
                source_byte_len,
                ..
            } = &node.operation
            else {
                continue;
            };
            let source_sha256 = source_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if let Some(previous_len) = expected.get(&source_sha256) {
                if previous_len != source_byte_len {
                    return Err(WorkerError::Protocol(
                        "imported exact source identities disagree on byte length".to_owned(),
                    ));
                }
            } else {
                expected.insert(source_sha256.clone(), *source_byte_len);
                source_order.push(source_sha256);
            }
        }
        if source_order.len() > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
            || sources.len() != source_order.len()
        {
            return Err(WorkerError::Protocol(
                "imported exact source count does not match the bounded graph identity".to_owned(),
            ));
        }
        let mut supplied = BTreeMap::<String, &[u8]>::new();
        let mut total_bytes = 0_u64;
        for source in sources {
            self.client.ensure_not_cancelled(cancelled)?;
            let source_len = source.len() as u64;
            total_bytes = total_bytes.checked_add(source_len).ok_or_else(|| {
                WorkerError::Protocol(
                    "imported exact sources exceed the bounded envelope".to_owned(),
                )
            })?;
            if source_len > MAX_STEP_SOURCE_BYTES
                || total_bytes > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES
                || supplied.insert(sha256_hex(source), *source).is_some()
            {
                return Err(WorkerError::Protocol(
                    "imported exact sources exceed or duplicate the bounded envelope".to_owned(),
                ));
            }
        }
        let mut source_files = Vec::with_capacity(source_order.len());
        for source_sha256 in source_order {
            self.client.ensure_not_cancelled(cancelled)?;
            let expected_len = expected[&source_sha256];
            let Some(source) = supplied.remove(&source_sha256) else {
                return Err(WorkerError::Protocol(
                    "imported exact source does not match its graph identity".to_owned(),
                ));
            };
            if source.len() as u64 != expected_len {
                return Err(WorkerError::Protocol(
                    "imported exact source does not match its graph identity".to_owned(),
                ));
            }
            let mut source_file = tempfile::Builder::new()
                .prefix(".ketchup-brep-graph-source-")
                .suffix(".step")
                .tempfile()
                .map_err(|error| WorkerError::Transport(error.to_string()))?;
            source_file
                .write_all(source)
                .and_then(|()| source_file.flush())
                .map_err(|error| WorkerError::Transport(error.to_string()))?;
            source_files.push((source_sha256, source_file));
        }
        if !supplied.is_empty() {
            return Err(WorkerError::Protocol(
                "imported exact source does not match its graph identity".to_owned(),
            ));
        }
        let source_paths = source_files
            .iter()
            .map(|(source_sha256, source)| (source_sha256.as_str(), source.path()))
            .collect::<Vec<_>>();
        self.evaluate_exact_brep_graph_from_sources(graph, &source_paths, cancelled)
    }

    fn evaluate_exact_brep_graph_from_sources(
        &mut self,
        graph: &ExactBRepGraph,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<ExactBRepGraphPackage, WorkerError> {
        self.client.ensure_not_cancelled(cancelled)?;
        let result = match self.client.evaluate_exact_brep_graph_with_cancellation(
            graph,
            imported_sources,
            cancelled,
        ) {
            Ok(result) => result,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.evaluate_exact_brep_graph_with_cancellation(
                    graph,
                    imported_sources,
                    cancelled,
                )?
            }
            Err(error) => return Err(error),
        };
        let valid_terminal = if graph.terminal_is_planar_offset() {
            let bounds_mm = [
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
            result.volume_mm3.is_finite()
                && result.volume_mm3 == 0.0
                && if graph.terminal_planar_offset_is_framed() {
                    result.area_mm2.is_finite()
                        && result.area_mm2 > 0.0
                        && result.topology_counts[0] != 0
                        && result.topology_counts[0] == result.topology_counts[1]
                        && result.topology_counts[2..] == [1, 0, 0]
                        && bounds_mm.iter().flatten().all(|value| value.is_finite())
                        && (0..3).all(|axis| bounds_mm[0][axis] <= bounds_mm[1][axis])
                } else {
                    graph.accepts_terminal_planar_offset_geometry(
                        bounds_mm,
                        None,
                        result.area_mm2,
                        result.topology_counts,
                        result.wire_count,
                    )
                }
        } else if graph.terminal_is_surface() {
            let bounds_mm = [
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
            result.volume_mm3.is_finite()
                && result.volume_mm3 == 0.0
                && result.area_mm2.is_finite()
                && result.area_mm2 > 0.0
                && result.topology_counts[0] > 0
                && result.topology_counts[1] > 0
                && result.topology_counts[2] > 0
                && result.topology_counts[4] == 0
                && bounds_mm.iter().flatten().all(|value| value.is_finite())
                && (0..3).all(|axis| bounds_mm[0][axis] <= bounds_mm[1][axis])
        } else {
            result.volume_mm3.is_finite()
                && result.volume_mm3 > 0.0
                && result.area_mm2.is_finite()
                && result.area_mm2 == 0.0
                && result.bounds_mm.iter().all(|value| value.is_finite())
                && !result.topology_counts.contains(&0)
        };
        if result.canonical_input_digest != graph.canonical_input_digest
            || result.graph_digest != graph.graph_digest
            || result.producer_feature_id != graph.producer_feature_id
            || result.result_fingerprint.is_empty()
            || result.exact_input_digest.is_empty()
            || !valid_terminal
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
            imported_sources,
            cancelled,
        ) {
            Ok(mesh) => mesh,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    cancelled,
                )?;
                self.client.tessellate_exact_brep_graph_with_cancellation(
                    graph,
                    &result.result_fingerprint,
                    mesh_file.path(),
                    imported_sources,
                    cancelled,
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
                area_mm2: result.area_mm2,
                topology_counts: result.topology_counts,
                wire_count: result.wire_count,
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
                faces: result
                    .faces
                    .into_iter()
                    .map(|face| ExactBRepGraphFaceEvidence {
                        semantic_role: face.semantic_role,
                        source_element_id: face.source_element_id,
                        face_ordinal: face.face_ordinal,
                        surface_kind: face.surface_kind,
                        corroborating_geometry_fingerprint: face.geometric_fingerprint,
                        centroid_mm: face.centroid_mm,
                        unit_normal: face.unit_normal,
                        axis_origin_mm: face.axis_origin_mm,
                        unit_axis_direction: face.unit_axis_direction,
                    })
                    .collect(),
                edges: result
                    .edges
                    .into_iter()
                    .map(|edge| ExactBRepGraphEdgeEvidence {
                        edge_ordinal: edge.edge_ordinal,
                        curve_kind: edge.curve_kind,
                        length_mm: edge.length_mm,
                        centroid_mm: edge.centroid_mm,
                        bounds_mm: edge.bounds_mm,
                        closed: edge.closed,
                        circle_radius_mm: edge.circle_radius_mm,
                        axis_origin_mm: edge.axis_origin_mm,
                        unit_axis_direction: edge.unit_axis_direction,
                        adjacent_face_ordinals: edge.adjacent_face_ordinals,
                    })
                    .collect(),
            },
            &mesh,
        )
        .map_err(|error| WorkerError::Protocol(error.to_string()))
    }

    pub fn export_exact_brep_graph_step(
        &mut self,
        snapshot: &Snapshot,
        expected: &ExactBRepGraphPackage,
        path: &Path,
    ) -> Result<(), M6EvaluationError> {
        self.export_exact_brep_graph_step_with_imported_sources(snapshot, expected, path, &[])
    }

    pub fn export_exact_brep_graph_step_with_imported_sources(
        &mut self,
        snapshot: &Snapshot,
        expected: &ExactBRepGraphPackage,
        path: &Path,
        sources: &[&[u8]],
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
        if graph != *expected.graph {
            return Err(ExactProductError::InvalidWorkerEvidence.into());
        }
        let prepared = prepare_exact_brep_graph_sources(&graph, sources)?;
        let source_paths = prepared.paths();
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
            &source_paths,
            &NEVER_CANCELLED,
        );
        match export {
            Ok(()) => {}
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    &NEVER_CANCELLED,
                )?;
                self.client.export_exact_brep_graph_step_with_cancellation(
                    &graph,
                    &expected.identity.result_fingerprint,
                    &temporary,
                    &source_paths,
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

    fn export_exact_brep_graph_step_from_blobs_once(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        path: &Path,
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), WorkerError> {
        let sources = exact_brep_graph_sources_from_blobs(graph, imported_source_blobs)?;
        let prepared = prepare_exact_brep_graph_sources(graph, &sources)?;
        let source_paths = prepared.paths();
        self.client.export_exact_brep_graph_step_with_cancellation(
            graph,
            result_fingerprint,
            path,
            &source_paths,
            &NEVER_CANCELLED,
        )
    }

    pub fn export_current_model_step(
        &mut self,
        snapshot: &Snapshot,
        occurrences: &[(ExactBodyPackage, Transform)],
        path: &Path,
    ) -> Result<Vec<u8>, M6EvaluationError> {
        self.export_current_model_step_with_imported_sources(
            snapshot,
            occurrences,
            path,
            &BTreeMap::new(),
        )
    }

    pub fn export_current_model_step_with_imported_sources(
        &mut self,
        snapshot: &Snapshot,
        occurrences: &[(ExactBodyPackage, Transform)],
        path: &Path,
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, M6EvaluationError> {
        let scene = snapshot.scene_query();
        let mut matched = Vec::with_capacity(occurrences.len());
        for (package, transform) in occurrences {
            let occurrence = scene
                .iter()
                .find(|occurrence| {
                    occurrence.visible
                        && occurrence.definition_id == package.definition_id()
                        && occurrence.transform == *transform
                })
                .ok_or(ExactProductError::InvalidWorkerEvidence)?;
            matched.push((package.clone(), occurrence.clone()));
        }
        self.export_current_model_step_scene_with_imported_sources(
            snapshot,
            &matched,
            path,
            imported_source_blobs,
        )
    }

    pub fn export_current_model_step_scene_with_imported_sources(
        &mut self,
        snapshot: &Snapshot,
        occurrences: &[(ExactBodyPackage, SceneOccurrence)],
        path: &Path,
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, M6EvaluationError> {
        if occurrences.is_empty() {
            return Err(ExactProductError::EmptyModelExport.into());
        }
        for (package, _) in occurrences {
            if !package.is_current(snapshot) {
                return Err(ExactProductError::StaleResult.into());
            }
            match package {
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
        let mut part_indices = BTreeMap::new();
        let mut manifest = StepAssemblyManifest {
            schema: "ketchup.step-xde-assembly.v2".to_owned(),
            document_id: snapshot.document_id().0,
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            parts: Vec::with_capacity(occurrences.len()),
            nodes: Vec::new(),
        };
        for (index, (package, _)) in occurrences.iter().enumerate() {
            let package_key = package.result_key();
            let geometry_key = (
                package_key.definition_id.0,
                package_key.producer_feature_id.0,
                package_key.result_fingerprint.clone(),
            );
            if part_indices.contains_key(&geometry_key) {
                continue;
            }
            let source = directory.path().join(format!("part-{index}.step"));
            let result = match package {
                ExactBodyPackage::Graph(expected) => self
                    .export_exact_brep_graph_step_from_blobs_once(
                        &expected.graph,
                        &expected.identity.result_fingerprint,
                        &source,
                        imported_source_blobs,
                    ),
                ExactBodyPackage::Imported(expected) => {
                    if let Some(part_index) = expected.source_part_index {
                        let original = directory.path().join(format!("imported-{index}.step"));
                        std::fs::write(&original, &expected.source_bytes)
                            .map_err(|error| WorkerError::Transport(error.to_string()))?;
                        self.client.export_step_xde_part_request_with_cancellation(
                            &original,
                            &sha256_hex(&expected.source_bytes),
                            part_index,
                            &expected.identity.result_fingerprint,
                            &source,
                            &NEVER_CANCELLED,
                        )
                    } else {
                        std::fs::write(&source, &expected.source_bytes)
                            .map_err(|error| WorkerError::Transport(error.to_string()))
                    }
                }
            };
            if let Err(error) = result {
                if error.permits_restart() {
                    self.client = Self::spawn_verified_client(
                        &self.executable,
                        &self.executable_sha256,
                        &NEVER_CANCELLED,
                    )?;
                    match package {
                        ExactBodyPackage::Graph(expected) => {
                            self.export_exact_brep_graph_step_from_blobs_once(
                                &expected.graph,
                                &expected.identity.result_fingerprint,
                                &source,
                                imported_source_blobs,
                            )?;
                        }
                        ExactBodyPackage::Imported(expected) => {
                            if let Some(part_index) = expected.source_part_index {
                                let original =
                                    directory.path().join(format!("imported-{index}.step"));
                                std::fs::write(&original, &expected.source_bytes)
                                    .map_err(|error| WorkerError::Transport(error.to_string()))?;
                                self.client.export_step_xde_part_request_with_cancellation(
                                    &original,
                                    &sha256_hex(&expected.source_bytes),
                                    part_index,
                                    &expected.identity.result_fingerprint,
                                    &source,
                                    &NEVER_CANCELLED,
                                )?;
                            } else {
                                std::fs::write(&source, &expected.source_bytes)
                                    .map_err(|error| WorkerError::Transport(error.to_string()))?;
                            }
                        }
                    }
                } else {
                    return Err(error.into());
                }
            }
            let source_bytes = self.client.read_bounded_output(
                &source,
                MAX_STEP_SOURCE_BYTES,
                "exact worker STEP output exceeds the bounded 32 MiB envelope",
            )?;
            let source_sha256 = sha256_hex(&source_bytes);
            let imported_result_fingerprint = self
                .client
                .inspect_step_part_request_with_cancellation(
                    &source,
                    &source_sha256,
                    None,
                    &NEVER_CANCELLED,
                )?
                .result_fingerprint;
            let definition = snapshot
                .definition(package_key.definition_id)
                .ok_or(ExactProductError::InvalidWorkerEvidence)?;
            let part_index = manifest.parts.len() as u32;
            manifest.parts.push(StepAssemblyPart {
                document_id: snapshot.document_id().0,
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                definition_id: package_key.definition_id.0,
                producer_feature_id: package_key.producer_feature_id.0,
                name: definition.name().to_owned(),
                expected_result_fingerprint: package_key.result_fingerprint,
                imported_result_fingerprint,
                source_sha256,
            });
            sources.push(source);
            part_indices.insert(geometry_key, part_index);
        }
        manifest.nodes = build_step_assembly_nodes(snapshot, occurrences, &part_indices)?;
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
        let verified = match assembly {
            Ok(bytes) => bytes,
            Err(error) if error.permits_restart() => {
                self.client = Self::spawn_verified_client(
                    &self.executable,
                    &self.executable_sha256,
                    &NEVER_CANCELLED,
                )?;
                self.client.assemble_step_model_request_with_cancellation(
                    &manifest,
                    &sources,
                    &temporary,
                    &NEVER_CANCELLED,
                )?
            }
            Err(error) => return Err(error.into()),
        };
        temporary
            .persist(path)
            .map_err(|error| WorkerError::Transport(error.error.to_string()))?;
        Ok(verified)
    }
}

#[allow(clippy::too_many_arguments)]
fn push_step_assembly_node(
    nodes: &mut Vec<StepAssemblyNode>,
    keys: &mut BTreeMap<String, u32>,
    key: String,
    parent_id: Option<u32>,
    part_index: Option<u32>,
    name: String,
    color: Option<[u8; 3]>,
    transform: Transform,
) -> Result<u32, ExactProductError> {
    if let Some(id) = keys.get(&key) {
        return Ok(*id);
    }
    if nodes.len() >= 1_024 || name.is_empty() || name.len() > 4_096 {
        return Err(ExactProductError::ExportResourceLimit);
    }
    let id = nodes.len() as u32;
    nodes.push(StepAssemblyNode {
        id,
        parent_id,
        part_index,
        name,
        color,
        transform_bits: transform.matrix().map(f64::to_bits),
    });
    keys.insert(key, id);
    Ok(id)
}

fn ensure_step_global_group(
    snapshot: &Snapshot,
    group_id: GroupId,
    nodes: &mut Vec<StepAssemblyNode>,
    keys: &mut BTreeMap<String, u32>,
) -> Result<u32, ExactProductError> {
    let key = format!("group:{}", group_id.0);
    if let Some(id) = keys.get(&key) {
        return Ok(*id);
    }
    let group = snapshot
        .group(group_id)
        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
    let parent = group
        .parent()
        .map(|parent| ensure_step_global_group(snapshot, parent, nodes, keys))
        .transpose()?;
    push_step_assembly_node(
        nodes,
        keys,
        key,
        parent,
        None,
        group.name().to_owned(),
        None,
        group.transform(),
    )
}

fn build_step_assembly_nodes(
    snapshot: &Snapshot,
    occurrences: &[(ExactBodyPackage, SceneOccurrence)],
    part_indices: &BTreeMap<(u64, u64, String), u32>,
) -> Result<Vec<StepAssemblyNode>, ExactProductError> {
    let mut path_body_counts = BTreeMap::new();
    let mut nested_roots = BTreeMap::new();
    for (_, occurrence) in occurrences {
        *path_body_counts
            .entry(occurrence.instance_path.clone())
            .or_insert(0usize) += 1;
        if !occurrence.instance_path.steps().is_empty() {
            nested_roots.insert(occurrence.instance_path.root_occurrence().0, true);
        }
    }

    let mut nodes = Vec::new();
    let mut keys = BTreeMap::new();
    for (package, occurrence) in occurrences {
        let result_key = package.result_key();
        let part_index = *part_indices
            .get(&(
                result_key.definition_id.0,
                result_key.producer_feature_id.0,
                result_key.result_fingerprint,
            ))
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let root_id = occurrence.instance_path.root_occurrence();
        let root = snapshot
            .occurrence(root_id)
            .ok_or(ExactProductError::InvalidWorkerEvidence)?;
        let mut parent = root
            .parent()
            .map(|group| ensure_step_global_group(snapshot, group, &mut nodes, &mut keys))
            .transpose()?;
        let root_key = format!("occurrence:{}", root_id.0);
        let root_is_assembly = nested_roots.contains_key(&root_id.0)
            || path_body_counts
                .get(&ketchup_core::document::InstancePath::root(root_id))
                .copied()
                .unwrap_or(0)
                > 1;
        if occurrence.instance_path.is_root() && !root_is_assembly {
            push_step_assembly_node(
                &mut nodes,
                &mut keys,
                root_key,
                parent,
                Some(part_index),
                root.name().to_owned(),
                occurrence.color,
                root.transform(),
            )?;
            continue;
        }
        parent = Some(push_step_assembly_node(
            &mut nodes,
            &mut keys,
            root_key.clone(),
            parent,
            None,
            root.name().to_owned(),
            root.color(),
            root.transform(),
        )?);
        if occurrence.instance_path.is_root() {
            push_step_assembly_node(
                &mut nodes,
                &mut keys,
                format!("{root_key}/body:{}", package.producer_feature_id().0),
                parent,
                Some(part_index),
                format!(
                    "{} body {}",
                    occurrence.definition_name,
                    package.producer_feature_id().0
                ),
                occurrence.color,
                Transform::identity(),
            )?;
            continue;
        }

        let mut owner_definition_id = root.definition_id();
        let mut path_key = root_key;
        for (position, step) in occurrence.instance_path.steps().iter().enumerate() {
            let terminal = position + 1 == occurrence.instance_path.steps().len();
            match step {
                InstancePathStep::Group(local_id) => {
                    path_key.push_str(&format!("/group:{}", local_id.0));
                    let local = snapshot
                        .local_group(LocalGroupKey {
                            definition_id: owner_definition_id,
                            local_id: *local_id,
                        })
                        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                    parent = Some(push_step_assembly_node(
                        &mut nodes,
                        &mut keys,
                        path_key.clone(),
                        parent,
                        None,
                        local.name().to_owned(),
                        None,
                        local.transform(),
                    )?);
                }
                InstancePathStep::Occurrence(local_id) => {
                    path_key.push_str(&format!("/occurrence:{}", local_id.0));
                    let local = snapshot
                        .local_occurrence(LocalOccurrenceKey {
                            definition_id: owner_definition_id,
                            local_id: *local_id,
                        })
                        .ok_or(ExactProductError::InvalidWorkerEvidence)?;
                    let multiple_bodies = path_body_counts
                        .get(&occurrence.instance_path)
                        .copied()
                        .unwrap_or(0)
                        > 1;
                    if terminal && !multiple_bodies {
                        push_step_assembly_node(
                            &mut nodes,
                            &mut keys,
                            path_key.clone(),
                            parent,
                            Some(part_index),
                            local.name().to_owned(),
                            occurrence.color,
                            local.transform(),
                        )?;
                    } else {
                        parent = Some(push_step_assembly_node(
                            &mut nodes,
                            &mut keys,
                            path_key.clone(),
                            parent,
                            None,
                            local.name().to_owned(),
                            local.color(),
                            local.transform(),
                        )?);
                    }
                    owner_definition_id = local.definition_id();
                }
            }
        }
        if path_body_counts
            .get(&occurrence.instance_path)
            .copied()
            .unwrap_or(0)
            > 1
        {
            push_step_assembly_node(
                &mut nodes,
                &mut keys,
                format!("{path_key}/body:{}", package.producer_feature_id().0),
                parent,
                Some(part_index),
                format!(
                    "{} body {}",
                    occurrence.definition_name,
                    package.producer_feature_id().0
                ),
                occurrence.color,
                Transform::identity(),
            )?;
        }
    }
    if nodes.is_empty() {
        return Err(ExactProductError::EmptyModelExport);
    }
    Ok(nodes)
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

fn validate_cam_simulation_wire_evidence(
    request: &CamSimulationWireRequest,
    evidence: &CamSimulationWireEvidence,
) -> Result<(), WorkerError> {
    let volumes = [
        evidence.stock_before_mm3,
        evidence.stock_after_mm3,
        evidence.removed_stock_mm3,
        evidence.residual_stock_mm3,
        evidence.gouge_mm3,
    ];
    let expected_stock = (request.stock_bounds_mm[1][0] - request.stock_bounds_mm[0][0])
        * (request.stock_bounds_mm[1][1] - request.stock_bounds_mm[0][1])
        * (request.stock_bounds_mm[1][2] - request.stock_bounds_mm[0][2]);
    if evidence.schema != CAM_SIMULATION_SCHEMA_V1
        || evidence.plan_digest != request.plan_digest
        || evidence.toolpath_digest != request.toolpath_digest
        || evidence.target_exact_graph_digest != request.target_exact_graph_digest
        || evidence.motion_count != request.motions.len()
        || evidence.cutting_motion_count
            != request
                .motions
                .iter()
                .filter(|motion| matches!(motion.kind, 1 | 2))
                .count()
        || evidence.fixture_count != request.fixtures.len()
        || volumes
            .into_iter()
            .any(|value| !value.is_finite() || value < 0.0)
        || (evidence.stock_before_mm3 - expected_stock).abs() > expected_stock.max(1.0) * 1.0e-8
        || (evidence.stock_before_mm3 - evidence.stock_after_mm3 - evidence.removed_stock_mm3).abs()
            > evidence.stock_before_mm3.max(1.0) * 1.0e-8
        || evidence.backend.is_empty()
        || evidence.tolerance.is_empty()
        || !evidence.result_fingerprint.is_empty()
    {
        return Err(WorkerError::Protocol(
            "CAM simulation evidence does not match its sealed request".to_owned(),
        ));
    }
    let fixture_ids = request
        .fixtures
        .iter()
        .map(|fixture| fixture.id)
        .collect::<BTreeSet<_>>();
    let mut collision_keys = BTreeSet::new();
    for collision in &evidence.collisions {
        let valid_target = match (collision.target, collision.fixture_id) {
            (0, None) => true,
            (1, Some(id)) => fixture_ids.contains(&id),
            _ => false,
        };
        if collision.motion_index >= request.motions.len()
            || collision.motion_kind != request.motions[collision.motion_index].kind
            || collision.participant > 1
            || !valid_target
            || !collision.common_volume_mm3.is_finite()
            || collision.common_volume_mm3 < 0.0
            || !collision.contact_area_mm2.is_finite()
            || collision.contact_area_mm2 < 0.0
            || !collision.distance_mm.is_finite()
            || collision.distance_mm < 0.0
            || collision.common_volume_mm3 == 0.0
                && collision.contact_area_mm2 == 0.0
                && collision.distance_mm > 1.0e-7
            || !collision_keys.insert((
                collision.motion_index,
                collision.participant,
                collision.target,
                collision.fixture_id,
            ))
        {
            return Err(WorkerError::Protocol(
                "CAM collision evidence is incomplete or duplicated".to_owned(),
            ));
        }
    }
    Ok(())
}

fn cam_simulation_evidence(
    evidence: CamSimulationWireEvidence,
) -> Result<CamSimulationEvidence, WorkerError> {
    let collisions = evidence
        .collisions
        .into_iter()
        .map(|collision| {
            Ok(CamCollisionEvidence {
                motion_index: collision.motion_index,
                motion_kind: match collision.motion_kind {
                    0 => CamMotionKind::Rapid,
                    1 => CamMotionKind::Plunge,
                    2 => CamMotionKind::Cut,
                    3 => CamMotionKind::Retract,
                    _ => return Err(WorkerError::Protocol("unknown CAM motion kind".into())),
                },
                participant: match collision.participant {
                    0 => CamCollisionParticipant::Cutter,
                    1 => CamCollisionParticipant::Holder,
                    _ => return Err(WorkerError::Protocol("unknown CAM participant".into())),
                },
                target: match (collision.target, collision.fixture_id) {
                    (0, None) => CamCollisionTarget::Stock,
                    (1, Some(id)) => CamCollisionTarget::Fixture(id),
                    _ => return Err(WorkerError::Protocol("unknown CAM collision target".into())),
                },
                common_volume_mm3: collision.common_volume_mm3,
                contact_area_mm2: collision.contact_area_mm2,
                distance_mm: collision.distance_mm,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CamSimulationEvidence {
        schema: evidence.schema,
        plan_digest: evidence.plan_digest,
        toolpath_digest: evidence.toolpath_digest,
        target_exact_graph_digest: evidence.target_exact_graph_digest,
        motion_count: evidence.motion_count,
        cutting_motion_count: evidence.cutting_motion_count,
        fixture_count: evidence.fixture_count,
        stock_before_mm3: evidence.stock_before_mm3,
        stock_after_mm3: evidence.stock_after_mm3,
        removed_stock_mm3: evidence.removed_stock_mm3,
        residual_stock_mm3: evidence.residual_stock_mm3,
        gouge_mm3: evidence.gouge_mm3,
        collisions,
        backend: evidence.backend,
        tolerance: evidence.tolerance,
        result_fingerprint: evidence.result_fingerprint,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct PreparedExactBRepGraphSources {
    files: Vec<(String, tempfile::NamedTempFile)>,
}

impl PreparedExactBRepGraphSources {
    fn paths(&self) -> Vec<(&str, &Path)> {
        self.files
            .iter()
            .map(|(source_sha256, source)| (source_sha256.as_str(), source.path()))
            .collect()
    }
}

fn prepare_exact_brep_graph_sources(
    graph: &ExactBRepGraph,
    sources: &[&[u8]],
) -> Result<PreparedExactBRepGraphSources, WorkerError> {
    let mut expected = BTreeMap::<String, u64>::new();
    let mut source_order = Vec::new();
    for node in &graph.nodes {
        let ExactBRepOperation::ImportedExact {
            source_sha256,
            source_byte_len,
            ..
        } = &node.operation
        else {
            continue;
        };
        let source_sha256 = source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if let Some(previous_len) = expected.get(&source_sha256) {
            if previous_len != source_byte_len {
                return Err(WorkerError::Protocol(
                    "imported exact source identities disagree on byte length".to_owned(),
                ));
            }
        } else {
            expected.insert(source_sha256.clone(), *source_byte_len);
            source_order.push(source_sha256);
        }
    }
    if source_order.len() > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
        || sources.len() != source_order.len()
    {
        return Err(WorkerError::Protocol(
            "imported exact source count does not match the bounded graph identity".to_owned(),
        ));
    }
    let mut supplied = BTreeMap::<String, &[u8]>::new();
    let mut total_bytes = 0_u64;
    for source in sources {
        let source_len = source.len() as u64;
        total_bytes = total_bytes.checked_add(source_len).ok_or_else(|| {
            WorkerError::Protocol("imported exact sources exceed the bounded envelope".to_owned())
        })?;
        if source_len > MAX_STEP_SOURCE_BYTES
            || total_bytes > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES
            || supplied.insert(sha256_hex(source), *source).is_some()
        {
            return Err(WorkerError::Protocol(
                "imported exact sources exceed or duplicate the bounded envelope".to_owned(),
            ));
        }
    }
    let mut files = Vec::with_capacity(source_order.len());
    for source_sha256 in source_order {
        let expected_len = expected[&source_sha256];
        let Some(source) = supplied.remove(&source_sha256) else {
            return Err(WorkerError::Protocol(
                "imported exact source does not match its graph identity".to_owned(),
            ));
        };
        if source.len() as u64 != expected_len {
            return Err(WorkerError::Protocol(
                "imported exact source does not match its graph identity".to_owned(),
            ));
        }
        let mut source_file = tempfile::Builder::new()
            .prefix(".ketchup-brep-graph-source-")
            .suffix(".step")
            .tempfile()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        source_file
            .write_all(source)
            .and_then(|()| source_file.flush())
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        files.push((source_sha256, source_file));
    }
    if !supplied.is_empty() {
        return Err(WorkerError::Protocol(
            "imported exact source does not match its graph identity".to_owned(),
        ));
    }
    Ok(PreparedExactBRepGraphSources { files })
}

fn exact_brep_graph_sources_from_blobs<'a>(
    graph: &ExactBRepGraph,
    blobs: &'a BTreeMap<String, Vec<u8>>,
) -> Result<Vec<&'a [u8]>, WorkerError> {
    let mut sources = Vec::new();
    let mut seen = BTreeMap::new();
    for node in &graph.nodes {
        let ExactBRepOperation::ImportedExact { source_sha256, .. } = &node.operation else {
            continue;
        };
        let source_sha256 = source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if seen.insert(source_sha256.clone(), ()).is_some() {
            continue;
        }
        let source = blobs.get(&source_sha256).ok_or_else(|| {
            WorkerError::Protocol("imported exact source blob is unavailable".to_owned())
        })?;
        sources.push(source.as_slice());
    }
    Ok(sources)
}

fn append_exact_brep_graph_sources(request: &mut String, sources: &[(&str, &Path)]) {
    match sources {
        [] => {}
        [(source_sha256, source_path)] => {
            write!(
                request,
                " {source_sha256} {}",
                hex_encode(source_path.to_string_lossy().as_bytes())
            )
            .expect("writing to a String cannot fail");
        }
        sources => {
            write!(request, " {}", sources.len()).expect("writing to a String cannot fail");
            for (source_sha256, source_path) in sources {
                write!(
                    request,
                    " {source_sha256} {}",
                    hex_encode(source_path.to_string_lossy().as_bytes())
                )
                .expect("writing to a String cannot fail");
            }
        }
    }
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if !is_sha256_digest(value) {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (slot, pair) in digest.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(digest)
}

fn parse_import_body_kind(value: &str) -> Option<BodyKind> {
    match value {
        "solid" => Some(BodyKind::Solid),
        "surface" => Some(BodyKind::Surface),
        _ => None,
    }
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

fn parse_exact_brep_graph_step_acknowledgment(
    response: &str,
    expected_protocol: &str,
    graph_digest: &str,
    result_fingerprint: &str,
) -> Result<(), WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
        return Err(parse_error_response(response, &fields));
    }
    if fields != [expected_protocol, graph_digest, result_fingerprint] {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    Ok(())
}

fn parse_exact_brep_graph_result(
    response: &str,
    expected_protocol: &str,
) -> Result<WorkerExactBRepGraphResult, WorkerError> {
    let fields = response.split_whitespace().collect::<Vec<_>>();
    if matches!(fields.first(), Some(&"ERR") | Some(&"ERR_DETAIL")) {
        return Err(parse_error_response(response, &fields));
    }
    let (wire_count_index, topology_offset, faces_index, edges_index) =
        match (fields.first().copied(), fields.len()) {
            (Some(protocol), 21)
                if protocol == expected_protocol && protocol == "OK_BREP_GRAPH_V6" =>
            {
                (None, 0, None, None)
            }
            (Some(protocol), 23)
                if protocol == expected_protocol && protocol == "OK_BREP_GRAPH_V6" =>
            {
                (None, 0, Some(21), Some(22))
            }
            (Some(protocol), 22)
                if protocol == expected_protocol
                    && matches!(
                        protocol,
                        "OK_BREP_GRAPH_V8"
                            | "OK_BREP_GRAPH_V9"
                            | "OK_BREP_GRAPH_V10"
                            | "OK_BREP_GRAPH_V11"
                            | "OK_BREP_GRAPH_V12"
                            | "OK_BREP_GRAPH_V13"
                            | "OK_BREP_GRAPH_V14"
                            | "OK_BREP_GRAPH_V15"
                            | "OK_BREP_GRAPH_V16"
                            | "OK_BREP_GRAPH_V17"
                            | "OK_BREP_GRAPH_V18"
                            | "OK_BREP_GRAPH_V19"
                            | "OK_BREP_GRAPH_V20"
                            | "OK_BREP_GRAPH_V21"
                            | "OK_BREP_GRAPH_V22"
                            | "OK_BREP_GRAPH_V23"
                    ) =>
            {
                (Some(16), 1, None, None)
            }
            (
                Some(
                    protocol @ ("OK_BREP_GRAPH_V13" | "OK_BREP_GRAPH_V14" | "OK_BREP_GRAPH_V15"
                    | "OK_BREP_GRAPH_V16" | "OK_BREP_GRAPH_V17" | "OK_BREP_GRAPH_V18"
                    | "OK_BREP_GRAPH_V19" | "OK_BREP_GRAPH_V20" | "OK_BREP_GRAPH_V21"
                    | "OK_BREP_GRAPH_V22" | "OK_BREP_GRAPH_V23"),
                ),
                23,
            ) if expected_protocol == protocol => (Some(16), 1, Some(22), None),
            (Some(protocol), 24)
                if protocol == expected_protocol
                    && matches!(
                        protocol,
                        "OK_BREP_GRAPH_V8"
                            | "OK_BREP_GRAPH_V9"
                            | "OK_BREP_GRAPH_V10"
                            | "OK_BREP_GRAPH_V11"
                            | "OK_BREP_GRAPH_V12"
                            | "OK_BREP_GRAPH_V13"
                            | "OK_BREP_GRAPH_V14"
                            | "OK_BREP_GRAPH_V15"
                            | "OK_BREP_GRAPH_V16"
                            | "OK_BREP_GRAPH_V17"
                            | "OK_BREP_GRAPH_V18"
                            | "OK_BREP_GRAPH_V19"
                            | "OK_BREP_GRAPH_V20"
                            | "OK_BREP_GRAPH_V21"
                            | "OK_BREP_GRAPH_V22"
                            | "OK_BREP_GRAPH_V23"
                    ) =>
            {
                (Some(16), 1, Some(22), Some(23))
            }
            _ => return Err(WorkerError::Protocol(response.to_owned())),
        };
    if !is_sha256_digest(fields[1])
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
        area_mm2: parse_f64(7)?,
        bounds_mm: [
            parse_f64(8)?,
            parse_f64(9)?,
            parse_f64(10)?,
            parse_f64(11)?,
            parse_f64(12)?,
            parse_f64(13)?,
        ],
        topology_counts: [
            parse_u32(14)?,
            parse_u32(15)?,
            parse_u32(16 + topology_offset)?,
            parse_u32(17 + topology_offset)?,
            parse_u32(18 + topology_offset)?,
        ],
        wire_count: wire_count_index.map(parse_u32).transpose()?,
        backend: hex_decode_utf8(fields[19 + topology_offset])
            .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?,
        tolerance: hex_decode_utf8(fields[20 + topology_offset])
            .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?,
        faces: faces_index
            .map(|index| {
                let encoded = hex_decode_utf8(fields[index])
                    .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?;
                serde_json::from_str(&encoded)
                    .map_err(|_| WorkerError::Protocol(response.to_owned()))
            })
            .transpose()?
            .unwrap_or_default(),
        edges: edges_index
            .map(|index| {
                let encoded = hex_decode_utf8(fields[index])
                    .ok_or_else(|| WorkerError::Protocol(response.to_owned()))?;
                serde_json::from_str(&encoded)
                    .map_err(|_| WorkerError::Protocol(response.to_owned()))
            })
            .transpose()?
            .unwrap_or_default(),
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

#[cfg(test)]
mod request_timeout_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_worker_identity_rejects_relative_changed_and_oversized_files() {
        assert!(matches!(
            exact_worker_identity(Path::new("relative-worker")),
            Err(WorkerError::Spawn(message))
                if message == "exact worker executable must be absolute"
        ));

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("worker.bin");
        std::fs::write(&executable, b"trusted worker").unwrap();
        let (canonical, expected_sha256) = exact_worker_identity(&executable).unwrap();
        assert_eq!(canonical, executable.canonicalize().unwrap());

        std::fs::write(&executable, b"replaced worker").unwrap();
        assert!(matches!(
            verify_exact_worker_identity(&canonical, &expected_sha256),
            Err(WorkerError::Spawn(message))
                if message == "exact worker executable identity changed"
        ));

        let oversized = directory.path().join("oversized-worker.bin");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_EXACT_WORKER_EXECUTABLE_BYTES + 1)
            .unwrap();
        assert!(matches!(
            exact_worker_identity(&oversized),
            Err(WorkerError::Spawn(message))
                if message == "exact worker executable is not a bounded file"
        ));
    }

    #[test]
    fn worker_mesh_output_is_bounded_by_receipt_before_allocation() {
        let output = tempfile::NamedTempFile::new().unwrap();
        output.as_file().set_len(1024 * 1024).unwrap();

        assert!(matches!(
            read_step_import_mesh_output(output.path(), 1, 1),
            Err(WorkerError::Transport(message))
                if message == "exact worker mesh output does not match its bounded receipt"
        ));
        assert!(matches!(
            read_step_import_mesh_output(output.path(), MAX_STEP_MESH_VERTICES + 1, 1),
            Err(WorkerError::Protocol(message))
                if message == "exact worker mesh receipt exceeds its bounded envelope"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn verified_exact_worker_denies_replacement_until_spawn_guard_is_released() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("worker.exe");
        std::fs::write(&executable, b"trusted worker").unwrap();
        let (_, expected_sha256) = exact_worker_identity(&executable).unwrap();

        let guarded = verify_exact_worker_identity(&executable, &expected_sha256).unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&executable)
                .is_err()
        );
        assert!(std::fs::remove_file(&executable).is_err());

        drop(guarded);
        std::fs::write(&executable, b"replacement").unwrap();
        std::fs::remove_file(executable).unwrap();
    }

    #[test]
    fn cam_simulation_oracle_rejects_tampered_and_incomplete_worker_evidence() {
        let request = CamSimulationWireRequest {
            schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
            plan_digest: "a".repeat(64),
            toolpath_digest: "b".repeat(64),
            target_exact_graph_digest: "c".repeat(64),
            stock_bounds_mm: [[0.0; 3], [1.0; 3]],
            setup_to_world: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            cutter_radius_mm: 0.5,
            cutter_length_mm: 1.0,
            tool_length_mm: 2.0,
            holder_radius_mm: 1.0,
            holder_offset_mm: 2.0,
            holder_length_mm: 1.0,
            motions: vec![CamSimulationWireMotion {
                kind: 2,
                path_kind: 0,
                start_mm: [0.0; 3],
                end_mm: [1.0, 0.0, 0.0],
                center_mm: [0.0; 3],
                clockwise: false,
            }],
            fixtures: vec![],
        };
        let valid = CamSimulationWireEvidence {
            schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
            plan_digest: request.plan_digest.clone(),
            toolpath_digest: request.toolpath_digest.clone(),
            target_exact_graph_digest: request.target_exact_graph_digest.clone(),
            motion_count: 1,
            cutting_motion_count: 1,
            fixture_count: 0,
            stock_before_mm3: 1.0,
            stock_after_mm3: 0.5,
            removed_stock_mm3: 0.5,
            residual_stock_mm3: 0.2,
            gouge_mm3: 0.0,
            collisions: vec![],
            backend: "occt-test".into(),
            tolerance: "bounded-test".into(),
            result_fingerprint: String::new(),
        };
        assert!(validate_cam_simulation_wire_evidence(&request, &valid).is_ok());

        let mut tampered = valid.clone();
        tampered.removed_stock_mm3 = 0.4;
        assert!(validate_cam_simulation_wire_evidence(&request, &tampered).is_err());
        let mut incomplete = valid.clone();
        incomplete.motion_count = 0;
        assert!(validate_cam_simulation_wire_evidence(&request, &incomplete).is_err());
        let mut forged_fingerprint = valid;
        forged_fingerprint.result_fingerprint = "d".repeat(64);
        assert!(validate_cam_simulation_wire_evidence(&request, &forged_fingerprint).is_err());
    }

    fn valid_exact_brep_graph_response(protocol: &str) -> String {
        let mut fields = vec![
            protocol.to_owned(),
            "a".repeat(64),
            "b".repeat(64),
            "1".to_owned(),
            "fnv1a64:0123456789abcdef".to_owned(),
            "fnv1a64:fedcba9876543210".to_owned(),
        ];
        fields.extend((0..8).map(|_| "0000000000000000".to_owned()));
        fields.extend((0..6).map(|_| "1".to_owned()));
        fields.extend(["62".to_owned(), "74".to_owned()]);
        fields.join(" ")
    }

    #[test]
    fn v13_graph_client_rejects_otherwise_valid_v12_result_token() {
        let response = valid_exact_brep_graph_response("OK_BREP_GRAPH_V12");
        assert!(parse_exact_brep_graph_result(&response, "OK_BREP_GRAPH_V12").is_ok());
        assert!(matches!(
            parse_exact_brep_graph_result(&response, "OK_BREP_GRAPH_V13"),
            Err(WorkerError::Protocol(rejected)) if rejected == response
        ));
    }

    #[test]
    fn step_v4_client_rejects_otherwise_valid_v3_acknowledgment() {
        let graph_digest = "a".repeat(64);
        let result_fingerprint = "fnv1a64:0123456789abcdef";
        let response = format!("OK_BREP_GRAPH_STEP_V3 {graph_digest} {result_fingerprint}");
        assert!(
            parse_exact_brep_graph_step_acknowledgment(
                &response,
                "OK_BREP_GRAPH_STEP_V3",
                &graph_digest,
                result_fingerprint,
            )
            .is_ok()
        );
        assert!(matches!(
            parse_exact_brep_graph_step_acknowledgment(
                &response,
                "OK_BREP_GRAPH_STEP_V4",
                &graph_digest,
                result_fingerprint,
            ),
            Err(WorkerError::Protocol(rejected)) if rejected == response
        ));
    }
}
