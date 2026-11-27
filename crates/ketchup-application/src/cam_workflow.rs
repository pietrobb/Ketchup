use ketchup_model::cam::{
    CamError, CamFixture, CamOperation, CamPlan, CamPlanId, CamPlannerError,
    CamPostprocessorDialect, CamPostprocessorOutput, CamSimulationEvidence, CamToolpath,
};
use ketchup_model::document::{DocumentId, Snapshot};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
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
    pub mutation_epoch: u64,
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
    Planning(CamPlannerError),
    Simulation(String),
    Postprocessing(String),
    ReviewLimit,
    InvalidToken,
    StaleReview,
    SubstitutedEvidence,
    ConfirmationRequired,
    InvalidPath,
    FileExists,
    ReplacedFileChanged,
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
            Self::ReplacedFileChanged => formatter.write_str(
                "the file the user agreed to replace changed or disappeared before the CAM export",
            ),
            Self::Write(reason) => write!(formatter, "CAM export failed: {reason}"),
        }
    }
}

impl std::error::Error for CamReviewError {}

#[derive(Clone)]
struct PendingCamReview {
    summary: CamReviewSummary,
    computed: CamComputedReview,
}

/// A program planned, simulated exactly and written for its controller, owned so a thread
/// can make it while the window paints; a review once admitted (review 2026-11-27
/// R1127-CAM-2).
#[derive(Clone)]
pub struct CamComputedReview {
    request: CamReviewRequest,
    toolpath: CamToolpath,
    simulation: CamSimulationEvidence,
    output: CamPostprocessorOutput,
}

impl CamComputedReview {
    /// Plans, simulates and writes the program `request` asks for on `snapshot`, with the
    /// exact worker at `worker_path`, stopped when `cancelled` is set.
    pub fn compute(
        worker_path: Option<&Path>,
        snapshot: &Snapshot,
        request: CamReviewRequest,
        cancelled: &AtomicBool,
    ) -> Result<Self, CamReviewError> {
        let plan = snapshot
            .cam_plan(request.plan_id)
            .ok_or(CamReviewError::PlanMissing)?;
        let (mut worker, toolpath) =
            plan_toolpath(worker_path, snapshot, plan, &request.operations, cancelled)?;
        let simulation = worker
            .simulate_cam(snapshot, plan, &toolpath, &request.fixtures, cancelled)
            .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        let output = CamPostprocessorOutput::generate(
            snapshot,
            plan,
            &toolpath,
            &simulation,
            request.dialect.clone(),
        )
        .map_err(|error| CamReviewError::Postprocessing(error.to_string()))?;
        Ok(Self {
            request,
            toolpath,
            simulation,
            output,
        })
    }
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

    /// The exact worker the reviews run on, for a review computed on a thread.
    #[must_use]
    pub fn worker_path(&self) -> Option<&Path> {
        self.worker_path.as_deref()
    }

    pub fn preview(
        &mut self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        request: CamReviewRequest,
        cancelled: &AtomicBool,
    ) -> Result<CamReviewSummary, CamReviewError> {
        self.make_room(snapshot, mutation_epoch)?;
        let computed =
            CamComputedReview::compute(self.worker_path(), snapshot, request, cancelled)?;
        self.admit(snapshot, mutation_epoch, computed)
    }

    /// Forgets the reviews of another document state; refused when every slot is taken.
    fn make_room(
        &mut self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
    ) -> Result<(), CamReviewError> {
        let source_digest = snapshot.canonical_digest();
        self.pending.retain(|review| {
            review.summary.document_id == snapshot.document_id()
                && review.summary.revision == snapshot.revision_id()
                && review.summary.canonical_digest == source_digest
                && review.summary.mutation_epoch == mutation_epoch
        });
        if self.pending.len() == MAX_PENDING_CAM_REVIEWS {
            return Err(CamReviewError::ReviewLimit);
        }
        Ok(())
    }

