use ketchup_core::cam::{
    CamFixture, CamOperation, CamPlanId, CamPostprocessorDialect, CamPostprocessorOutput,
    CamSimulationEvidence, CamToolpath,
};
use ketchup_core::document::{DocumentId, Snapshot};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::{VecDeque, hash_map::RandomState};
use std::hash::BuildHasher;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const MAX_PENDING_CAM_REVIEWS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub struct CamReviewRequest {
    pub plan_id: CamPlanId,
    pub operations: Vec<CamOperation>,
    pub fixtures: Vec<CamFixture>,
    pub dialect: CamPostprocessorDialect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CamReviewSummary {
    pub token: String,
    pub document_id: DocumentId,
    pub revision: u64,
    pub canonical_digest: String,
    pub plan_digest: String,
    pub toolpath_digest: String,
    pub simulation_fingerprint: String,
    pub content_digest: String,
    pub dialect: CamPostprocessorDialect,
    pub units: String,
    pub work_offset: String,
    pub tool_number: u32,
    pub spindle_rpm: u32,
    pub cutting_feed_mm_per_min: f64,
    pub plunge_feed_mm_per_min: f64,
    pub safe_retract_z_mm: f64,
    pub motion_count: usize,
    pub removed_stock_mm3: f64,
    pub residual_stock_mm3: f64,
    pub gouge_mm3: f64,
}

#[derive(Debug)]
pub enum CamReviewError {
    PlanMissing,
    WorkerUnavailable,
    Planning(String),
    Simulation(String),
    Postprocessing(String),
    ReviewLimit,
    InvalidToken,
    StaleReview,
    SubstitutedEvidence,
    ConfirmationRequired,
    InvalidPath,
    FileExists,
    Write(String),
}

impl std::fmt::Display for CamReviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlanMissing => formatter.write_str("CAM plan does not exist"),
            Self::WorkerUnavailable => formatter.write_str("exact worker is unavailable"),
            Self::Planning(reason) => write!(formatter, "CAM toolpath rejected: {reason}"),
            Self::Simulation(reason) => write!(formatter, "CAM simulation rejected: {reason}"),
            Self::Postprocessing(reason) => {
                write!(formatter, "CAM postprocessing rejected: {reason}")
            }
            Self::ReviewLimit => formatter.write_str("all bounded CAM review slots are active"),
            Self::InvalidToken => formatter.write_str("CAM review token is invalid or expired"),
            Self::StaleReview => {
                formatter.write_str("CAM review no longer matches the current document")
            }
            Self::SubstitutedEvidence => {
                formatter.write_str("CAM review evidence changed before export")
            }
            Self::ConfirmationRequired => {
                formatter.write_str("CAM export requires explicit confirmation")
            }
            Self::InvalidPath => {
                formatter.write_str("CAM export path must have an existing parent directory")
            }
            Self::FileExists => {
                formatter.write_str("CAM export refuses to replace an existing file")
            }
            Self::Write(reason) => write!(formatter, "CAM export failed: {reason}"),
        }
    }
}

impl std::error::Error for CamReviewError {}

#[derive(Clone)]
struct PendingCamReview {
    summary: CamReviewSummary,
    request: CamReviewRequest,
    toolpath: CamToolpath,
    simulation: CamSimulationEvidence,
    output: CamPostprocessorOutput,
}

pub struct CamReviewWorkflow {
    worker_path: Option<PathBuf>,
    key: RandomState,
    next_id: u64,
    pending: VecDeque<PendingCamReview>,
}

impl CamReviewWorkflow {
    #[must_use]
    pub fn new(worker_path: Option<PathBuf>) -> Self {
        Self {
            worker_path,
            key: RandomState::new(),
            next_id: 1,
            pending: VecDeque::new(),
        }
    }

    pub fn set_worker_path(&mut self, worker_path: PathBuf) {
        self.worker_path = Some(worker_path);
    }

    pub fn revoke(&mut self) {
        self.key = RandomState::new();
        self.next_id = 1;
        self.pending.clear();
    }

