use ketchup_application::cam_workflow::{
    CamReviewError, CamReviewRequest, CamReviewSummary, CamReviewWorkflow,
};
use ketchup_application::evaluation::{
    EvaluationReport, EvidenceStatus, ExactEvaluationProgress, ExactEvaluationTask, ExactSource,
    ProducerKey, exact_worker_candidates,
};
use ketchup_application::fea_workflow::{
    ExactFeaFaceTraction, ExactFeaSetup, ExactVolumeMeshWireOptions, FeaReviewError,
    FeaReviewSummary, FeaReviewWorkflow, FeaStudyRequest,
};
use ketchup_application::pdm_workflow::{
    LocalPdmWorkflow, PdmCreateReleaseRequest, PdmDocumentState, PdmWorkflowError,
};
mod model_tools;
mod production;
use ketchup_application::batch_task::{
    OccurrenceBatchError, OccurrenceBatchOperation, OccurrenceBatchState, OccurrenceBatchTask,
};
use ketchup_application::model_query::{ModelQuery, created_receipt};
use ketchup_application::validation::{ASSISTANT_VALIDATOR_IDS, assistant_validator_catalog};
use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionError, SessionSettings,
};
use ketchup_core::assistant_sidecar::AssistantCadEditProgram;
use ketchup_core::cam::{
    CamFixture, CamOperation, CamPath2d, CamPathSegment2d, CamPlanId, CamPostprocessorDialect,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, InstancePath, OccurrenceId, Snapshot,
};
use ketchup_core::exact_product::{ExactBodyPackage, ExactResultRegistry};
use ketchup_core::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_core::local_pdm::{
    DependencyChangeKind, LocalPdmError, ReleaseAudit, ReleaseCatalogEntry, ReleaseComparison,
    ReleaseConflictVerdict, ReleaseDependencyInput, ReleaseManifest, ReleaseRelationship,
    VerifiedRelease,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeSet, VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};

pub const PROTOCOL: &str = "ketchup.headless.v1";
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BATCH_JOBS: usize = 16;
pub const MAX_VERIFY_JOBS: usize = 16;
const METHODS: &[&str] = &[
    "capabilities",
    "new",
    "open",
    "state",
    "production_codes",
    "summary",
    "query",
    "detail",
    "workset_create",
    "workset_status",
    "batch_job_start",
    "batch_job_status",
    "batch_job_step",
    "batch_job_cancel",
    "apply",
    "set_production_codes",
    "production_job",
    "cam_preview",
    "cam_export",
    "fea_review",
    "pdm_release_create",
    "pdm_release_open",
    "pdm_catalog",
    "pdm_compare",
    "evaluate",
    "verify_job_start",
    "verify_job_status",
    "verify_job_cancel",
    "list_validators",
    "run_validators",
    "set_grounded",
    "undo",
    "redo",
    "save",
];
const GUARDED_METHODS: &[&str] = &[
    "new",
    "open",
    "apply",
    "set_production_codes",
    "production_job",
    "cam_preview",
    "cam_export",
    "fea_review",
    "pdm_release_create",
    "pdm_release_open",
    "pdm_catalog",
    "pdm_compare",
    "verify_job_start",
    "batch_job_start",
    "batch_job_step",
    "set_grounded",
    "undo",
    "redo",
    "save",
];

fn method_requires_guard(method: &str) -> bool {
    GUARDED_METHODS.contains(&method)
}

#[derive(Debug)]
struct Error {
    code: String,
    message: String,
    details: Option<Value>,
}
impl Error {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_params", message)
    }
}
impl From<CamReviewError> for Error {
    fn from(error: CamReviewError) -> Self {
        let code = match &error {
            CamReviewError::PlanMissing => "cam_plan_missing",
            CamReviewError::WorkerUnavailable => "exact_worker_unavailable",
            CamReviewError::Planning(_) => "cam_planning_rejected",
            CamReviewError::Simulation(_) => "cam_simulation_rejected",
            CamReviewError::Postprocessing(_) => "cam_postprocessing_rejected",
            CamReviewError::ReviewLimit => "cam_review_limit",
            CamReviewError::InvalidToken => "cam_review_token_invalid",
            CamReviewError::StaleReview => "stale_state",
            CamReviewError::SubstitutedEvidence => "cam_review_substituted",
            CamReviewError::ConfirmationRequired => "confirmation_required",
            CamReviewError::InvalidPath => "invalid_path",
            CamReviewError::FileExists => "file_exists",
            CamReviewError::Write(_) => "io_error",
        };
        Self::new(code, error.to_string())
    }
}

impl From<FeaReviewError> for Error {
    fn from(error: FeaReviewError) -> Self {
        let code = match &error {
            FeaReviewError::ConfirmationRequired => "confirmation_required",
            FeaReviewError::WorkerUnavailable => "exact_worker_unavailable",
            FeaReviewError::InvalidRefinementLevels => "fea_refinement_invalid",
            FeaReviewError::InvalidTarget(_) => "fea_target_rejected",
            FeaReviewError::Meshing(_) => "fea_meshing_rejected",
            FeaReviewError::Setup(_) => "fea_setup_rejected",
            FeaReviewError::Solve(_) => "fea_solve_rejected",
        };
        Self::new(code, error.to_string())
    }
}

impl From<PdmWorkflowError> for Error {
    fn from(error: PdmWorkflowError) -> Self {
        let code = match &error {
            PdmWorkflowError::ConfirmationRequired => "confirmation_required",
            PdmWorkflowError::Cancelled => "cancelled",
            PdmWorkflowError::StaleState => "stale_state",
            PdmWorkflowError::Core(error) => match error {
                LocalPdmError::Io(_) | LocalPdmError::Json(_) | LocalPdmError::Persistence(_) => {
                    "pdm_io_error"
                }
                LocalPdmError::StaleSnapshot => "stale_state",
                LocalPdmError::InvalidAudit => "pdm_audit_invalid",
                LocalPdmError::TooManyDependencies
                | LocalPdmError::DependencyTooLarge
                | LocalPdmError::DependenciesTooLarge => "pdm_dependency_limit",
                LocalPdmError::TooManyReleases | LocalPdmError::LineageTooDeep => "pdm_limit",
                LocalPdmError::ParentDocumentMismatch => "pdm_parent_document_mismatch",
                LocalPdmError::NoChangesAgainstParent => "pdm_no_changes",
                LocalPdmError::InvalidLogicalPath | LocalPdmError::DuplicateLogicalPath => {
                    "pdm_dependency_invalid"
                }
                LocalPdmError::InvalidReleaseId
                | LocalPdmError::ManifestTooLarge
                | LocalPdmError::InvalidManifest
                | LocalPdmError::ManifestIdentityMismatch => "pdm_manifest_invalid",
                LocalPdmError::InvalidRepositoryPath => "pdm_repository_invalid",
                LocalPdmError::ReleaseAlreadyExists => "pdm_release_exists",
                LocalPdmError::MissingRelease { .. } => "pdm_release_missing",
                LocalPdmError::MissingObject { .. }
                | LocalPdmError::ObjectTampered { .. }
                | LocalPdmError::ObjectConflict { .. }
                | LocalPdmError::DocumentIdentityMismatch => "pdm_verification_failed",
            },
        };
        Self::new(code, error.to_string())
    }
}