    /// Makes the program `computed` on `snapshot` a review its token can export.
    pub fn admit(
        &mut self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        computed: CamComputedReview,
    ) -> Result<CamReviewSummary, CamReviewError> {
        self.make_room(snapshot, mutation_epoch)?;
        let source_digest = snapshot.canonical_digest();
        let CamComputedReview {
            request,
            toolpath,
            simulation,
            output,
        } = &computed;
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(CamReviewError::ReviewLimit)?;
        let token = format!(
            "cam-review-{id:016x}-{:016x}",
            self.key.hash_one((
                id,
                snapshot.document_id().0,
                snapshot.revision_id(),
                source_digest.as_str(),
                mutation_epoch,
                output.content_digest.as_str(),
            ))
        );
        let summary = CamReviewSummary {
            token,
            document_id: snapshot.document_id(),
            revision: snapshot.revision_id(),
            canonical_digest: source_digest,
            mutation_epoch,
            plan_digest: toolpath.plan_digest.clone(),
            toolpath_digest: toolpath.toolpath_digest.clone(),
            simulation_fingerprint: simulation.result_fingerprint.clone(),
            content_digest: output.content_digest.clone(),
            dialect: request.dialect.clone(),
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
            computed,
        });
        Ok(summary)
    }

    /// Writes the reviewed program to the new file `path`, or with `replacing` over the file
    /// there the user agreed to replace, while `replacing` still finds it unchanged.
    #[expect(
        clippy::too_many_arguments,
        reason = "the review, its document state, the target and the consent are all distinct inputs"
    )]
    pub fn export(
        &mut self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        token: &str,
        path: &Path,
        confirmed: bool,
        replacing: Option<&dyn Fn(&Path) -> bool>,
        cancelled: &AtomicBool,
    ) -> Result<CamReviewSummary, CamReviewError> {
        let request = self.reviewed_request(snapshot, mutation_epoch, token, confirmed)?;
        let again = CamComputedReview::compute(self.worker_path(), snapshot, request, cancelled)?;
        self.export_computed(snapshot, mutation_epoch, token, path, replacing, &again)
    }

    /// What the review `token` asked for, to plan and simulate again before its export;
    /// refused unless confirmed and of the current document state.
    pub fn reviewed_request(
        &self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        token: &str,
        confirmed: bool,
    ) -> Result<CamReviewRequest, CamReviewError> {
        if !confirmed {
            return Err(CamReviewError::ConfirmationRequired);
        }
        let (_, review) = self.reviewed(snapshot, mutation_epoch, token)?;
        if snapshot.cam_plan(review.computed.request.plan_id).is_none() {
            return Err(CamReviewError::StaleReview);
        }
        Ok(review.computed.request.clone())
    }

    fn reviewed(
        &self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        token: &str,
    ) -> Result<(usize, &PendingCamReview), CamReviewError> {
        let index = self
            .pending
            .iter()
            .position(|review| review.summary.token == token)
            .ok_or(CamReviewError::InvalidToken)?;
        let review = &self.pending[index];
        if snapshot.document_id() != review.summary.document_id
            || snapshot.revision_id() != review.summary.revision
            || snapshot.canonical_digest() != review.summary.canonical_digest
            || mutation_epoch != review.summary.mutation_epoch
        {
            return Err(CamReviewError::StaleReview);
        }
        Ok((index, review))
    }

    /// Writes the reviewed program `token` to `path` as [`Self::export`] does, `again`
    /// being its request planned and simulated again, which must come out the same.
    pub fn export_computed(
        &mut self,
        snapshot: &Snapshot,
        mutation_epoch: u64,
        token: &str,
        path: &Path,
        replacing: Option<&dyn Fn(&Path) -> bool>,
        again: &CamComputedReview,
    ) -> Result<CamReviewSummary, CamReviewError> {
        let (index, review) = self.reviewed(snapshot, mutation_epoch, token)?;
        let reviewed = &review.computed;
        if again.request != reviewed.request
            || again.toolpath != reviewed.toolpath
            || again.simulation != reviewed.simulation
            || again.output != reviewed.output
            || again.output.content_digest != review.summary.content_digest
        {
            return Err(CamReviewError::SubstitutedEvidence);
        }
        let summary = review.summary.clone();
        let output = &again.output;
        let parent = path
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or(CamReviewError::InvalidPath)?;
        let unchanged = |still_same: &dyn Fn(&Path) -> bool| path.is_file() && still_same(path);
        match replacing {
            None if path.exists() => return Err(CamReviewError::FileExists),
            Some(still_same) if !unchanged(still_same) => {
                return Err(CamReviewError::ReplacedFileChanged);
            }
            _ => {}
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| CamReviewError::Write(error.to_string()))?;
        temporary
            .write_all(&output.content)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| CamReviewError::Write(error.to_string()))?;
        match replacing {
            None => temporary.persist_noclobber(path).map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    CamReviewError::FileExists
                } else {
                    CamReviewError::Write(error.error.to_string())
                }
            })?,
            Some(still_same) => {
                // Checked again just before the rename: the program replaces only the
                // file the user agreed to replace.
                if !unchanged(still_same) {
                    return Err(CamReviewError::ReplacedFileChanged);
                }
                temporary
                    .persist(path)
                    .map_err(|error| CamReviewError::Write(error.error.to_string()))?
            }
        };
        self.pending.remove(index);
        Ok(summary)
    }
}

/// Starts the exact worker and plans the toolpath, first reading every
/// face an operation names from the evaluated target.
fn plan_toolpath(
    worker_path: Option<&Path>,
    snapshot: &Snapshot,
    plan: &CamPlan,
    operations: &[CamOperation],
    cancelled: &AtomicBool,
) -> Result<(ExactWorkerSupervisor, CamToolpath), CamReviewError> {
    let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(
        worker_path.ok_or(CamReviewError::WorkerUnavailable)?,
        cancelled,
    )
    .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
    let resolved;
    let operations = if operations.iter().any(CamOperation::names_face) {
        let target = plan.target();
        let graph =
            ExactBRepGraph::from_snapshot(snapshot, target.definition_id, target.feature_id)
                .map_err(|error| CamReviewError::Planning(CamError::TargetGraph(error).into()))?;
        let package = worker
            .evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
                &graph,
                &[],
                cancelled,
            )
            .map_err(|error| CamReviewError::Simulation(error.to_string()))?;
        resolved = operations
            .iter()
            .map(|operation| operation.resolve_face(&package, plan))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CamReviewError::Planning)?;
        resolved.as_slice()
    } else {
        operations
    };
    let toolpath =
        CamToolpath::plan(snapshot, plan, operations).map_err(CamReviewError::Planning)?;
    Ok((worker, toolpath))
}