    pub fn preview(
        &mut self,
        snapshot: &Snapshot,
        request: CamReviewRequest,
        cancelled: &AtomicBool,
    ) -> Result<CamReviewSummary, CamReviewError> {
        if self.pending.len() == MAX_PENDING_CAM_REVIEWS {
            return Err(CamReviewError::ReviewLimit);
        }
        let plan = snapshot
            .cam_plan(request.plan_id)
            .ok_or(CamReviewError::PlanMissing)?;
        let toolpath = CamToolpath::plan(snapshot, plan, &request.operations)
            .map_err(|error| CamReviewError::Planning(error.to_string()))?;
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(
            self.worker_path
                .as_deref()
                .ok_or(CamReviewError::WorkerUnavailable)?,
            cancelled,
        )
        .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        let simulation = worker
            .simulate_cam(snapshot, plan, &toolpath, &request.fixtures, cancelled)
            .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        let output = CamPostprocessorOutput::generate(
            snapshot,
            plan,
            &toolpath,
            &simulation,
            request.dialect,
        )
        .map_err(|error| CamReviewError::Postprocessing(error.to_string()))?;
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(CamReviewError::ReviewLimit)?;
        let source_digest = snapshot.canonical_digest();
        let token = format!(
            "cam-review-{id:016x}-{:016x}",
            self.key.hash_one((
                id,
                snapshot.document_id().0,
                snapshot.revision_id(),
                source_digest.as_str(),
                output.content_digest.as_str(),
            ))
        );
        let summary = CamReviewSummary {
            token,
            document_id: snapshot.document_id(),
            revision: snapshot.revision_id(),
            canonical_digest: source_digest,
            plan_digest: toolpath.plan_digest.clone(),
            toolpath_digest: toolpath.toolpath_digest.clone(),
            simulation_fingerprint: simulation.result_fingerprint.clone(),
            content_digest: output.content_digest.clone(),
            dialect: request.dialect,
            units: output.program.units.clone(),
            work_offset: output.program.work_offset.clone(),
            tool_number: output.program.tool_number,
            spindle_rpm: output.program.spindle_rpm,
            cutting_feed_mm_per_min: output.program.cutting_feed_mm_per_min,
            plunge_feed_mm_per_min: output.program.plunge_feed_mm_per_min,
            safe_retract_z_mm: output.program.safe_retract_z_mm,
            motion_count: output.program.motions.len(),
            removed_stock_mm3: simulation.removed_stock_mm3,
            residual_stock_mm3: simulation.residual_stock_mm3,
            gouge_mm3: simulation.gouge_mm3,
        };
        self.pending.push_back(PendingCamReview {
            summary: summary.clone(),
            request,
            toolpath,
            simulation,
            output,
        });
        Ok(summary)
    }

    pub fn export(
        &mut self,
        snapshot: &Snapshot,
        token: &str,
        path: &Path,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<CamReviewSummary, CamReviewError> {
        if !confirmed {
            return Err(CamReviewError::ConfirmationRequired);
        }
        let index = self
            .pending
            .iter()
            .position(|review| review.summary.token == token)
            .ok_or(CamReviewError::InvalidToken)?;
        let review = self.pending[index].clone();
        if snapshot.document_id() != review.summary.document_id
            || snapshot.revision_id() != review.summary.revision
            || snapshot.canonical_digest() != review.summary.canonical_digest
        {
            return Err(CamReviewError::StaleReview);
        }
        let plan = snapshot
            .cam_plan(review.request.plan_id)
            .ok_or(CamReviewError::StaleReview)?;
        let toolpath = CamToolpath::plan(snapshot, plan, &review.request.operations)
            .map_err(|error| CamReviewError::Planning(error.to_string()))?;
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(
            self.worker_path
                .as_deref()
                .ok_or(CamReviewError::WorkerUnavailable)?,
            cancelled,
        )
        .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        let simulation = worker
            .simulate_cam(
                snapshot,
                plan,
                &toolpath,
                &review.request.fixtures,
                cancelled,
            )
            .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        let output = CamPostprocessorOutput::generate(
            snapshot,
            plan,
            &toolpath,
            &simulation,
            review.request.dialect,
        )
        .map_err(|error| CamReviewError::Postprocessing(error.to_string()))?;
        if toolpath != review.toolpath
            || simulation != review.simulation
            || output != review.output
            || output.content_digest != review.summary.content_digest
        {
            return Err(CamReviewError::SubstitutedEvidence);
        }
        let parent = path
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or(CamReviewError::InvalidPath)?;
        if path.exists() {
            return Err(CamReviewError::FileExists);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| CamReviewError::Write(error.to_string()))?;
        temporary
            .write_all(&output.content)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| CamReviewError::Write(error.to_string()))?;
        temporary.persist_noclobber(path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                CamReviewError::FileExists
            } else {
                CamReviewError::Write(error.error.to_string())
            }
        })?;
        self.pending.remove(index);
        Ok(review.summary)
    }
}