impl From<SessionError> for Error {
    fn from(error: SessionError) -> Self {
        if let SessionError::Planning(diagnostic) = error {
            return Self {
                code: diagnostic.code.clone(),
                message: diagnostic.failed_invariant.clone(),
                details: Some(json!(diagnostic)),
            };
        }
        let code = match &error {
            SessionError::Canonical(e) => e.code(),
            SessionError::Prepare(_) => "proposal_prepare_rejected",
            SessionError::Commit(_) => "proposal_commit_rejected",
            SessionError::Persistence(_) => "persistence_error",
            SessionError::ReviewOnly => "review_only",
            SessionError::NoUndo => "no_undo",
            SessionError::NoRedo => "no_redo",
            SessionError::Evaluation(_) => "evaluation_error",
            SessionError::Planning(_) => unreachable!(),
        };
        Self {
            code: code.into(),
            message: error.to_string(),
            details: Some(json!({"diagnostic":format!("{error:?}")})),
        }
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: String,
    id: Value,
    method: String,
    params: Value,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CamOperationInput {
    Face {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        target_z_mm: f64,
    },
    Pocket {
        id: u64,
        minimum_mm: [f64; 2],
        maximum_mm: [f64; 2],
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
    Contour {
        id: u64,
        center_path: CamPathInput,
        top_z_mm: f64,
        bottom_z_mm: f64,
        applied_radial_allowance_mm: f64,
    },
    Drill {
        id: u64,
        points_mm: Vec<[f64; 2]>,
        top_z_mm: f64,
        bottom_z_mm: f64,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CamPathInput {
    start_mm: [f64; 2],
    segments: Vec<CamPathSegmentInput>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum CamPathSegmentInput {
    Line {
        to_mm: [f64; 2],
    },
    Arc {
        to_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CamFixtureInput {
    id: u64,
    minimum_mm: [f64; 3],
    maximum_mm: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaReviewInput {
    definition_id: u64,
    feature_id: u64,
    occurrence_id: u64,
    case_id: String,
    material: FeaMaterialInput,
    constrained_face_ordinals: Vec<u32>,
    face_tractions: Vec<FeaFaceTractionInput>,
    mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    solve_settings: FeaSolveSettingsInput,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaMaterialInput {
    id: u64,
    youngs_modulus_mpa: f64,
    poisson_ratio: f64,
    yield_strength_mpa: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaFaceTractionInput {
    face_ordinal: u32,
    traction_local_n_per_mm2: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaSolveSettingsInput {
    maximum_nodes: usize,
    relative_pivot_tolerance: f64,
    maximum_small_deformation_ratio: f64,
    convergence_relative_tolerance: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PdmDependencyInput {
    logical_path: String,
    source_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PdmAuditInput {
    actor: String,
    created_unix_ms: u64,
    note: String,
}

impl From<CamOperationInput> for CamOperation {
    fn from(operation: CamOperationInput) -> Self {
        match operation {
            CamOperationInput::Face {
                id,
                minimum_mm,
                maximum_mm,
                target_z_mm,
            } => Self::Face {
                id,
                minimum_mm,
                maximum_mm,
                target_z_mm,
            },
            CamOperationInput::Pocket {
                id,
                minimum_mm,
                maximum_mm,
                top_z_mm,
                bottom_z_mm,
            } => Self::Pocket {
                id,
                minimum_mm,
                maximum_mm,
                top_z_mm,
                bottom_z_mm,
            },
            CamOperationInput::Contour {
                id,
                center_path,
                top_z_mm,
                bottom_z_mm,
                applied_radial_allowance_mm,
            } => Self::Contour {
                id,
                center_path: CamPath2d {
                    start_mm: center_path.start_mm,
                    segments: center_path
                        .segments
                        .into_iter()
                        .map(|segment| match segment {
                            CamPathSegmentInput::Line { to_mm } => CamPathSegment2d::Line { to_mm },
                            CamPathSegmentInput::Arc {
                                to_mm,
                                center_mm,
                                clockwise,
                            } => CamPathSegment2d::Arc {
                                to_mm,
                                center_mm,
                                clockwise,
                            },
                        })
                        .collect(),
                },
                top_z_mm,
                bottom_z_mm,
                applied_radial_allowance_mm,
            },
            CamOperationInput::Drill {
                id,
                points_mm,
                top_z_mm,
                bottom_z_mm,
            } => Self::Drill {
                id,
                points_mm,
                top_z_mm,
                bottom_z_mm,
            },
        }
    }
}

struct BatchJob {
    handle: String,
    task: OccurrenceBatchTask,
}

enum VerifyJobState {
    Running(ExactEvaluationTask),
    Completed(Value),
    Failed(String),
    TimedOut(Value),
    Cancelled,
}

struct VerifyJob {
    handle: String,
    source: ExactSource,
    scope: Option<Vec<ProducerKey>>,
    progress: ExactEvaluationProgress,
    started_at: Instant,
    elapsed_ms: u64,
    timeout_ms: u64,
    deadline_stop: Option<std::sync::mpsc::Sender<()>>,
    state: VerifyJobState,
}

impl VerifyJob {
    fn terminal(&self) -> bool {
        !matches!(self.state, VerifyJobState::Running(_))
    }
}

pub struct Server {
    session: DocumentSession,
    settings: SessionSettings,
    // The process starts with an internal placeholder session so its first New/Open needs no discard.
    initial_placeholder: bool,
    model_queries: ModelQuery,
    batch_jobs: VecDeque<BatchJob>,
    batch_job_key: RandomState,
    next_batch_job: u64,
    verify_jobs: VecDeque<VerifyJob>,
    verify_job_key: RandomState,
    next_verify_job: u64,
    cam_reviews: CamReviewWorkflow,
    fea_reviews: FeaReviewWorkflow,
    pdm: LocalPdmWorkflow,
    compact_result: bool,
}
impl Server {
    pub fn new(settings: SessionSettings) -> Self {
        let worker_path = settings.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        Self {
            session: DocumentSession::new(settings.clone()),
            settings,
            initial_placeholder: true,
            model_queries: ModelQuery::default(),
            batch_jobs: VecDeque::new(),
            batch_job_key: RandomState::new(),
            next_batch_job: 1,
            verify_jobs: VecDeque::new(),
            verify_job_key: RandomState::new(),
            next_verify_job: 1,
            cam_reviews: CamReviewWorkflow::new(worker_path.clone()),
            fea_reviews: FeaReviewWorkflow::new(worker_path),
            pdm: LocalPdmWorkflow::new(),
            compact_result: false,
        }
    }
    fn batch_job_handle(&self, id: u64) -> String {
        format!("batch-{id:016x}-{:016x}", self.batch_job_key.hash_one(id))
    }

    fn verify_job_handle(&self, id: u64) -> String {
        format!("verify-{id:016x}-{:016x}", self.verify_job_key.hash_one(id))
    }

    fn revoke_jobs(&mut self) {
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.verify_jobs.clear();
        self.verify_job_key = RandomState::new();
        self.next_verify_job = 1;
        self.cam_reviews.revoke();
    }

    fn start_verify_job(
        &mut self,
        scope: Option<BTreeSet<ProducerKey>>,
        timeout_ms: u64,
    ) -> Result<Value> {
        if self.verify_jobs.len() == MAX_VERIFY_JOBS {
            for index in 0..self.verify_jobs.len() {
                self.refresh_verify_job(index);
            }
            let terminal = self
                .verify_jobs
                .iter()
                .position(VerifyJob::terminal)
                .ok_or_else(|| {
                    Error::new(
                        "verify_job_limit",
                        "all bounded Verify job slots are active",
                    )
                })?;
            self.verify_jobs.remove(terminal);
        }
        let id = self.next_verify_job;
        self.next_verify_job = id
            .checked_add(1)
            .ok_or_else(|| Error::new("verify_job_ids_exhausted", "Verify job IDs exhausted"))?;
        let handle = self.verify_job_handle(id);
        let started_at = Instant::now();
        let task = self
            .session
            .start_scoped_exact_evaluation_task(scope.as_ref());
        let source = task.source.clone();
        let progress = task.progress();
        let deadline_cancelled = std::sync::Arc::clone(&task.cancelled);
        let deadline_finished = std::sync::Arc::clone(&task.finished);
        let (deadline_stop, deadline_wait) = std::sync::mpsc::channel();
        let remaining =
            std::time::Duration::from_millis(timeout_ms).saturating_sub(started_at.elapsed());
        std::thread::spawn(move || {
            if matches!(
                deadline_wait.recv_timeout(remaining),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) && !deadline_finished.load(std::sync::atomic::Ordering::Acquire)
            {
                deadline_cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
        });
        self.verify_jobs.push_back(VerifyJob {
            handle,
            source,
            scope: scope.map(|scope| scope.into_iter().collect()),
            progress,
            started_at,
            elapsed_ms: 0,
            timeout_ms,
            deadline_stop: Some(deadline_stop),
            state: VerifyJobState::Running(task),
        });
        Ok(self.verify_job_value(self.verify_jobs.len() - 1))
    }

    fn time_out_verify_job(&mut self, index: usize, progress: ExactEvaluationProgress) {
        if let VerifyJobState::Running(task) = &self.verify_jobs[index].state {
            task.cancel();
        }
        let active = progress.active_producer.map(|key| {
            json!({
                "definition_id":key.definition_id.0,
                "feature_id":key.feature_id.0,
                "elapsed_ms":progress.active_elapsed_ms,
            })
        });
        let remaining = progress
            .total_producers
            .saturating_sub(progress.completed_producers);
        self.verify_jobs[index].state = VerifyJobState::TimedOut(json!({
            "code":"verify_timeout",
            "message":"exact Verify exceeded its native deadline",
            "elapsed_ms":self.verify_jobs[index].elapsed_ms,
            "timeout_ms":self.verify_jobs[index].timeout_ms,
            "active_body":active,
            "completed_bodies":progress.completed_producers,
            "remaining_bodies":remaining,
        }));
    }

    fn refresh_verify_job(&mut self, index: usize) {
        let (progress, result) = match &self.verify_jobs[index].state {
            VerifyJobState::Running(task) => (task.progress(), Some(task.poll())),
            _ => return,
        };
        self.verify_jobs[index].progress = progress;
        self.verify_jobs[index].elapsed_ms = self.verify_jobs[index]
            .started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        match result.expect("running Verify job has a poll result") {
            Ok(Ok(products)) => {
                let published = {
                    let VerifyJobState::Running(task) = &self.verify_jobs[index].state else {
                        unreachable!("Verify job state changed while polling")
                    };
                    self.session.publish_exact_evaluation(task, products)
                };
                self.verify_jobs[index].state = match published {
                    Ok(report) => {
                        VerifyJobState::Completed(evaluation_report(&self.session, &report))
                    }
                    Err(error) => VerifyJobState::Failed(error.to_string()),
                };
            }
            Ok(Err(reason)) => self.verify_jobs[index].state = VerifyJobState::Failed(reason),
            Err(std::sync::mpsc::TryRecvError::Empty)
                if self.verify_jobs[index].elapsed_ms >= self.verify_jobs[index].timeout_ms =>
            {
                self.time_out_verify_job(index, progress);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected)
                if self.verify_jobs[index].elapsed_ms >= self.verify_jobs[index].timeout_ms =>
            {
                self.time_out_verify_job(index, progress);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.verify_jobs[index].state =
                    VerifyJobState::Failed("exact evaluation worker disconnected".into());
            }
        }
        if self.verify_jobs[index].terminal()
            && let Some(stop) = self.verify_jobs[index].deadline_stop.take()
        {
            let _ = stop.send(());
        }
        if matches!(
            &self.verify_jobs[index].state,
            VerifyJobState::Completed(_) | VerifyJobState::Failed(_)
        ) {
            self.verify_jobs[index].progress.active_producer = None;
            self.verify_jobs[index].progress.active_elapsed_ms = None;
        }
    }

    fn verify_job_value(&self, index: usize) -> Value {
        let job = &self.verify_jobs[index];
        let mut value = json!({
            "job_handle":job.handle,
            "source":{"document_id":job.source.0.0,"revision":job.source.1,"canonical_digest":job.source.2},
            "scope":job.scope.as_ref().map(|scope| scope.iter().map(|key| json!({"definition_id":key.definition_id.0,"feature_id":key.feature_id.0})).collect::<Vec<_>>()),
            "progress":{"total_bodies":job.progress.total_producers,"completed_bodies":job.progress.completed_producers,"reused_bodies":job.progress.reused_producers,
                "active_body":job.progress.active_producer.map(|key| json!({"definition_id":key.definition_id.0,"feature_id":key.feature_id.0,"elapsed_ms":job.progress.active_elapsed_ms}))},
            "timing":{"elapsed_ms":job.elapsed_ms,"timeout_ms":job.timeout_ms},
            "state":match job.state {
                VerifyJobState::Running(_) => "running",
                VerifyJobState::Completed(_) => "completed",
                VerifyJobState::Failed(_) => "failed",
                VerifyJobState::TimedOut(_) => "timed_out",
                VerifyJobState::Cancelled => "cancelled",
            }
        });
        match &job.state {
            VerifyJobState::Completed(report) => value["report"] = report.clone(),
            VerifyJobState::Failed(reason) => value["diagnostic"] = json!(reason),
            VerifyJobState::TimedOut(diagnostic) => value["diagnostic"] = diagnostic.clone(),
            VerifyJobState::Running(_) | VerifyJobState::Cancelled => {}
        }
        value
    }

    fn state(&self) -> Value {
        let s = self.session.snapshot();
        json!({"document_id":s.document_id().0,"revision":s.revision_id(),"canonical_digest":s.canonical_digest(),
            "mutation_epoch":self.session.mutation_epoch(),
            "undo_steps":self.session.visible_undo_steps(),"redo_steps":self.session.visible_redo_steps(),
            "definitions":s.definitions().map(|d| json!({"id":d.id().0,"name":d.name(),"feature_ids":d.feature_ids().iter().map(|id|id.0).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "occurrences":s.occurrences().map(|o| json!({"id":o.id().0,"definition_id":o.definition_id().0,"name":o.name(),"transform":o.transform().matrix(),"color":o.color()})).collect::<Vec<_>>(),
            "features":s.features().map(|f| json!({"id":f.id().0,"definition_id":f.definition_id().0,"name":f.name(),"kind":format!("{:?}",f.kind()).split([' ', '{', '(']).next().unwrap_or("unknown")})).collect::<Vec<_>>(),
            "grounded_occurrence_ids":s.grounded_occurrences().map(|id|id.0).collect::<Vec<_>>()})
    }
    fn state_result(&self) -> Value {
        if self.compact_result {
            return self.compact_state_result();
        }
        json!({"state":self.state(),"path":self.session.path().map(|p|p.to_string_lossy()),
            "recovery":self.session.recovery_state().map(|recovery| json!({
                "requested_path":recovery.requested_path().to_string_lossy(),
                "source_path":recovery.source_path().to_string_lossy(),
                "save_as_required":true
            })),
            "modified":self.session.is_modified()})
    }
    fn guard(&self, p: &Map<String, Value>) -> Result<()> {
        let revision = uint(p, "expected_revision")?;
        let digest = string(p, "expected_digest")?;
        let mutation_epoch = uint(p, "expected_mutation_epoch")?;
        let s = self.session.snapshot();
        if revision != s.revision_id()
            || digest != s.canonical_digest()
            || mutation_epoch != self.session.mutation_epoch()
        {
            return Err(Error {
                code: "stale_state".into(),
                message: "expected revision/digest/mutation_epoch does not match observed document"
                    .into(),
                details: Some(
                    json!({"revision":s.revision_id(),"canonical_digest":s.canonical_digest(),"mutation_epoch":self.session.mutation_epoch(),"repair_hint":"Read state and explicitly re-plan; do not blindly retry a mutation."}),
                ),
            });
        }
        Ok(())
    }
    fn discard_guard(&self, p: &Map<String, Value>) -> Result<()> {
        if !boolean(p, "discard_unsaved", false)?
            && !self.initial_placeholder
            && self.session.is_modified()
        {
            return Err(Error::new(
                "unsaved_changes",
                "new/open requires discard_unsaved=true for unsaved changes",
            ));
        }
        Ok(())
    }
    fn dispatch_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let p = params
            .as_object()
            .ok_or_else(|| Error::invalid("params must be an object"))?;
        let (fields, mutation): (&[&str], bool) = match method {
            "capabilities" | "state" | "production_codes" | "list_validators" => (&[], false),
            "new" => (&["discard_unsaved"], true),
            "open" => (&["path", "discard_unsaved"], true),
            "apply" => (&["program", "selection"], true),
            "set_production_codes" => (&["assignments"], true),
            "production_job" => (
                &["adapters", "vertical_pocket_tool_number", "timeout_ms"],
                true,
            ),
            "cam_preview" => (&["plan_id", "operations", "fixtures", "dialect"], true),
            "cam_export" => (&["review_token", "path", "confirmed"], true),
            "fea_review" => (
                &[
                    "definition_id",
                    "feature_id",
                    "occurrence_id",
                    "case_id",
                    "material",
                    "constrained_face_ordinals",
                    "face_tractions",
                    "mesh_levels",
                    "solve_settings",
                    "confirmed",
                ],
                true,
            ),
            "pdm_release_create" => (
                &[
                    "repository",
                    "parent_release_id",
                    "dependencies",
                    "audit",
                    "confirmed",
                ],
                true,
            ),
            "pdm_release_open" => (&["repository", "release_id"], true),
            "pdm_catalog" => (&["repository"], true),
            "pdm_compare" => (&["repository", "left_release_id", "right_release_id"], true),
            "evaluate" => (&["timeout_ms"], false),
            "run_validators" => (&["ids"], false),
            "set_grounded" => (&["occurrence_ids", "grounded"], true),
            "undo" | "redo" => (&[], true),
            "save" => (&["path", "overwrite"], true),
            _ => {
                return Err(Error::new(
                    "unknown_method",
                    format!("unknown method {method}"),
                ));
            }
        };
        for key in p.keys() {
            if !fields.contains(&key.as_str())
                && !(mutation
                    && [
                        "expected_revision",
                        "expected_digest",
                        "expected_mutation_epoch",
                    ]
                    .contains(&key.as_str()))
            {
                return Err(Error::invalid(format!("unknown field {key}")));
            }
        }
        debug_assert_eq!(mutation, method_requires_guard(method));
        match method {
            "capabilities" => Ok(
                json!({"methods":METHODS.iter().map(|name| json!({"name":name,"mutates":method_requires_guard(name)})).collect::<Vec<_>>(),
                "cad_program_schema":serde_json::from_str::<Value>(include_str!(concat!(env!("OUT_DIR"),"/cad-program-schema.json"))).expect("build-generated schema"),
                "bounds":{"max_line_bytes":MAX_LINE_BYTES,"max_output_bytes":MAX_LINE_BYTES,"max_selection":100,"max_operations":64,"max_batch_jobs":MAX_BATCH_JOBS,"max_verify_jobs":MAX_VERIFY_JOBS,"evaluation_timeout_ms":{"default":30000,"min":1,"max":300000}},
                "mutation_preconditions":["expected_revision","expected_digest","expected_mutation_epoch"],"units":"mm","transform":"row-major 4x4 local occurrence transform","transactions":"one apply = one atomic CAD program; newly allocated Definition, Sketch and body references use zero-based earlier operation_index plus a typed output, never guessed IDs","protocol":PROTOCOL}),
            ),
            "state" => Ok(self.state_result()),
            "production_codes" => Ok(self.production_codes()),
            "set_production_codes" => self.set_production_codes(p),
            "production_job" => self.production_job(p),
            "new" => {
                self.discard_guard(p)?;
                self.session = DocumentSession::new(self.settings.clone());
                self.revoke_jobs();
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "open" => {
                self.discard_guard(p)?;
                let next = DocumentSession::open(string(p, "path")?, self.settings.clone())?;
                self.session = next;
                self.revoke_jobs();
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "save" => {
                self.session.save(
                    string(p, "path")?,
                    SaveOptions {
                        overwrite: boolean(p, "overwrite", false)?,
                    },
                )?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "apply" => {
                let program: AssistantCadEditProgram = serde_json::from_value(
                    p.get("program")
                        .cloned()
                        .ok_or_else(|| Error::invalid("missing program"))?,
                )
                .map_err(|e| Error::invalid(e.to_string()))?;
                let selection = ids(p, "selection", true)?
                    .into_iter()
                    .map(OccurrenceId)
                    .collect();
                let before = self.session.snapshot();
                self.session.apply_cad_program(&program, &selection)?;
                self.initial_placeholder = false;
                let mut result = self.state_result();
                result["created"] = if self.compact_result {
                    created_receipt(&before, &self.session.snapshot())
                } else {
                    created(&before, &self.session.snapshot())
                };
                Ok(result)
            }
            "cam_preview" => {
                let request = cam_review_request(p)?;
                let snapshot = self.session.snapshot();
                let mutation_epoch = self.session.mutation_epoch();
                let cancelled = AtomicBool::new(false);
                let review =
                    self.cam_reviews
                        .preview(&snapshot, mutation_epoch, request, &cancelled)?;
                Ok(cam_review_value(&review))
            }
            "cam_export" => {
                let snapshot = self.session.snapshot();
                let mutation_epoch = self.session.mutation_epoch();
                let token = string(p, "review_token")?;
                let path = Path::new(string(p, "path")?);
                let confirmed = boolean(p, "confirmed", false)?;
                let cancelled = AtomicBool::new(false);
                let review = self.cam_reviews.export(
                    &snapshot,
                    mutation_epoch,
                    token,
                    path,
                    confirmed,
                    &cancelled,
                )?;
                Ok(json!({"exported":true,"path":path,"review":cam_review_value(&review)}))
            }
            "fea_review" => {
                let (request, confirmed) = fea_review_request(p)?;
                let snapshot = self.session.snapshot();
                let cancelled = AtomicBool::new(false);
                let review = self
                    .fea_reviews
                    .review(&snapshot, &request, confirmed, &cancelled)?;
                Ok(fea_review_value(&review))
            }
            "pdm_release_create" => {
                let current = PdmDocumentState::observed(
                    self.session.snapshot(),
                    self.session.mutation_epoch(),
                );
                let source = current.source_identity();
                let request = pdm_create_request(p)?;
                let manifest = self.pdm.create(
                    &current,
                    self.session.container_data(),
                    &source,
                    &request,
                    boolean(p, "confirmed", false)?,
                    &AtomicBool::new(false),
                )?;
                Ok(
                    json!({"created":true,"manifest":pdm_manifest_value(&manifest),"document_mutated":false}),
                )
            }
            "pdm_release_open" => {
                let current = PdmDocumentState::observed(
                    self.session.snapshot(),
                    self.session.mutation_epoch(),
                );
                let source = current.source_identity();
                let release = self.pdm.open(
                    &current,
                    &source,
                    string(p, "repository")?,
                    string(p, "release_id")?,
                    &AtomicBool::new(false),
                )?;
                Ok(pdm_verified_release_value(&release, current.snapshot()))
            }
            "pdm_catalog" => {
                let current = PdmDocumentState::observed(
                    self.session.snapshot(),
                    self.session.mutation_epoch(),
                );
                let source = current.source_identity();
                let catalog = self.pdm.catalog(
                    &current,
                    &source,
                    string(p, "repository")?,
                    &AtomicBool::new(false),
                )?;
                Ok(
                    json!({"releases":catalog.iter().map(pdm_catalog_entry_value).collect::<Vec<_>>(),"document_mutated":false}),
                )
            }
            "pdm_compare" => {
                let current = PdmDocumentState::observed(
                    self.session.snapshot(),
                    self.session.mutation_epoch(),
                );
                let source = current.source_identity();
                let comparison = self.pdm.compare(
                    &current,
                    &source,
                    string(p, "repository")?,
                    string(p, "left_release_id")?,
                    string(p, "right_release_id")?,
                    &AtomicBool::new(false),
                )?;
                Ok(pdm_comparison_value(&comparison))
            }
            "set_grounded" => {
                let selected = ids(p, "occurrence_ids", false)?;
                let grounded = p
                    .get("grounded")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| Error::invalid("grounded must be boolean"))?;
                let batch = CommandBatch::new(
                    selected
                        .into_iter()
                        .map(|id| CanonicalCommand::SetOccurrenceGrounded {
                            id: OccurrenceId(id),
                            grounded,
                        })
                        .collect(),
                );
                // This is the only canonical-command adapter, not a raw command endpoint.
                let proposal = self.session.plan_commands(batch)?;
                self.session.apply_proposal(&proposal)?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "undo" => {
                self.session.undo()?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "redo" => {
                self.session.redo()?;
                self.initial_placeholder = false;
                Ok(self.state_result())
            }
            "list_validators" => Ok(json!({"validators":assistant_validator_catalog()})),
            "run_validators" => {
                let names = p
                    .get("ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::invalid("ids must be an array"))?;
                let names = names
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| Error::invalid("validator id must be string"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if names.is_empty()
                    || names.len() > ASSISTANT_VALIDATOR_IDS.len()
                    || names.iter().collect::<BTreeSet<_>>().len() != names.len()
                {
                    return Err(Error::invalid("validator ids must be nonempty and unique"));
                }
                let selection = AssistantValidationSelection::only(&names);
                if !selection.is_valid() {
                    return Err(Error::invalid("unknown validator id"));
                }
                Ok(self.session.validators(&selection))
            }
            "evaluate" => {
                let timeout = uint(p, "timeout_ms")?;
                // The session accounts for preparation and waiting in this budget,
                // and rejects expired results before publication.
                if !(1..=300_000).contains(&timeout) {
                    return Err(Error::new(
                        "invalid_params",
                        "timeout_ms must be between 1 and 300000",
                    ));
                }
                let report = self
                    .session
                    .evaluate_with_timeout(std::time::Duration::from_millis(timeout))?;
                Ok(evaluation_report(&self.session, &report))
            }
            _ => unreachable!(),
        }
    }
    pub fn handle(&mut self, line: &[u8]) -> Value {
        let value: Value = match crate::json_input::parse(line) {
            Ok(v) => v,
            Err(e) => return failure(Value::Null, Error::new("invalid_json", e.to_string())),
        };
        let id = value
            .get("id")
            .filter(|id| valid_id(id))
            .cloned()
            .unwrap_or(Value::Null);
        let request: Request = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => return failure(id, Error::new("invalid_request", e.to_string())),
        };
        if !valid_id(&request.id) {
            return failure(
                Value::Null,
                Error::new(
                    "invalid_request",
                    "id must be null, u64, or a string of at most 256 bytes",
                ),
            );
        }
        if request.protocol != PROTOCOL {
            return failure(
                id,
                Error::new("unsupported_protocol", "expected ketchup.headless.v1"),
            );
        }
        match self.dispatch(&request.method, request.params) {
            Ok(result) => json!({"protocol":PROTOCOL,"id":id,"result":result}),
            Err(error) => failure(id, error),
        }
    }
}
fn valid_id(id: &Value) -> bool {
    id.is_null() || id.as_u64().is_some() || id.as_str().is_some_and(|s| s.len() <= 256)
}
fn failure(id: Value, error: Error) -> Value {
    let error = model_tools::bounded_error(error);
    let mut detail = json!({"code":error.code,"message":error.message});
    if let Some(details) = error.details {
        detail["details"] = details;
    }
    json!({"protocol":PROTOCOL,"id":id,"error":detail})
}
fn uint(p: &Map<String, Value>, key: &str) -> Result<u64> {
    p.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::invalid(format!("{key} must be u64")))
}
fn string<'a>(p: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::invalid(format!("{key} must be nonempty string")))
}
fn boolean(p: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    p.get(key).map_or(Ok(default), |v| {
        v.as_bool()
            .ok_or_else(|| Error::invalid(format!("{key} must be boolean")))
    })
}
fn ids(p: &Map<String, Value>, key: &str, empty: bool) -> Result<Vec<u64>> {
    let values = match p.get(key) {
        None if empty => return Ok(Vec::new()),
        Some(Value::Array(v)) => v,
        _ => return Err(Error::invalid(format!("{key} must be an array"))),
    };
    if values.len() > 100 || (!empty && values.is_empty()) {
        return Err(Error::invalid("ID count out of bounds"));
    }
    let ids = values
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|id| *id != 0)
                .ok_or_else(|| Error::invalid("IDs must be positive u64"))
        })
        .collect::<Result<Vec<_>>>()?;
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(Error::invalid("duplicate IDs"));
    }
    Ok(ids)
}
fn cam_review_request(p: &Map<String, Value>) -> Result<CamReviewRequest> {
    let operations: Vec<CamOperationInput> = serde_json::from_value(
        p.get("operations")
            .cloned()
            .ok_or_else(|| Error::invalid("missing operations"))?,
    )
    .map_err(|error| Error::invalid(error.to_string()))?;
    let fixtures: Vec<CamFixtureInput> = serde_json::from_value(
        p.get("fixtures")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| Error::invalid(error.to_string()))?;
    let dialect = match string(p, "dialect")? {
        "iso_metric_gcode" => CamPostprocessorDialect::IsoMetricGCode,
        "controller_neutral_json" => CamPostprocessorDialect::ControllerNeutralJson,
        _ => return Err(Error::invalid("unsupported CAM postprocessor dialect")),
    };
    Ok(CamReviewRequest {
        plan_id: CamPlanId(uint(p, "plan_id")?),
        operations: operations.into_iter().map(Into::into).collect(),
        fixtures: fixtures
            .into_iter()
            .map(|fixture| CamFixture {
                id: fixture.id,
                minimum_mm: fixture.minimum_mm,
                maximum_mm: fixture.maximum_mm,
            })
            .collect(),
        dialect,
    })
}

fn fea_review_request(p: &Map<String, Value>) -> Result<(FeaStudyRequest, bool)> {
    let mut payload = Map::new();
    for field in [
        "definition_id",
        "feature_id",
        "occurrence_id",
        "case_id",
        "material",
        "constrained_face_ordinals",
        "face_tractions",
        "mesh_levels",
        "solve_settings",
        "confirmed",
    ] {
        payload.insert(
            field.to_owned(),
            p.get(field)
                .cloned()
                .ok_or_else(|| Error::invalid(format!("missing {field}")))?,
        );
    }
    let input: FeaReviewInput = serde_json::from_value(Value::Object(payload))
        .map_err(|error| Error::invalid(error.to_string()))?;
    if input.definition_id == 0
        || input.feature_id == 0
        || input.occurrence_id == 0
        || input.material.id == 0
        || input.case_id.trim().is_empty()
        || input.case_id.len() > 128
    {
        return Err(Error::invalid(
            "FEA target, material and case IDs must be bounded and nonzero",
        ));
    }
    Ok((
        FeaStudyRequest {
            definition_id: DefinitionId(input.definition_id),
            feature_id: FeatureId(input.feature_id),
            setup: ExactFeaSetup {
                case_id: input.case_id,
                instance_path: InstancePath::root(OccurrenceId(input.occurrence_id)),
                material: FeaMaterial {
                    id: input.material.id,
                    youngs_modulus_mpa: input.material.youngs_modulus_mpa,
                    poisson_ratio: input.material.poisson_ratio,
                    yield_strength_mpa: input.material.yield_strength_mpa,
                },
                constrained_face_ordinals: input.constrained_face_ordinals,
                face_tractions: input
                    .face_tractions
                    .into_iter()
                    .map(|traction| ExactFeaFaceTraction {
                        face_ordinal: traction.face_ordinal,
                        traction_local_n_per_mm2: traction.traction_local_n_per_mm2,
                    })
                    .collect(),
            },
            mesh_levels: input.mesh_levels,
            solve_settings: FeaSolveSettings {
                maximum_nodes: input.solve_settings.maximum_nodes,
                relative_pivot_tolerance: input.solve_settings.relative_pivot_tolerance,
                maximum_small_deformation_ratio: input
                    .solve_settings
                    .maximum_small_deformation_ratio,
                convergence_relative_tolerance: input.solve_settings.convergence_relative_tolerance,
            },
        },
        input.confirmed,
    ))
}

fn fea_review_value(review: &FeaReviewSummary) -> Value {
    json!({
        "review_digest":review.review_digest,
        "source":{"document_id":review.document_id,"revision":review.revision,"canonical_digest":review.canonical_digest},
        "graph_digest":review.graph_digest,
        "case_id":review.case_id,
        "units":review.units,
        "solver_version":review.solver_version,
        "levels":review.levels.iter().map(|level| json!({
            "node_count":level.node_count,
            "element_count":level.element_count,
            "mesh_fingerprint":level.mesh_fingerprint,
            "relative_volume_error":level.relative_volume_error,
            "minimum_quality":level.minimum_quality,
            "maximum_edge_ratio":level.maximum_edge_ratio,
            "maximum_displacement_mm":level.maximum_displacement_mm,
            "maximum_von_mises_stress_mpa":level.maximum_von_mises_stress_mpa,
            "maximum_free_dof_residual_n":level.maximum_free_dof_residual_n,
            "force_balance_n":level.force_balance_n,
            "within_declared_limits":level.within_declared_limits,
        })).collect::<Vec<_>>(),
        "convergence":{
            "final_displacement_relative_change":review.final_displacement_relative_change,
            "final_energy_relative_change":review.final_energy_relative_change,
            "required_relative_tolerance":review.required_relative_tolerance,
            "converged":review.converged,
        },
        "all_levels_within_declared_limits":review.all_levels_within_declared_limits,
        "excluded_physics":review.excluded_physics,
        "document_mutated":false,
    })
}

fn pdm_create_request(p: &Map<String, Value>) -> Result<PdmCreateReleaseRequest> {
    let dependencies: Vec<PdmDependencyInput> = serde_json::from_value(
        p.get("dependencies")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| Error::invalid(error.to_string()))?;
    let audit: PdmAuditInput = serde_json::from_value(
        p.get("audit")
            .cloned()
            .ok_or_else(|| Error::invalid("missing audit"))?,
    )
    .map_err(|error| Error::invalid(error.to_string()))?;
    let parent_release_id = match p.get("parent_release_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        _ => {
            return Err(Error::invalid(
                "parent_release_id must be null or nonempty string",
            ));
        }
    };
    Ok(PdmCreateReleaseRequest {
        repository: PathBuf::from(string(p, "repository")?),
        parent_release_id,
        dependencies: dependencies
            .into_iter()
            .map(|dependency| {
                ReleaseDependencyInput::new(dependency.logical_path, dependency.source_path)
            })
            .collect(),
        audit: ReleaseAudit::new(audit.actor, audit.created_unix_ms, audit.note),
    })
}

fn pdm_manifest_value(manifest: &ReleaseManifest) -> Value {
    json!({
        "schema":manifest.schema,
        "release_id":manifest.release_id,
        "parent_release_id":manifest.parent_release_id,
        "document":{
            "document_id":manifest.document.document_id,
            "revision":manifest.document.revision,
            "canonical_digest":manifest.document.canonical_digest,
            "units":manifest.document.units,
            "object":{"sha256":manifest.document.object.sha256,"byte_len":manifest.document.object.byte_len},
        },
        "dependencies":manifest.dependencies.iter().map(|dependency| json!({
            "logical_path":dependency.logical_path,
            "object":{"sha256":dependency.object.sha256,"byte_len":dependency.object.byte_len},
        })).collect::<Vec<_>>(),
        "audit":{"actor":manifest.audit.actor,"created_unix_ms":manifest.audit.created_unix_ms,"note":manifest.audit.note},
    })
}

fn pdm_verified_release_value(release: &VerifiedRelease, current: &Snapshot) -> Value {
    json!({
        "verified":true,
        "manifest":pdm_manifest_value(&release.manifest),
        "dependency_objects":release.dependency_objects.iter().map(|(logical_path, object)| json!({
            "logical_path":logical_path,"object":{"sha256":object.sha256,"byte_len":object.byte_len},
        })).collect::<Vec<_>>(),
        "matches_current_document":release.snapshot.document_id() == current.document_id()
            && release.snapshot.revision_id() == current.revision_id()
            && release.snapshot.canonical_digest() == current.canonical_digest(),
        "document_mutated":false,
    })
}

fn pdm_catalog_entry_value(entry: &ReleaseCatalogEntry) -> Value {
    json!({
        "release_id":entry.release_id,
        "parent_release_id":entry.parent_release_id,
        "document_id":entry.document_id,
        "revision":entry.revision,
        "canonical_digest":entry.canonical_digest,
        "dependency_count":entry.dependency_count,
        "audit":{"actor":entry.audit.actor,"created_unix_ms":entry.audit.created_unix_ms,"note":entry.audit.note},
    })
}

fn pdm_comparison_value(comparison: &ReleaseComparison) -> Value {
    let (relationship, common_ancestor) = match &comparison.relationship {
        ReleaseRelationship::Same => ("same", None),
        ReleaseRelationship::LeftAncestor => ("left_ancestor", None),
        ReleaseRelationship::RightAncestor => ("right_ancestor", None),
        ReleaseRelationship::Diverged { common_ancestor } => {
            ("diverged", Some(common_ancestor.as_str()))
        }
        ReleaseRelationship::Unrelated => ("unrelated", None),
    };
    let verdict = match comparison.conflict_verdict {
        ReleaseConflictVerdict::AlreadyCurrent => "already_current",
        ReleaseConflictVerdict::FastForward => "fast_forward",
        ReleaseConflictVerdict::IncomingBehind => "incoming_behind",
        ReleaseConflictVerdict::DivergedConflict => "diverged_conflict",
        ReleaseConflictVerdict::UnrelatedConflict => "unrelated_conflict",
    };
    json!({
        "left_release_id":comparison.left_release_id,
        "right_release_id":comparison.right_release_id,
        "relationship":relationship,
        "common_ancestor":common_ancestor,
        "conflict_verdict":verdict,
        "document_changed":comparison.document_changed,
        "dependency_changes":comparison.dependency_changes.iter().map(|change| json!({
            "logical_path":change.logical_path,
            "kind":match change.kind {
                DependencyChangeKind::Added => "added",
                DependencyChangeKind::Removed => "removed",
                DependencyChangeKind::Modified => "modified",
            },
        })).collect::<Vec<_>>(),
        "document_mutated":false,
    })
}

fn cam_review_value(review: &CamReviewSummary) -> Value {
    json!({
        "review_token":review.token,
        "source":{"document_id":review.document_id.0,"revision":review.revision,"canonical_digest":review.canonical_digest,"mutation_epoch":review.mutation_epoch},
        "plan_digest":review.plan_digest,
        "toolpath_digest":review.toolpath_digest,
        "simulation_fingerprint":review.simulation_fingerprint,
        "content_digest":review.content_digest,
        "dialect":match review.dialect { CamPostprocessorDialect::IsoMetricGCode => "iso_metric_gcode", CamPostprocessorDialect::ControllerNeutralJson => "controller_neutral_json" },
        "units":review.units,
        "work_offset":review.work_offset,
        "tool_number":review.tool_number,
        "spindle_rpm":review.spindle_rpm,
        "cutting_feed_mm_per_min":review.cutting_feed_mm_per_min,
        "plunge_feed_mm_per_min":review.plunge_feed_mm_per_min,
        "safe_retract_z_mm":review.safe_retract_z_mm,
        "motion_count":review.motion_count,
        "removed_stock_mm3":review.removed_stock_mm3,
        "residual_stock_mm3":review.residual_stock_mm3,
        "gouge_mm3":review.gouge_mm3,
    })
}

fn created(before: &Snapshot, after: &Snapshot) -> Value {
    json!({"definition_ids":after.definitions().filter(|d|before.definition(d.id()).is_none()).map(|d|d.id().0).collect::<Vec<_>>(),
        "occurrence_ids":after.occurrences().filter(|o|before.occurrence(o.id()).is_none()).map(|o|o.id().0).collect::<Vec<_>>(),
        "feature_ids":after.features().filter(|f|before.feature(f.id()).is_none()).map(|f|f.id().0).collect::<Vec<_>>()})
}
fn evaluation_report(session: &DocumentSession, report: &EvaluationReport) -> Value {
    let snapshot = session.snapshot();
    let producers = report
        .producers
        .iter()
        .map(|producer| (producer.key.definition_id.0, producer.key.feature_id.0))
        .collect::<BTreeSet<_>>();
    json!({"document_id":report.source.0.0,"revision":report.source.1,"canonical_digest":report.source.2,
        "complete":report.complete,"topology_complete":report.topology_complete,"not_evaluated":report.not_evaluated,
        "producers":report.producers.iter().map(|p|json!({"definition_id":p.key.definition_id.0,"feature_id":p.key.feature_id.0,"render":status(&p.render),"topology":status(&p.topology)})).collect::<Vec<_>>(),
        "geometry":geometry(session.exact_results(),&snapshot,&producers),"topology_geometry":geometry(session.topology_results(),&snapshot,&producers)})
}

fn status(s: &EvidenceStatus) -> Value {
    match s {
        EvidenceStatus::Current => json!({"status":"current"}),
        EvidenceStatus::Evaluated => json!({"status":"evaluated"}),
        EvidenceStatus::Failed { reason } => json!({"status":"failed","reason":reason}),
        EvidenceStatus::NotEvaluated { reason } => {
            json!({"status":"not_evaluated","reason":reason})
        }
    }
}
fn geometry(
    registry: &ExactResultRegistry,
    snapshot: &Snapshot,
    producers: &BTreeSet<(u64, u64)>,
) -> Vec<Value> {
    registry.values().filter(|p| p.is_current(snapshot)
        && producers.contains(&(p.definition_id().0, p.producer_feature_id().0))).map(|p| {
        let key=p.result_key();
        // This volume is explicitly mesh evidence, not a fabricated native BRep volume.
        let vertices=p.vertices();
        let volume:f64=p.triangles().iter().map(|t| {
            let [a,b,c]=t.vertex_indices.map(|i|vertices[i as usize].position_mm);
            (a[0]*(b[1]*c[2]-b[2]*c[1])+a[1]*(b[2]*c[0]-b[0]*c[2])+a[2]*(b[0]*c[1]-b[1]*c[0]))/6.0
        }).sum();
        let kind=match p.as_ref() { ExactBodyPackage::Rectangle(_)=>"rectangle",ExactBodyPackage::Revolve(_)=>"revolve",ExactBodyPackage::Graph(_)=>"graph",ExactBodyPackage::Imported(_)=>"imported" };
        json!({"definition_id":p.definition_id().0,"feature_id":p.producer_feature_id().0,"kind":kind,"bounds_mm":p.bounds_mm(),
            "mesh_signed_volume_mm3":volume,"native_evidence":match p.as_ref() { ExactBodyPackage::Graph(g)=>json!({"volume_mm3":g.volume_mm3,"area_mm2":g.area_mm2,"topology_counts":g.topology_counts}), ExactBodyPackage::Imported(g)=>json!({"volume_mm3":g.volume_mm3,"topology_counts":g.topology_counts}), _=>Value::Null },"vertex_count":vertices.len(),"triangle_count":p.triangles().len(),
            "result_fingerprint":key.result_fingerprint,"canonical_input_digest":key.canonical_input_digest,"exact_input_digest":key.exact_input_digest,"backend":key.backend,"evaluator":key.evaluator,"tolerance":key.tolerance})
    }).collect()
}

// Both input and serialization enforce the wire bound without unbounded read_line.
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() > MAX_LINE_BYTES - 1 {
            return Err(io::Error::other("output limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut server: Server,
) -> io::Result<()> {
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        let mut terminated = false;
        loop {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            let take = buffer
                .iter()
                .position(|b| *b == b'\n')
                .map_or(buffer.len(), |i| i + 1);
            terminated = buffer[take - 1] == b'\n';
            if !oversized && line.len() + take <= MAX_LINE_BYTES {
                line.extend_from_slice(&buffer[..take]);
            } else {
                oversized = true;
                line.clear();
            }
            input.consume(take);
            if terminated {
                break;
            }
        }
        if line.is_empty() && !oversized && !terminated {
            return Ok(());
        }
        let response = if oversized {
            failure(
                Value::Null,
                Error::new("line_too_large", "maximum input line is 4 MiB"),
            )
        } else if !terminated {
            failure(
                Value::Null,
                Error::new("invalid_json", "unterminated JSON line"),
            )
        } else {
            server.handle(&line)
        };
        let mut bytes = Bounded(Vec::new());
        if serde_json::to_writer(&mut bytes, &response).is_err() {
            bytes.0.clear();
            let mut error = Error::new(
                "output_too_large",
                "response exceeds 4 MiB; query state before further mutation",
            );
            error.details = Some(json!({"mutation_outcome":"possibly_applied"}));
            serde_json::to_writer(&mut bytes, &failure(response["id"].clone(), error))?;
        }
        output.write_all(&bytes.0)?;
        output.write_all(b"\n")?;
        output.flush()?;
        if !terminated {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(server: &mut Server, method: &str, mut params: Value) -> Value {
        if params.get("expected_revision").is_some()
            && params.get("expected_mutation_epoch").is_none()
        {
            params["expected_mutation_epoch"] = json!(server.session.mutation_epoch());
        }
        server.handle(
            serde_json::to_string(
                &json!({"protocol":PROTOCOL,"id":7,"method":method,"params":params}),
            )
            .unwrap()
            .as_bytes(),
        )
    }

    #[test]
    fn invalid_pdm_repository_path_has_a_stable_protocol_code() {
        let error = Error::from(PdmWorkflowError::Core(LocalPdmError::InvalidRepositoryPath));
        assert_eq!(error.code, "pdm_repository_invalid");
    }

    #[test]
    fn capabilities_mark_every_guarded_method_as_mutating() {
        let mut server = Server::new(SessionSettings::default());
        let capabilities = request(&mut server, "capabilities", json!({}));
        let methods = capabilities["result"]["methods"].as_array().unwrap();
        for name in [
            "cam_preview",
            "fea_review",
            "pdm_release_open",
            "pdm_catalog",
            "pdm_compare",
            "batch_job_start",
            "verify_job_start",
        ] {
            let advertised = methods
                .iter()
                .find(|method| method["name"] == name)
                .unwrap();
            assert_eq!(advertised["mutates"], true, "{name}");
            assert_eq!(
                request(&mut server, name, json!({}))["error"]["code"],
                "invalid_params",
                "{name} is guarded and must advertise that precondition"
            );
        }
    }

    #[test]
    fn rejects_unknown_version_fields_and_stale_without_mutation() {
        let mut s = Server::new(SessionSettings::default());
        let before = s.state();
        for (method, params) in [
            ("wat", json!({})),
            ("state", json!({"extra":1})),
            (
                "new",
                json!({"expected_revision":999,"expected_digest":"stale","discard_unsaved":true}),
            ),
        ] {
            assert!(request(&mut s, method, params).get("error").is_some());
            assert_eq!(s.state(), before);
        }
        assert_eq!(
            s.handle(br#"{"protocol":"bad","id":"abc","method":"state","params":{}}"#)["error"]["code"],
            "unsupported_protocol"
        );
        assert_eq!(s.handle(br#"{"protocol":"ketchup.headless.v1","id":1,"method":"state","params":{"x":1e999}}"#)["error"]["code"],"invalid_json");
    }
    #[test]
    fn undo_rejects_the_original_revision_digest_guard_without_destroying_redo() {
        let mut server = Server::new(SessionSettings::default());
        let initial = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let program = json!({"operations":[{
            "operation":"create_part",
            "name":"Undo ABA probe",
            "workplane":{"type":"principal","plane":"xy"},
            "entities":[{"type":"circle","id":1,"center_mm":[0,0],"radius_mm":5}],
            "constraints":[],
            "feature":{"type":"extrusion","distance_mm":10},
            "translation_mm":[0,0,0]
        }]});
        let applied = request(
            &mut server,
            "apply",
            json!({
                "expected_revision":initial["revision"],
                "expected_digest":initial["canonical_digest"],
                "expected_mutation_epoch":initial["mutation_epoch"],
                "program":program,
                "selection":[]
            }),
        );
        let applied_state = applied["result"]["state"].clone();
        let undone = request(
            &mut server,
            "undo",
            json!({
                "expected_revision":applied_state["revision"],
                "expected_digest":applied_state["canonical_digest"],
                "expected_mutation_epoch":applied_state["mutation_epoch"]
            }),
        );
        assert_eq!(
            undone["result"]["state"]["canonical_digest"],
            initial["canonical_digest"]
        );
        assert_eq!(undone["result"]["state"]["redo_steps"], 1);

        let replayed = request(
            &mut server,
            "apply",
            json!({
                "expected_revision":initial["revision"],
                "expected_digest":initial["canonical_digest"],
                "expected_mutation_epoch":initial["mutation_epoch"],
                "program":program,
                "selection":[]
            }),
        );
        assert_eq!(replayed["error"]["code"], "stale_state", "{replayed}");
        let after = request(&mut server, "state", json!({}))["result"]["state"].clone();
        assert_eq!(after, undone["result"]["state"]);
        assert_eq!(after["redo_steps"], 1);
    }

    #[test]
    fn verify_jobs_start_publish_status_and_cancel_through_native_task() {
        let mut server = Server::new(SessionSettings::default());
        let state = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let stale = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],"expected_digest":"stale"}),
        );
        assert_eq!(stale["error"]["code"], "stale_state");

        let start = || {
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"]})
        };
        let invalid_timeout = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],"timeout_ms":0}),
        );
        assert_eq!(invalid_timeout["error"]["code"], "invalid_params");

        let completed_before_late_poll = request(
            &mut server,
            "verify_job_start",
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],"timeout_ms":100}),
        );
        let completed_handle = completed_before_late_poll["result"]["job_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let completed = request(
            &mut server,
            "verify_job_status",
            json!({"handle":completed_handle}),
        );
        assert_eq!(completed["result"]["state"], "completed", "{completed}");
        assert!(
            completed["result"]["timing"]["elapsed_ms"]
                .as_u64()
                .unwrap()
                >= 100
        );

        let cancelled = request(&mut server, "verify_job_start", start());
        assert_eq!(cancelled["result"]["timing"]["timeout_ms"], 30_000);
        let cancelled_handle = cancelled["result"]["job_handle"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(cancelled["result"]["state"], "running");
        assert_eq!(cancelled["result"]["progress"]["total_bodies"], 0);
        let cancelled = request(
            &mut server,
            "verify_job_cancel",
            json!({"handle":cancelled_handle}),
        );
        assert_eq!(cancelled["result"]["state"], "cancelled");

        let started = request(&mut server, "verify_job_start", start());
        let handle = started["result"]["job_handle"].as_str().unwrap().to_owned();
        let mut status = Value::Null;
        for _ in 0..100 {
            status = request(&mut server, "verify_job_status", json!({"handle":handle}));
            if status["result"]["state"] == "completed" {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(status["result"]["state"], "completed", "{status}");
        assert_eq!(status["result"]["report"]["complete"], false);
        assert_eq!(
            status["result"]["report"]["not_evaluated"],
            "no exact producers selected"
        );
        let unknown = request(&mut server, "verify_job_status", json!({"handle":"forged"}));
        assert_eq!(unknown["error"]["code"], "verify_job_not_found");
    }

    #[test]
    fn completed_verify_jobs_are_reclaimed_without_status_polling() {
        let mut server = Server::new(SessionSettings::default());
        let state = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let start = || {
            json!({"expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],"timeout_ms":30_000})
        };
        for _ in 0..MAX_VERIFY_JOBS {
            let started = request(&mut server, "verify_job_start", start());
            assert_eq!(started["result"]["state"], "running", "{started}");
        }

        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while server.verify_jobs.iter().any(|job| {
            matches!(&job.state, VerifyJobState::Running(task)
                if !task.finished.load(std::sync::atomic::Ordering::Acquire))
        }) {
            assert!(Instant::now() < deadline, "Verify jobs did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let replacement = request(&mut server, "verify_job_start", start());
        assert_eq!(replacement["result"]["state"], "running", "{replacement}");
        assert_eq!(server.verify_jobs.len(), MAX_VERIFY_JOBS);
    }

    #[test]
    fn schema_covers_current_all_operation_variants() {
        let mut s = Server::new(SessionSettings::default());
        let caps = request(&mut s, "capabilities", json!({}));
        let variants =
            caps["result"]["cad_program_schema"]["$defs"]["AssistantCadEditOperation"]["oneOf"]
                .as_array()
                .unwrap();
        assert_eq!(variants.len(), 30);
        for operation in [
            "append_feature",
            "create_program_sketch",
            "append_program_pocket",
            "create_spatial_path",
            "create_helix_path",
            "create_construction_point",
            "create_construction_axis",
            "create_construction_plane",
            "create_helix",
            "create_thread",
            "fillet_edges",
            "chamfer_edges",
            "upsert_cam_plan",
        ] {
            assert!(
                variants
                    .iter()
                    .any(|v| v["properties"]["operation"]["const"] == operation)
            );
        }
    }
    #[test]
    fn schema_covers_program_body_feature_references() {
        let mut s = Server::new(SessionSettings::default());
        let caps = request(&mut s, "capabilities", json!({}));
        let references =
            caps["result"]["cad_program_schema"]["$defs"]["AssistantCadFeatureReference"]["oneOf"]
                .as_array()
                .unwrap();
        assert!(references.iter().any(|value| value["type"] == "integer"));
        assert!(
            references
                .iter()
                .any(|value| { value["$ref"] == "#/$defs/AssistantCadProgramFeatureReference" })
        );
        let local_reference =
            &caps["result"]["cad_program_schema"]["$defs"]["AssistantCadProgramFeatureReference"];
        assert_eq!(local_reference["additionalProperties"], false);
        assert_eq!(
            local_reference["required"],
            json!(["operation_index", "output"])
        );
        let outputs = caps["result"]["cad_program_schema"]["$defs"]
            ["AssistantCadProgramFeatureOutput"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(
            outputs
                .iter()
                .map(|value| value["const"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "definition",
                "sketch_feature",
                "construction_feature",
                "body_feature"
            ]
        );
    }
    #[test]
    fn typed_program_outputs_apply_one_atomic_public_pocket_edit() {
        let mut server = Server::new(SessionSettings::default());
        let before = request(&mut server, "state", json!({}))["result"]["state"].clone();
        let program = json!({"operations":[
            {
                "operation":"create_part",
                "name":"Wall",
                "workplane":{"type":"principal","plane":"xy"},
                "entities":[{"type":"circle","id":1,"center_mm":[0,0],"radius_mm":10}],
                "constraints":[],
                "feature":{"type":"extrusion","distance_mm":20},
                "translation_mm":[0,0,0]
            },
            {
                "operation":"create_program_sketch",
                "definition":{"operation_index":0,"output":"definition"},
                "name":"Opening profile",
                "workplane":{"type":"principal","plane":"xy"},
                "entities":[{"type":"circle","id":1,"center_mm":[0,0],"radius_mm":4}],
                "constraints":[]
            },
            {
                "operation":"append_program_pocket",
                "definition":{"operation_index":0,"output":"definition"},
                "name":"Opening",
                "target_feature":{"operation_index":0,"output":"body_feature"},
                "profile_feature":{"operation_index":1,"output":"sketch_feature"},
                "depth_mm":5
            }
        ]});
        let applied = request(
            &mut server,
            "apply",
            json!({
                "expected_revision": before["revision"],
                "expected_digest": before["canonical_digest"],
                "program": program,
                "selection": []
            }),
        );
        assert!(applied.get("error").is_none(), "{applied}");
        let state = &applied["result"]["state"];
        assert_eq!(state["revision"], before["revision"].as_u64().unwrap() + 1);
        assert_eq!(state["undo_steps"], 1);
        assert_eq!(applied["result"]["created"]["definition_ids"], json!([1]));
        assert_eq!(
            state["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|feature| feature["kind"] == "Pocket")
                .count(),
            1
        );

        let undone = request(
            &mut server,
            "undo",
            json!({
                "expected_revision": state["revision"],
                "expected_digest": state["canonical_digest"]
            }),
        );
        assert_eq!(
            undone["result"]["state"]["canonical_digest"],
            before["canonical_digest"]
        );
    }

    #[test]
    fn persistent_history_drives_compact_redo_modified_and_discard_guard() {
        let mut author = Server::new(SessionSettings::default());
        let initial = request(&mut author, "state", json!({}))["result"]["state"].clone();
        let created = request(
            &mut author,
            "apply",
            json!({
                "expected_revision":initial["revision"],
                "expected_digest":initial["canonical_digest"],
                "selection":[],
                "program":{"operations":[{
                    "operation":"create_part",
                    "name":"History authority",
                    "workplane":{"type":"principal","plane":"xy"},
                    "entities":[
                        {"type":"line","id":1,"start_mm":[0,0],"end_mm":[10,0]},
                        {"type":"line","id":2,"start_mm":[10,0],"end_mm":[10,10]},
                        {"type":"line","id":3,"start_mm":[10,10],"end_mm":[0,10]},
                        {"type":"line","id":4,"start_mm":[0,10],"end_mm":[0,0]}
                    ],
                    "constraints":[],
                    "feature":{"type":"extrusion","distance_mm":5},
                    "translation_mm":[0,0,0]
                }]}
            }),
        );
        assert!(created.get("error").is_none(), "{created}");
        let base = created["result"]["state"].clone();
        let grounded = request(
            &mut author,
            "set_grounded",
            json!({
                "expected_revision":base["revision"],
                "expected_digest":base["canonical_digest"],
                "occurrence_ids":[1],
                "grounded":true
            }),
        );
        let grounded_state = grounded["result"]["state"].clone();
        let undone = request(
            &mut author,
            "undo",
            json!({
                "expected_revision":grounded_state["revision"],
                "expected_digest":grounded_state["canonical_digest"]
            }),
        );
        assert_eq!(
            undone["result"]["state"]["canonical_digest"],
            base["canonical_digest"]
        );
        assert_eq!(undone["result"]["state"]["redo_steps"], 1);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.ketchup");
        let undone_state = undone["result"]["state"].clone();
        let saved = request(
            &mut author,
            "save",
            json!({
                "expected_revision":undone_state["revision"],
                "expected_digest":undone_state["canonical_digest"],
                "path":path,
                "overwrite":false
            }),
        );
        assert_eq!(saved["result"]["modified"], false);
        assert_eq!(saved["result"]["state"]["redo_steps"], 1);

        let mut reopened = Server::new(SessionSettings::default());
        let placeholder = request(&mut reopened, "state", json!({}))["result"]["state"].clone();
        let opened = request(
            &mut reopened,
            "open",
            json!({
                "expected_revision":placeholder["revision"],
                "expected_digest":placeholder["canonical_digest"],
                "path":path,
                "discard_unsaved":false,
                "response":"compact"
            }),
        );
        assert_eq!(opened["result"]["response"], "compact");
        assert_eq!(opened["result"]["state"]["redo_steps"], 1);
        assert_eq!(opened["result"]["modified"], false);
        let opened_state = opened["result"]["state"].clone();

        let redone = request(
            &mut reopened,
            "redo",
            json!({
                "expected_revision":opened_state["revision"],
                "expected_digest":opened_state["canonical_digest"],
                "response":"compact"
            }),
        );
        let redone_state = redone["result"]["state"].clone();
        assert_eq!(redone["result"]["modified"], true);
        assert_eq!(redone_state["redo_steps"], 0);
        let reverted = request(
            &mut reopened,
            "set_grounded",
            json!({
                "expected_revision":redone_state["revision"],
                "expected_digest":redone_state["canonical_digest"],
                "occurrence_ids":[1],
                "grounded":false,
                "response":"compact"
            }),
        );
        let reverted_state = reverted["result"]["state"].clone();
        assert_eq!(
            reverted_state["canonical_digest"],
            opened_state["canonical_digest"]
        );
        assert_eq!(reverted["result"]["modified"], true);

        let refused = request(
            &mut reopened,
            "new",
            json!({
                "expected_revision":reverted_state["revision"],
                "expected_digest":reverted_state["canonical_digest"],
                "discard_unsaved":false,
                "response":"compact"
            }),
        );
        assert_eq!(refused["error"]["code"], "unsaved_changes");
        let after_refusal = request(&mut reopened, "state", json!({}));
        assert_eq!(
            after_refusal["result"]["state"]["revision"],
            reverted_state["revision"]
        );
        assert_eq!(
            after_refusal["result"]["state"]["canonical_digest"],
            reverted_state["canonical_digest"]
        );
    }

    #[test]
    fn oversized_open_response_reports_possible_replacement_and_recovers_compactly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large-state.ketchup");
        let mut document = ketchup_core::document::DocumentStore::new();
        let mut commands = vec![CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Part".to_owned(),
        }];
        commands.extend((1..=35_000).map(|id| {
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1),
                name: "Instance".to_owned(),
                transform: ketchup_core::document::Transform::from_translation(
                    id as f64 + 0.123456789,
                    0.123456789,
                    0.123456789,
                )
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            }
        }));
        document.apply_batch(&CommandBatch::new(commands)).unwrap();
        std::fs::write(&path, ketchup_core::persistence::save(&document.current())).unwrap();
        let author = DocumentSession::open(&path, SessionSettings::default()).unwrap();
        let mut expected = Server::new(SessionSettings::default());
        expected.session = author;
        let loaded = request(&mut expected, "summary", json!({}))["result"]["state"].clone();
        let mut server = Server::new(SessionSettings::default());
        let before = request(&mut server, "summary", json!({}))["result"]["state"].clone();
        assert_ne!(before["canonical_digest"], loaded["canonical_digest"]);
        let mut input = Vec::new();
        for (id, method, params) in [
            (
                1,
                "open",
                json!({"path":path,"discard_unsaved":false,
                "expected_revision":before["revision"],"expected_digest":before["canonical_digest"],
                "expected_mutation_epoch":before["mutation_epoch"]}),
            ),
            (2, "summary", json!({})),
            (
                3,
                "new",
                json!({"discard_unsaved":false,"response":"compact",
                "expected_revision":before["revision"],"expected_digest":before["canonical_digest"],
                "expected_mutation_epoch":before["mutation_epoch"]}),
            ),
        ] {
            serde_json::to_writer(
                &mut input,
                &json!({"protocol":PROTOCOL,"id":id,"method":method,"params":params}),
            )
            .unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(io::Cursor::new(input), &mut output, server).unwrap();
        let output = String::from_utf8(output).unwrap();
        let responses: Vec<Value> = output
            .lines()
            .map(|line| {
                assert!(line.len() < MAX_LINE_BYTES);
                serde_json::from_str(line).unwrap()
            })
            .collect();
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["error"]["code"], "output_too_large");
        assert_eq!(
            responses[0]["error"]["details"]["mutation_outcome"],
            "possibly_applied"
        );
        let mut observed = responses[1]["result"]["state"].clone();
        observed["mutation_epoch"] = loaded["mutation_epoch"].clone();
        assert_eq!(observed, loaded);
        assert_eq!(responses[2]["error"]["code"], "stale_state");
        assert_eq!(
            responses[2]["error"]["details"]["mutation_epoch"],
            responses[1]["result"]["state"]["mutation_epoch"]
        );
    }

    #[test]
    fn reviewed_cam_export_is_exact_guarded_confirmed_and_no_clobber() {
        use ketchup_core::cam::{
            CamCutParameters, CamPlan, CamSetup, CamStock, CamTool, CamToolKind, CamWorkOffset,
        };
        use ketchup_core::document::{Dimension, FeatureKind};

        let worker_path = exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before the headless CAM integration test");
        let mut server = Server::new(SessionSettings {
            exact_worker_path: Some(worker_path),
            ..SessionSettings::default()
        });
        let definition = DefinitionId(95);
        let profile = FeatureId(950);
        let solid = FeatureId(951);
        let seed = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "CAM target".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "20x10 profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: solid,
                    definition_id: definition,
                    name: "20x10x5 target".into(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::new("5", 5.0).unwrap(),
                    },
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&seed).unwrap();
        let plan = CamPlan::new(
            &server.session.snapshot(),
            CamPlanId(95),
            "Reviewed facing",
            CamStock {
                minimum_mm: [0.0, 0.0, 0.0],
                maximum_mm: [20.0, 10.0, 7.0],
            },
            CamTool {
                number: 1,
                kind: CamToolKind::FlatEndMill,
                diameter_mm: 2.0,
                flute_length_mm: 5.0,
                overall_length_mm: 10.0,
                holder_diameter_mm: 6.0,
                holder_length_mm: 10.0,
                spindle_rpm: 10_000,
                feed_mm_per_min: 600.0,
                plunge_mm_per_min: 200.0,
            },
            CamSetup {
                work_offset: CamWorkOffset::G54,
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
                safe_height_mm: 10.0,
            },
            CamCutParameters {
                maximum_stepdown_mm: 2.0,
                stepover_ratio: 0.5,
                radial_allowance_mm: 0.0,
                axial_allowance_mm: 0.0,
            },
            definition,
            solid,
        )
        .unwrap();
        let proposal = server
            .session
            .plan_commands(CommandBatch::new(vec![CanonicalCommand::UpsertCamPlan(
                plan,
            )]))
            .unwrap();
        server.session.apply_proposal(&proposal).unwrap();
        let state = server.state();
        let preview_params = || {
            json!({
                "expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],
                "plan_id":95,
                "operations":[{"type":"face","id":1,"minimum_mm":[1.0,1.0],"maximum_mm":[19.0,9.0],"target_z_mm":5.0}],
                "fixtures":[],
                "dialect":"iso_metric_gcode"
            })
        };
        let preview = request(&mut server, "cam_preview", preview_params());
        assert!(preview.get("error").is_none(), "{preview}");
        assert_eq!(preview["result"]["units"], "mm");
        assert_eq!(preview["result"]["work_offset"], "G54");
        assert_eq!(preview["result"]["tool_number"], 1);
        assert_eq!(
            preview["result"]["source"]["mutation_epoch"],
            state["mutation_epoch"]
        );
        assert!((preview["result"]["removed_stock_mm3"].as_f64().unwrap() - 400.0).abs() < 1.0e-5);
        assert!(preview["result"]["residual_stock_mm3"].as_f64().unwrap() < 1.0e-5);
        assert!(preview["result"]["gouge_mm3"].as_f64().unwrap() < 1.0e-5);
        let token = preview["result"]["review_token"]
            .as_str()
            .unwrap()
            .to_owned();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reviewed.nc");

        let cancelled = request(
            &mut server,
            "cam_export",
            json!({"expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "review_token":token,"path":path,"confirmed":false}),
        );
        assert_eq!(cancelled["error"]["code"], "confirmation_required");
        assert!(!path.exists());
        assert_eq!(server.state(), state);

        let exported = request(
            &mut server,
            "cam_export",
            json!({"expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "review_token":token,"path":path,"confirmed":true}),
        );
        assert_eq!(exported["result"]["exported"], true, "{exported}");
        let artifact = std::fs::read_to_string(&path).unwrap();
        assert!(artifact.starts_with("%\n;KETCHUP_CAM_POSTPROCESSOR_V1\n"));
        assert!(artifact.contains("\nG21\nG90\nG17\nG54\nT1 M6\nS10000 M3\n"));
        assert_eq!(server.state(), state);

        let second = request(&mut server, "cam_preview", preview_params());
        let second_token = second["result"]["review_token"].as_str().unwrap();
        let refused = request(
            &mut server,
            "cam_export",
            json!({"expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "review_token":second_token,"path":path,"confirmed":true}),
        );
        assert_eq!(refused["error"]["code"], "file_exists");

        let collision = request(
            &mut server,
            "cam_preview",
            json!({
                "expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "plan_id":95,
                "operations":[{"type":"face","id":1,"minimum_mm":[1.0,1.0],"maximum_mm":[19.0,9.0],"target_z_mm":5.0}],
                "fixtures":[{"id":1,"minimum_mm":[-0.5,-0.5,9.0],"maximum_mm":[0.5,0.5,12.0]}],
                "dialect":"iso_metric_gcode"
            }),
        );
        assert_eq!(collision["error"]["code"], "cam_postprocessing_rejected");
        let gouge = request(
            &mut server,
            "cam_preview",
            json!({
                "expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "plan_id":95,
                "operations":[{"type":"pocket","id":2,"minimum_mm":[2.0,2.0],"maximum_mm":[18.0,8.0],"top_z_mm":5.0,"bottom_z_mm":1.0}],
                "fixtures":[],"dialect":"iso_metric_gcode"
            }),
        );
        assert_eq!(gouge["error"]["code"], "cam_postprocessing_rejected");
        let malformed = request(
            &mut server,
            "cam_preview",
            json!({
                "expected_revision":state["revision"],"expected_digest":state["canonical_digest"],
                "plan_id":95,
                "operations":[{"type":"face","id":1,"minimum_mm":[1.0,1.0],"maximum_mm":[19.0,9.0],"target_z_mm":5.0,"unknown":true}],
                "fixtures":[],"dialect":"iso_metric_gcode"
            }),
        );
        assert_eq!(malformed["error"]["code"], "invalid_params");
        assert_eq!(server.state(), state);

        let third = request(&mut server, "cam_preview", preview_params());
        let third_token = third["result"]["review_token"].as_str().unwrap().to_owned();
        let mutation = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(96),
                    name: "stale review mutation".into(),
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&mutation).unwrap();
        let current = server.state();
        let stale = request(
            &mut server,
            "cam_export",
            json!({"expected_revision":current["revision"],"expected_digest":current["canonical_digest"],
                "review_token":third_token,"path":directory.path().join("stale.nc"),"confirmed":true}),
        );
        assert_eq!(stale["error"]["code"], "stale_state");
        assert!(!directory.path().join("stale.nc").exists());
        for _ in 0..14 {
            let preview = request(
                &mut server,
                "cam_preview",
                json!({
                    "expected_revision":current["revision"],
                    "expected_digest":current["canonical_digest"],
                    "expected_mutation_epoch":current["mutation_epoch"],
                    "plan_id":95,
                    "operations":[{"type":"face","id":1,"minimum_mm":[1.0,1.0],"maximum_mm":[19.0,9.0],"target_z_mm":5.0}],
                    "fixtures":[],
                    "dialect":"iso_metric_gcode"
                }),
            );
            assert!(preview.get("error").is_none(), "{preview}");
        }

        server.session.undo().unwrap();
        let restored = server.state();
        assert_eq!(restored["revision"], state["revision"]);
        assert_eq!(restored["canonical_digest"], state["canonical_digest"]);
        assert_ne!(restored["mutation_epoch"], state["mutation_epoch"]);
        let undo_stale_path = directory.path().join("undo-stale.nc");
        let undo_stale = request(
            &mut server,
            "cam_export",
            json!({"expected_revision":restored["revision"],"expected_digest":restored["canonical_digest"],
                "expected_mutation_epoch":restored["mutation_epoch"],"review_token":third_token,
                "path":undo_stale_path,"confirmed":true}),
        );
        assert_eq!(undo_stale["error"]["code"], "cam_review_token_invalid");
        assert!(!undo_stale_path.exists());
        let fresh_after_stale_reviews = request(
            &mut server,
            "cam_preview",
            json!({
                "expected_revision":restored["revision"],
                "expected_digest":restored["canonical_digest"],
                "expected_mutation_epoch":restored["mutation_epoch"],
                "plan_id":95,
                "operations":[{"type":"face","id":1,"minimum_mm":[1.0,1.0],"maximum_mm":[19.0,9.0],"target_z_mm":5.0}],
                "fixtures":[],
                "dialect":"iso_metric_gcode"
            }),
        );
        assert!(
            fresh_after_stale_reviews.get("error").is_none(),
            "{fresh_after_stale_reviews}"
        );
    }

    #[test]
    fn exact_fea_review_refines_solves_and_preserves_document_state() {
        use ketchup_core::document::{Dimension, FeatureKind, ProfileSegment, Transform};

        let worker_path = exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before the headless FEA integration test");
        let mut server = Server::new(SessionSettings {
            exact_worker_path: Some(worker_path),
            ..SessionSettings::default()
        });
        let definition = DefinitionId(71);
        let profile = FeatureId(72);
        let solid = FeatureId(73);
        let occurrence = OccurrenceId(74);
        let seed = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "FEA cylinder".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "Circular section".into(),
                    kind: FeatureKind::SegmentProfile {
                        segments: vec![
                            ProfileSegment::CircularArc {
                                start_mm: [10.0, 0.0],
                                end_mm: [-10.0, 0.0],
                                center_mm: [0.0, 0.0],
                                clockwise: false,
                            },
                            ProfileSegment::CircularArc {
                                start_mm: [-10.0, 0.0],
                                end_mm: [10.0, 0.0],
                                center_mm: [0.0, 0.0],
                                clockwise: false,
                            },
                        ],
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: solid,
                    definition_id: definition,
                    name: "Cylinder".into(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::new("20", 20.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: occurrence,
                    definition_id: definition,
                    name: "Cylinder instance".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&seed).unwrap();
        let evaluated = request(&mut server, "evaluate", json!({"timeout_ms":30000}));
        assert_eq!(evaluated["result"]["complete"], true, "{evaluated}");
        let faces = request(
            &mut server,
            "query",
            json!({"kind":"faces","limit":20,"definition_id":definition.0}),
        );
        let items = faces["result"]["items"].as_array().unwrap();
        let bottom = items
            .iter()
            .min_by(|left, right| {
                left["geometry"]["centroid_mm"][2]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&right["geometry"]["centroid_mm"][2].as_f64().unwrap())
            })
            .unwrap()["ordinal"]
            .as_u64()
            .unwrap();
        let top = items
            .iter()
            .max_by(|left, right| {
                left["geometry"]["centroid_mm"][2]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&right["geometry"]["centroid_mm"][2].as_f64().unwrap())
            })
            .unwrap()["ordinal"]
            .as_u64()
            .unwrap();
        assert_ne!(bottom, top);
        let state = server.state();
        let params = |confirmed| {
            json!({
                "expected_revision":state["revision"],
                "expected_digest":state["canonical_digest"],
                "definition_id":definition.0,
                "feature_id":solid.0,
                "occurrence_id":occurrence.0,
                "case_id":"cylinder-pressure",
                "material":{"id":1,"youngs_modulus_mpa":200000.0,"poisson_ratio":0.3,"yield_strength_mpa":250.0},
                "constrained_face_ordinals":[bottom],
                "face_tractions":[{"face_ordinal":top,"traction_local_n_per_mm2":[0.0,0.0,-0.01]}],
                "mesh_levels":[
                    {"surface_deflection_mm":0.3,"angular_deflection_rad":0.3,"max_tetrahedra":512,"max_relative_volume_error":0.05,"min_tetrahedron_quality":0.000001},
                    {"surface_deflection_mm":0.12,"angular_deflection_rad":0.12,"max_tetrahedra":1024,"max_relative_volume_error":0.02,"min_tetrahedron_quality":0.000001}
                ],
                "solve_settings":{"maximum_nodes":256,"relative_pivot_tolerance":1.0e-12,"maximum_small_deformation_ratio":0.05,"convergence_relative_tolerance":0.5},
                "confirmed":confirmed
            })
        };
        let refused = request(&mut server, "fea_review", params(false));
        assert_eq!(refused["error"]["code"], "confirmation_required");
        assert_eq!(server.state(), state);

        let reviewed = request(&mut server, "fea_review", params(true));
        assert!(reviewed.get("error").is_none(), "{reviewed}");
        assert_eq!(reviewed["result"]["units"], "mm,N,MPa");
        assert_eq!(reviewed["result"]["levels"].as_array().unwrap().len(), 2);
        assert_eq!(reviewed["result"]["convergence"]["converged"], true);
        assert_eq!(reviewed["result"]["document_mutated"], false);
        assert_eq!(server.state(), state);
        assert!(reviewed["result"]["review_digest"].as_str().unwrap().len() == 64);

        let stale = request(
            &mut server,
            "fea_review",
            json!({"expected_revision":999,"expected_digest":"stale"}),
        );
        assert_eq!(stale["error"]["code"], "stale_state");
        assert_eq!(server.state(), state);
    }

    #[test]
    fn local_pdm_release_lineage_is_guarded_verified_and_non_mutating() {
        use ketchup_core::document::{Dimension, FeatureKind};

        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repository");
        let dependency = directory.path().join("bearing.step");
        std::fs::write(&dependency, b"ISO-10303-21; bearing v1").unwrap();
        let mut server = Server::new(SessionSettings::default());
        let definition = DefinitionId(301);
        let profile = FeatureId(302);
        let solid = FeatureId(303);
        let seed = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: definition,
                    name: "Released bracket".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: profile,
                    definition_id: definition,
                    name: "80x40 profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: solid,
                    definition_id: definition,
                    name: "80x40x12 bracket".into(),
                    kind: FeatureKind::Extrusion {
                        profile,
                        height: Dimension::new("12", 12.0).unwrap(),
                    },
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&seed).unwrap();
        let root_state = server.state();
        let root_params = |confirmed| {
            json!({
                "expected_revision":root_state["revision"],
                "expected_digest":root_state["canonical_digest"],
                "repository":repository,
                "parent_release_id":null,
                "dependencies":[{"logical_path":"supplier/bearing.step","source_path":dependency}],
                "audit":{"actor":"designer","created_unix_ms":1700000000000_u64,"note":"reviewed root"},
                "confirmed":confirmed,
            })
        };
        let cancelled = request(&mut server, "pdm_release_create", root_params(false));
        assert_eq!(cancelled["error"]["code"], "confirmation_required");
        assert!(!repository.exists());
        assert_eq!(server.state(), root_state);

        let root = request(&mut server, "pdm_release_create", root_params(true));
        assert!(root.get("error").is_none(), "{root}");
        let root_id = root["result"]["manifest"]["release_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(root_id.len(), 64);
        assert_eq!(root["result"]["document_mutated"], false);
        assert_eq!(server.state(), root_state);

        let mutation = server
            .session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(304),
                    name: "Manufacturing notes".into(),
                },
            ]))
            .unwrap();
        server.session.apply_proposal(&mutation).unwrap();
        std::fs::write(&dependency, b"ISO-10303-21; bearing v2").unwrap();
        let child_state = server.state();
        let child = request(
            &mut server,
            "pdm_release_create",
            json!({
                "expected_revision":child_state["revision"],
                "expected_digest":child_state["canonical_digest"],
                "repository":repository,
                "parent_release_id":root_id,
                "dependencies":[{"logical_path":"supplier/bearing.step","source_path":dependency}],
                "audit":{"actor":"reviewer","created_unix_ms":1700000001000_u64,"note":"machining release"},
                "confirmed":true,
            }),
        );
        assert!(child.get("error").is_none(), "{child}");
        let child_id = child["result"]["manifest"]["release_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(server.state(), child_state);

        let catalog = request(
            &mut server,
            "pdm_catalog",
            json!({"expected_revision":child_state["revision"],"expected_digest":child_state["canonical_digest"],"repository":repository}),
        );
        assert_eq!(catalog["result"]["releases"].as_array().unwrap().len(), 2);
        assert_eq!(server.state(), child_state);
        let comparison = request(
            &mut server,
            "pdm_compare",
            json!({"expected_revision":child_state["revision"],"expected_digest":child_state["canonical_digest"],
                "repository":repository,"left_release_id":root_id,"right_release_id":child_id}),
        );
        assert_eq!(comparison["result"]["conflict_verdict"], "fast_forward");
        assert_eq!(
            comparison["result"]["dependency_changes"][0]["kind"],
            "modified"
        );
        assert_eq!(comparison["result"]["document_changed"], true);
        assert_eq!(server.state(), child_state);

        let opened = request(
            &mut server,
            "pdm_release_open",
            json!({"expected_revision":child_state["revision"],"expected_digest":child_state["canonical_digest"],
                "repository":repository,"release_id":child_id}),
        );
        assert_eq!(opened["result"]["verified"], true, "{opened}");
        assert_eq!(opened["result"]["matches_current_document"], true);
        let dependency_object = &opened["result"]["dependency_objects"][0];
        assert_eq!(
            dependency_object["object"],
            opened["result"]["manifest"]["dependencies"][0]["object"]
        );
        assert!(dependency_object.get("verified_object_path").is_none());
        assert_eq!(server.state(), child_state);

        let stale = request(
            &mut server,
            "pdm_catalog",
            json!({"expected_revision":0,"expected_digest":"stale","repository":repository}),
        );
        assert_eq!(stale["error"]["code"], "stale_state");
        let malformed = request(
            &mut server,
            "pdm_compare",
            json!({"expected_revision":child_state["revision"],"expected_digest":child_state["canonical_digest"],
                "repository":repository,"left_release_id":root_id,"right_release_id":child_id,"unknown":true}),
        );
        assert_eq!(malformed["error"]["code"], "invalid_params");

        let manifest_path =
            ketchup_core::local_pdm::release_manifest_path(&repository, &child_id).unwrap();
        let tampered = std::fs::read_to_string(&manifest_path)
            .unwrap()
            .replace("reviewer", "intruder");
        std::fs::write(&manifest_path, tampered).unwrap();
        let rejected = request(
            &mut server,
            "pdm_release_open",
            json!({"expected_revision":child_state["revision"],"expected_digest":child_state["canonical_digest"],
                "repository":repository,"release_id":child_id}),
        );
        assert_eq!(rejected["error"]["code"], "pdm_manifest_invalid");
        assert_eq!(server.state(), child_state);
    }

    #[test]
    fn bounded_lines_resynchronize() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 1];
        input.extend_from_slice(b"\n{\"protocol\":\"ketchup.headless.v1\",\"id\":9,\"method\":\"state\",\"params\":{}}\n");
        let mut output = Vec::new();
        serve(
            io::Cursor::new(input),
            &mut output,
            Server::new(SessionSettings::default()),
        )
        .unwrap();
        let lines = String::from_utf8(output).unwrap();
        let lines: Vec<_> = lines.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).unwrap()["error"]["code"],
            "line_too_large"
        );
        assert_eq!(serde_json::from_str::<Value>(lines[1]).unwrap()["id"], 9);
    }
}
