use std::{
    fmt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    time::{Duration, Instant},
};

use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentId, DocumentStore, FeatureId,
    FeatureKind, MeshBodySpec, Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError};
use ketchup_core::exact_product::ExactBRepGraphPackage;
use ketchup_core::mesh_recognition::{
    CylinderRecognition, MeshRecognition, MeshRecognitionCandidate, MeshRecognitionResiduals,
    recognize_mesh_body_cancellable,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, SketchEntity, SketchEntityId, SketchSpec,
    WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};
use ketchup_scheduler::ExactWorkerSupervisor;

const MESH_CONVERSION_PROGRESS_STEPS: u8 = 3;
const MAX_VERIFICATION_WORK_UNITS: usize = 32_000_000;
const MAX_VERIFICATION_VERTICES: usize = 1_000_000;
const TRIANGLE_BVH_LEAF_SIZE: usize = 8;
static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq)]
pub enum MeshConversionError {
    SourceNotFound,
    SourceIsNotSoleMeshBody,
    NoMatch(String),
    Ambiguous(String),
    IdentifierOverflow,
    InvalidCandidate,
    InvalidBatch(String),
    ExactGraph(String),
    ExactVerificationRequired,
    ExactVerificationMismatch,
    VerificationResourceLimit,
    Cancelled,
    TimedOut,
    Stale,
}

impl fmt::Display for MeshConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotFound => formatter.write_str("source mesh feature was not found"),
            Self::SourceIsNotSoleMeshBody => {
                formatter.write_str("source definition is not a sole mesh body")
            }
            Self::NoMatch(reason) => write!(formatter, "mesh was not recognized: {reason}"),
            Self::Ambiguous(reason) => write!(formatter, "mesh recognition is ambiguous: {reason}"),
            Self::IdentifierOverflow => {
                formatter.write_str("feature identifier space is exhausted")
            }
            Self::InvalidCandidate => formatter.write_str("recognized exact candidate is invalid"),
            Self::InvalidBatch(reason) => {
                write!(formatter, "conversion batch is invalid: {reason}")
            }
            Self::ExactGraph(reason) => {
                write!(formatter, "exact conversion graph is invalid: {reason}")
            }
            Self::ExactVerificationRequired => {
                formatter.write_str("exact worker verification is required before conversion")
            }
            Self::ExactVerificationMismatch => {
                formatter.write_str("exact result does not match the recognized source mesh")
            }
            Self::VerificationResourceLimit => {
                formatter.write_str("mesh is too large for bounded exact conversion verification")
            }
            Self::Cancelled => formatter.write_str("mesh conversion was cancelled"),
            Self::TimedOut => formatter.write_str("mesh conversion timed out"),
            Self::Stale => formatter.write_str("mesh conversion plan is stale"),
        }
    }
}

impl std::error::Error for MeshConversionError {}

#[derive(Clone)]
pub struct MeshConversionPlan {
    document_id: DocumentId,
    source_revision: u64,
    target_revision: u64,
    source_digest: String,
    mutation_epoch: u64,
    source_definition_id: DefinitionId,
    source_feature_id: FeatureId,
    source_mesh: MeshBodySpec,
    tolerance_mm: f64,
    candidate: MeshRecognitionCandidate,
    residuals: MeshRecognitionResiduals,
    batch: CommandBatch,
    preview: Snapshot,
    graph: ExactBRepGraph,
}

impl MeshConversionPlan {
    #[must_use]
    pub const fn source_definition_id(&self) -> DefinitionId {
        self.source_definition_id
    }

    #[must_use]
    pub const fn source_feature_id(&self) -> FeatureId {
        self.source_feature_id
    }

    #[must_use]
    pub const fn candidate(&self) -> &MeshRecognitionCandidate {
        &self.candidate
    }

    #[must_use]
    pub const fn residuals(&self) -> MeshRecognitionResiduals {
        self.residuals
    }

    #[must_use]
    pub const fn tolerance_mm(&self) -> f64 {
        self.tolerance_mm
    }

    #[must_use]
    pub const fn graph(&self) -> &ExactBRepGraph {
        &self.graph
    }

    #[must_use]
    pub fn preview(&self) -> &Snapshot {
        &self.preview
    }

    #[must_use]
    pub fn batch_digest(&self) -> String {
        self.batch.digest()
    }

    #[must_use]
    pub fn matches_source(&self, snapshot: &Snapshot, mutation_epoch: u64) -> bool {
        self.document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && self.source_digest == snapshot.canonical_digest()
            && self.mutation_epoch == mutation_epoch
    }
}

#[derive(Clone, Debug)]
pub struct MeshConversionVerification {
    graph_digest: String,
    result_fingerprint: String,
    max_source_to_exact_mm: f64,
    max_exact_to_source_mm: f64,
    package: ExactBRepGraphPackage,
}

impl MeshConversionVerification {
    #[must_use]
    pub const fn max_source_to_exact_mm(&self) -> f64 {
        self.max_source_to_exact_mm
    }

    #[must_use]
    pub const fn max_exact_to_source_mm(&self) -> f64 {
        self.max_exact_to_source_mm
    }

    #[must_use]
    pub const fn package(&self) -> &ExactBRepGraphPackage {
        &self.package
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshConversionSource {
    document_id: DocumentId,
    revision: u64,
    canonical_digest: String,
    mutation_epoch: u64,
}

impl MeshConversionSource {
    fn new(snapshot: &Snapshot, mutation_epoch: u64) -> Self {
        Self {
            document_id: snapshot.document_id(),
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch,
        }
    }

    #[must_use]
    pub fn matches(&self, snapshot: &Snapshot, mutation_epoch: u64) -> bool {
        self.document_id == snapshot.document_id()
            && self.revision == snapshot.revision_id()
            && self.canonical_digest == snapshot.canonical_digest()
            && self.mutation_epoch == mutation_epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshConversionStage {
    Recognizing,
    EvaluatingExact,
    Verifying,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeshConversionProgress {
    pub stage: MeshConversionStage,
    pub completed: u8,
    pub total: u8,
}

pub struct PreparedMeshConversion {
    pub plan: MeshConversionPlan,
    pub verification: MeshConversionVerification,
}

pub enum MeshConversionTaskEvent {
    Progress(MeshConversionProgress),
    Finished(Box<Result<PreparedMeshConversion, MeshConversionError>>),
}

pub struct MeshConversionTask {
    pub source: MeshConversionSource,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<MeshConversionTaskEvent>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for MeshConversionTask {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            } else {
                let _ = std::thread::Builder::new()
                    .name("mesh-conversion-reaper".to_owned())
                    .spawn(move || {
                        let _ = worker.join();
                    });
            }
        }
    }
}

impl MeshConversionTask {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn poll(&self) -> Result<MeshConversionTaskEvent, TryRecvError> {
        self.receiver.try_recv().map(|event| match event {
            MeshConversionTaskEvent::Finished(result) if self.is_cancelled() && result.is_ok() => {
                MeshConversionTaskEvent::Finished(Box::new(Err(MeshConversionError::Cancelled)))
            }
            event => event,
        })
    }

    pub fn wait(self, timeout: Duration) -> Result<PreparedMeshConversion, MeshConversionError> {
        let started_at = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(started_at.elapsed());
            match self.receiver.recv_timeout(remaining) {
                Ok(MeshConversionTaskEvent::Progress(_)) => {}
                Ok(MeshConversionTaskEvent::Finished(result)) => {
                    return if self.is_cancelled() && result.is_ok() {
                        Err(MeshConversionError::Cancelled)
                    } else {
                        *result
                    };
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel();
                    return Err(MeshConversionError::TimedOut);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(MeshConversionError::Cancelled);
                }
            }
        }
    }
}

pub fn start_mesh_conversion(
    document: &DocumentStore,
    source_feature_id: FeatureId,
    tolerance_mm: f64,
    executable: PathBuf,
    timeout: Duration,
    completed: impl FnOnce() + Send + 'static,
) -> Result<MeshConversionTask, String> {
    let snapshot = document.current();
    let mutation_epoch = document.mutation_epoch();
    let next_revision_id = document.next_revision_id();
    let source = MeshConversionSource::new(&snapshot, mutation_epoch);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let (sender, receiver) = mpsc::channel();
    let started_at = Instant::now();
    let worker = std::thread::Builder::new()
        .name("mesh-to-exact-conversion".to_owned())
        .spawn(move || {
            let timed_out = Arc::new(AtomicBool::new(false));
            let timeout_cancelled = Arc::clone(&worker_cancelled);
            let timeout_flag = Arc::clone(&timed_out);
            let (done_sender, done_receiver) = mpsc::channel();
            let watchdog_started_at = started_at;
            let watchdog = std::thread::spawn(move || {
                let remaining = timeout.saturating_sub(watchdog_started_at.elapsed());
                if done_receiver.recv_timeout(remaining).is_err() {
                    timeout_flag.store(true, Ordering::Release);
                    timeout_cancelled.store(true, Ordering::Release);
                }
            });
            let send_progress = |stage, completed| {
                sender.send(MeshConversionTaskEvent::Progress(MeshConversionProgress {
                    stage,
                    completed,
                    total: MESH_CONVERSION_PROGRESS_STEPS,
                }))
            };
            let result = (|| {
                send_progress(MeshConversionStage::Recognizing, 0)
                    .map_err(|_| MeshConversionError::Cancelled)?;
                let plan = prepare_mesh_conversion_from_snapshot(
                    snapshot,
                    mutation_epoch,
                    next_revision_id,
                    source_feature_id,
                    tolerance_mm,
                    &worker_cancelled,
                )?;
                if worker_cancelled.load(Ordering::Acquire) {
                    return Err(MeshConversionError::Cancelled);
                }
                send_progress(MeshConversionStage::EvaluatingExact, 1)
                    .map_err(|_| MeshConversionError::Cancelled)?;
                let mut worker =
                    ExactWorkerSupervisor::spawn_with_cancellation(executable, &worker_cancelled)
                        .map_err(|error| MeshConversionError::ExactGraph(error.to_string()))?;
                let package = worker
                    .evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
                        plan.graph(),
                        &[],
                        &worker_cancelled,
                    )
                    .map_err(|error| MeshConversionError::ExactGraph(error.to_string()))?;
                send_progress(MeshConversionStage::Verifying, 2)
                    .map_err(|_| MeshConversionError::Cancelled)?;
                let verification =
                    verify_mesh_conversion_with_cancellation(&plan, package, &worker_cancelled)?;
                Ok(PreparedMeshConversion { plan, verification })
            })();
            let _ = done_sender.send(());
            let _ = watchdog.join();
            let result = if timed_out.load(Ordering::Acquire) || started_at.elapsed() >= timeout {
                Err(MeshConversionError::TimedOut)
            } else if worker_cancelled.load(Ordering::Acquire) {
                Err(MeshConversionError::Cancelled)
            } else {
                result
            };
            if sender
                .send(MeshConversionTaskEvent::Finished(Box::new(result)))
                .is_ok()
            {
                let _ = std::thread::Builder::new()
                    .name("mesh-conversion-completed".to_owned())
                    .spawn(completed);
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(MeshConversionTask {
        source,
        cancelled,
        receiver,
        worker: Some(worker),
    })
}

pub fn prepare_mesh_conversion(
    document: &DocumentStore,
    source_feature_id: FeatureId,
    tolerance_mm: f64,
) -> Result<MeshConversionPlan, MeshConversionError> {
    prepare_mesh_conversion_from_snapshot(
        document.current(),
        document.mutation_epoch(),
        document.next_revision_id(),
        source_feature_id,
        tolerance_mm,
        &NEVER_CANCELLED,
    )
}

fn prepare_mesh_conversion_from_snapshot(
    snapshot: Snapshot,
    mutation_epoch: u64,
    next_revision_id: u64,
    source_feature_id: FeatureId,
    tolerance_mm: f64,
    cancelled: &AtomicBool,
) -> Result<MeshConversionPlan, MeshConversionError> {
    let source = snapshot
        .feature(source_feature_id)
        .ok_or(MeshConversionError::SourceNotFound)?;
    let definition = snapshot
        .definition(source.definition_id())
        .ok_or(MeshConversionError::SourceNotFound)?;
    let FeatureKind::MeshBody(source_mesh) = source.kind() else {
        return Err(MeshConversionError::SourceIsNotSoleMeshBody);
    };
    if definition.feature_ids() != [source_feature_id] {
        return Err(MeshConversionError::SourceIsNotSoleMeshBody);
    }
    let recognition = recognize_mesh_body_cancellable(source_mesh, tolerance_mm, || {
        cancelled.load(Ordering::Acquire)
    });
    if cancelled.load(Ordering::Acquire) {
        return Err(MeshConversionError::Cancelled);
    }
    let (candidate, residuals) = match recognition {
        MeshRecognition::Candidate {
            candidate,
            residuals,
        } => (candidate, residuals),
        MeshRecognition::NoMatch { reason } => return Err(MeshConversionError::NoMatch(reason)),
        MeshRecognition::Ambiguous { reason, .. } => {
            return Err(MeshConversionError::Ambiguous(reason));
        }
    };
    let next_feature = snapshot
        .features()
        .map(|feature| feature.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(MeshConversionError::IdentifierOverflow)?;
    let extrusion_id = FeatureId(next_feature);
    let transform_id = FeatureId(
        next_feature
            .checked_add(1)
            .ok_or(MeshConversionError::IdentifierOverflow)?,
    );
    let chain = candidate_feature_chain(
        source.definition_id(),
        source_feature_id,
        extrusion_id,
        transform_id,
        &candidate,
    )?;
    let producer_feature_id = chain.producer_feature_id;
    let batch = CommandBatch::new(chain.commands);
    let preview = snapshot
        .preview_batch_at_revision(&batch, next_revision_id)
        .map_err(|error| MeshConversionError::InvalidBatch(error.to_string()))?;
    let graph =
        ExactBRepGraph::from_snapshot(&preview, source.definition_id(), producer_feature_id)
            .map_err(exact_graph_error)?;
    Ok(MeshConversionPlan {
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        target_revision: next_revision_id,
        source_digest: snapshot.canonical_digest(),
        mutation_epoch,
        source_definition_id: source.definition_id(),
        source_feature_id,
        source_mesh: source_mesh.clone(),
        tolerance_mm,
        candidate,
        residuals,
        batch,
        preview,
        graph,
    })
}

pub fn verify_mesh_conversion(
    plan: &MeshConversionPlan,
    package: ExactBRepGraphPackage,
) -> Result<MeshConversionVerification, MeshConversionError> {
    verify_mesh_conversion_with_cancellation(plan, package, &NEVER_CANCELLED)
}

fn verify_mesh_conversion_with_cancellation(
    plan: &MeshConversionPlan,
    package: ExactBRepGraphPackage,
    cancelled: &AtomicBool,
) -> Result<MeshConversionVerification, MeshConversionError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(MeshConversionError::Cancelled);
    }
    if package.graph != plan.graph
        || package.identity.document_id != plan.preview.document_id()
        || package.identity.source_revision != plan.preview.revision_id()
        || package.identity.source_digest != plan.preview.canonical_digest()
        || package.identity.definition_id != plan.source_definition_id
        || package.identity.producer_feature_id != FeatureId(plan.graph.producer_feature_id)
    {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    let source_triangles = plan.source_mesh.triangles.len();
    let exact_triangles = package.triangles.len();
    if source_triangles == 0 || exact_triangles == 0 {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    let total_vertices = plan
        .source_mesh
        .vertices_mm
        .len()
        .checked_add(package.vertices.len())
        .ok_or(MeshConversionError::VerificationResourceLimit)?;
    let sampled_points = source_triangles
        .checked_add(exact_triangles)
        .and_then(|count| count.checked_mul(7))
        .ok_or(MeshConversionError::VerificationResourceLimit)?;
    if total_vertices > MAX_VERIFICATION_VERTICES || sampled_points > MAX_VERIFICATION_WORK_UNITS {
        return Err(MeshConversionError::VerificationResourceLimit);
    }
    if package.vertices.iter().any(|vertex| {
        vertex
            .position_mm
            .into_iter()
            .any(|value| !value.is_finite())
    }) || package.triangles.iter().any(|triangle| {
        triangle
            .vertex_indices
            .into_iter()
            .any(|index| index as usize >= package.vertices.len())
    }) {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    let mut work_budget = VerificationWorkBudget::new(MAX_VERIFICATION_WORK_UNITS);
    work_budget.consume(package.vertices.len())?;
    let source_vertices = plan.source_mesh.vertices_mm.as_slice();
    let source_faces = plan.source_mesh.triangles.as_slice();
    let exact_vertices = package
        .vertices
        .iter()
        .map(|vertex| vertex.position_mm)
        .collect::<Vec<_>>();
    let exact_faces = package
        .triangles
        .iter()
        .map(|triangle| triangle.vertex_indices)
        .collect::<Vec<_>>();
    let source_to_exact = directed_mesh_sample_distance(
        source_vertices,
        source_faces,
        &exact_vertices,
        &exact_faces,
        cancelled,
        &mut work_budget,
    )?;
    let exact_to_source = directed_mesh_sample_distance(
        &exact_vertices,
        &exact_faces,
        source_vertices,
        source_faces,
        cancelled,
        &mut work_budget,
    )?;
    if source_to_exact > plan.tolerance_mm || exact_to_source > plan.tolerance_mm {
        return Err(MeshConversionError::ExactVerificationMismatch);
    }
    Ok(MeshConversionVerification {
        graph_digest: package.graph.graph_digest.clone(),
        result_fingerprint: package.identity.result_fingerprint.clone(),
        max_source_to_exact_mm: source_to_exact,
        max_exact_to_source_mm: exact_to_source,
        package,
    })
}

pub fn commit_mesh_conversion(
    document: &mut DocumentStore,
    plan: &MeshConversionPlan,
    verification: MeshConversionVerification,
) -> Result<ExactBRepGraphPackage, MeshConversionError> {
    let current = document.current();
    if document.mutation_epoch() != plan.mutation_epoch
        || current.document_id() != plan.document_id
        || current.revision_id() != plan.source_revision
        || document.next_revision_id() != plan.target_revision
        || plan.preview.revision_id() != plan.target_revision
        || current.canonical_digest() != plan.source_digest
        || verification.graph_digest != plan.graph.graph_digest
        || verification.package.graph != plan.graph
        || verification.package.identity.document_id != plan.preview.document_id()
        || verification.package.identity.source_revision != plan.preview.revision_id()
        || verification.package.identity.source_digest != plan.preview.canonical_digest()
        || verification.package.identity.definition_id != plan.source_definition_id
        || verification.package.identity.producer_feature_id
            != FeatureId(plan.graph.producer_feature_id)
        || verification.result_fingerprint != verification.package.identity.result_fingerprint
    {
        return Err(MeshConversionError::Stale);
    }
    document
        .apply_batch(&plan.batch)
        .map_err(|error| MeshConversionError::InvalidBatch(error.to_string()))?;
    debug_assert_eq!(
        document.current().canonical_digest(),
        plan.preview.canonical_digest()
    );
    debug_assert_eq!(document.current().revision_id(), plan.target_revision);
    Ok(verification.package)
}

struct CandidateFeatureChain {
    commands: Vec<CanonicalCommand>,
    producer_feature_id: FeatureId,
}

fn candidate_feature_chain(
    definition_id: DefinitionId,
    profile_id: FeatureId,
    extrusion_id: FeatureId,
    transform_id: FeatureId,
    candidate: &MeshRecognitionCandidate,
) -> Result<CandidateFeatureChain, MeshConversionError> {
    if let MeshRecognitionCandidate::Cylinder(cylinder) = candidate {
        return cylinder_feature_chain(
            definition_id,
            profile_id,
            extrusion_id,
            transform_id,
            cylinder,
        );
    }
    let (profile, height_mm, origin, basis_u, mut basis_v, axis) = match candidate {
        MeshRecognitionCandidate::Box(value) => {
            let [width, depth, height] = value.dimensions_mm;
            (
                FeatureKind::Profile {
                    points_mm: vec![
                        [-width * 0.5, -depth * 0.5],
                        [width * 0.5, -depth * 0.5],
                        [width * 0.5, depth * 0.5],
                        [-width * 0.5, depth * 0.5],
                    ],
                },
                height,
                subtract_3d(value.center_mm, scale_3d(value.axes[2], height * 0.5)),
                value.axes[0],
                value.axes[1],
                value.axes[2],
            )
        }
        MeshRecognitionCandidate::LinearExtrusion(value) => (
            FeatureKind::Profile {
                points_mm: value.profile_mm.clone(),
            },
            value.height_mm,
            value.base_origin_mm,
            value.profile_basis[0],
            value.profile_basis[1],
            value.axis,
        ),
        MeshRecognitionCandidate::Cylinder(_) => {
            unreachable!("cylinder candidates return before generic extrusion planning")
        }
    };
    let mut profile = profile;
    if determinant(basis_u, basis_v, axis) < 0.0 {
        basis_v = scale_3d(basis_v, -1.0);
        reflect_profile_y(&mut profile);
    }
    ensure_counter_clockwise_profile(&mut profile);
    if !height_mm.is_finite() || height_mm <= 0.0 {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let transform = Transform::from_matrix([
        basis_u[0], basis_v[0], axis[0], origin[0], basis_u[1], basis_v[1], axis[1], origin[1],
        basis_u[2], basis_v[2], axis[2], origin[2], 0.0, 0.0, 0.0, 1.0,
    ])
    .map_err(|_| MeshConversionError::InvalidCandidate)?;
    if transform.rigid_inverse().is_none() {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let mut commands = vec![
        CanonicalCommand::DeleteFeature { id: profile_id },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: "Recognized profile".to_owned(),
            kind: profile,
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: "Recognized extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: profile_id,
                height: Dimension::new(format!("{height_mm:.17}"), height_mm)
                    .map_err(|_| MeshConversionError::InvalidCandidate)?,
            },
        },
    ];
    let producer_feature_id = if transform == Transform::identity() {
        extrusion_id
    } else {
        commands.push(CanonicalCommand::CreateFeature {
            id: transform_id,
            definition_id,
            name: "Recognized placement".to_owned(),
            kind: FeatureKind::RigidTransform {
                target: extrusion_id,
                transform,
            },
        });
        transform_id
    };
    Ok(CandidateFeatureChain {
        commands,
        producer_feature_id,
    })
}

fn cylinder_feature_chain(
    definition_id: DefinitionId,
    workplane_id: FeatureId,
    sketch_id: FeatureId,
    pad_id: FeatureId,
    cylinder: &CylinderRecognition,
) -> Result<CandidateFeatureChain, MeshConversionError> {
    if !cylinder.radius_mm.is_finite()
        || cylinder.radius_mm <= 0.0
        || !cylinder.height_mm.is_finite()
        || cylinder.height_mm <= 0.0
    {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let [basis_u, basis_v] = perpendicular_basis(cylinder.axis)?;
    let origin = subtract_3d(
        cylinder.center_mm,
        scale_3d(cylinder.axis, cylinder.height_mm * 0.5),
    );
    let frame = WorkplaneFrame::from_axes(origin, basis_u, basis_v)
        .map_err(|_| MeshConversionError::InvalidCandidate)?;
    let sketch = SketchSpec {
        workplane: workplane_id,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: cylinder.radius_mm,
        }],
        constraints: Vec::new(),
    };
    let region = sketch
        .solved_regions()
        .map_err(|_| MeshConversionError::InvalidCandidate)?
        .into_iter()
        .next()
        .ok_or(MeshConversionError::InvalidCandidate)?
        .id;
    Ok(CandidateFeatureChain {
        commands: vec![
            CanonicalCommand::DeleteFeature { id: workplane_id },
            CanonicalCommand::CreateFeature {
                id: workplane_id,
                definition_id,
                name: "Recognized cylinder plane".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id,
                name: "Recognized circle".to_owned(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad_id,
                definition_id,
                name: "Recognized cylinder".to_owned(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(
                        Dimension::new(format!("{:.17}", cylinder.height_mm), cylinder.height_mm)
                            .map_err(|_| MeshConversionError::InvalidCandidate)?,
                    ),
                }),
            },
        ],
        producer_feature_id: pad_id,
    })
}

fn reflect_profile_y(profile: &mut FeatureKind) {
    if let FeatureKind::Profile { points_mm } = profile {
        for point in points_mm.iter_mut() {
            point[1] = -point[1];
        }
        points_mm.reverse();
    }
}

fn ensure_counter_clockwise_profile(profile: &mut FeatureKind) {
    let FeatureKind::Profile { points_mm } = profile else {
        return;
    };
    let twice_area = points_mm
        .iter()
        .zip(points_mm.iter().cycle().skip(1))
        .take(points_mm.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    if twice_area < 0.0 {
        points_mm.reverse();
    }
}

fn perpendicular_basis(axis: [f64; 3]) -> Result<[[f64; 3]; 2], MeshConversionError> {
    let axis_length = length_3d(axis);
    if !axis_length.is_finite() || axis_length <= f64::EPSILON {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let axis = scale_3d(axis, axis_length.recip());
    let helper = if axis[0].abs() <= axis[1].abs() && axis[0].abs() <= axis[2].abs() {
        [1.0, 0.0, 0.0]
    } else if axis[1].abs() <= axis[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let basis_u = cross_3d(helper, axis);
    let basis_u_length = length_3d(basis_u);
    if !basis_u_length.is_finite() || basis_u_length <= f64::EPSILON {
        return Err(MeshConversionError::InvalidCandidate);
    }
    let basis_u = scale_3d(basis_u, basis_u_length.recip());
    let basis_v = cross_3d(axis, basis_u);
    Ok([basis_u, basis_v])
}

struct VerificationWorkBudget {
    remaining: usize,
}

impl VerificationWorkBudget {
    const fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }

    fn consume(&mut self, units: usize) -> Result<(), MeshConversionError> {
        self.remaining = self
            .remaining
            .checked_sub(units)
            .ok_or(MeshConversionError::VerificationResourceLimit)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct AxisAlignedBounds {
    minimum: [f64; 3],
    maximum: [f64; 3],
}

impl AxisAlignedBounds {
    fn from_triangle(vertices: &[[f64; 3]], triangle: [u32; 3]) -> Self {
        let [a, b, c] = triangle.map(|index| vertices[index as usize]);
        let mut bounds = Self {
            minimum: a,
            maximum: a,
        };
        bounds.include_point(b);
        bounds.include_point(c);
        bounds
    }

    fn include_point(&mut self, point: [f64; 3]) {
        for (axis, value) in point.into_iter().enumerate() {
            self.minimum[axis] = self.minimum[axis].min(value);
            self.maximum[axis] = self.maximum[axis].max(value);
        }
    }

    fn include_bounds(&mut self, other: Self) {
        self.include_point(other.minimum);
        self.include_point(other.maximum);
    }

    fn distance_squared(self, point: [f64; 3]) -> f64 {
        (0..3)
            .map(|axis| {
                if point[axis] < self.minimum[axis] {
                    self.minimum[axis] - point[axis]
                } else if point[axis] > self.maximum[axis] {
                    point[axis] - self.maximum[axis]
                } else {
                    0.0
                }
            })
            .map(|distance| distance * distance)
            .sum()
    }
}

struct TriangleSpatialEntry {
    face_index: usize,
    bounds: AxisAlignedBounds,
    centroid: [f64; 3],
}

struct TriangleBvhNode {
    bounds: AxisAlignedBounds,
    kind: TriangleBvhNodeKind,
}

enum TriangleBvhNodeKind {
    Leaf(Vec<usize>),
    Branch { left: usize, right: usize },
}

struct TriangleSpatialIndex {
    nodes: Vec<TriangleBvhNode>,
    root: usize,
}

impl TriangleSpatialIndex {
    fn build(
        vertices: &[[f64; 3]],
        faces: &[[u32; 3]],
        cancelled: &AtomicBool,
        budget: &mut VerificationWorkBudget,
    ) -> Result<Self, MeshConversionError> {
        let mut entries = Vec::with_capacity(faces.len());
        for (face_index, triangle) in faces.iter().copied().enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(MeshConversionError::Cancelled);
            }
            budget.consume(1)?;
            let [a, b, c] = triangle.map(|index| vertices[index as usize]);
            entries.push(TriangleSpatialEntry {
                face_index,
                bounds: AxisAlignedBounds::from_triangle(vertices, triangle),
                centroid: scale_3d(add_3d(add_3d(a, b), c), 1.0 / 3.0),
            });
        }
        if entries.is_empty() {
            return Err(MeshConversionError::ExactVerificationMismatch);
        }
        let mut nodes = Vec::with_capacity(faces.len().saturating_mul(2));
        let root = Self::build_node(entries, &mut nodes, cancelled, budget)?;
        Ok(Self { nodes, root })
    }

    fn build_node(
        entries: Vec<TriangleSpatialEntry>,
        nodes: &mut Vec<TriangleBvhNode>,
        cancelled: &AtomicBool,
        budget: &mut VerificationWorkBudget,
    ) -> Result<usize, MeshConversionError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(MeshConversionError::Cancelled);
        }
        budget.consume(
            entries
                .len()
                .checked_mul(3)
                .ok_or(MeshConversionError::VerificationResourceLimit)?,
        )?;
        let mut bounds = entries[0].bounds;
        let mut centroid_bounds = AxisAlignedBounds {
            minimum: entries[0].centroid,
            maximum: entries[0].centroid,
        };
        for entry in entries.iter().skip(1) {
            if cancelled.load(Ordering::Acquire) {
                return Err(MeshConversionError::Cancelled);
            }
            bounds.include_bounds(entry.bounds);
            centroid_bounds.include_point(entry.centroid);
        }
        if entries.len() <= TRIANGLE_BVH_LEAF_SIZE {
            let node = nodes.len();
            nodes.push(TriangleBvhNode {
                bounds,
                kind: TriangleBvhNodeKind::Leaf(
                    entries.into_iter().map(|entry| entry.face_index).collect(),
                ),
            });
            return Ok(node);
        }

        let mut axis = 0;
        for candidate in 1..3 {
            let candidate_extent =
                centroid_bounds.maximum[candidate] - centroid_bounds.minimum[candidate];
            let current_extent = centroid_bounds.maximum[axis] - centroid_bounds.minimum[axis];
            if candidate_extent.total_cmp(&current_extent).is_gt() {
                axis = candidate;
            }
        }
        let split = (centroid_bounds.minimum[axis] + centroid_bounds.maximum[axis]) * 0.5;
        let mut left_count = 0;
        for entry in &entries {
            if cancelled.load(Ordering::Acquire) {
                return Err(MeshConversionError::Cancelled);
            }
            left_count += usize::from(entry.centroid[axis] < split);
        }
        let minimum_balanced = entries.len() / 8;
        let (left_entries, right_entries) =
            if left_count < minimum_balanced || entries.len() - left_count < minimum_balanced {
                let mut entries = entries;
                let right = entries.split_off(entries.len() / 2);
                (entries, right)
            } else {
                let mut left = Vec::with_capacity(left_count);
                let mut right = Vec::with_capacity(entries.len() - left_count);
                for entry in entries {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(MeshConversionError::Cancelled);
                    }
                    if entry.centroid[axis] < split {
                        left.push(entry);
                    } else {
                        right.push(entry);
                    }
                }
                (left, right)
            };
        let left = Self::build_node(left_entries, nodes, cancelled, budget)?;
        let right = Self::build_node(right_entries, nodes, cancelled, budget)?;
        let node = nodes.len();
        nodes.push(TriangleBvhNode {
            bounds,
            kind: TriangleBvhNodeKind::Branch { left, right },
        });
        Ok(node)
    }

    fn nearest_distance(
        &self,
        point: [f64; 3],
        vertices: &[[f64; 3]],
        faces: &[[u32; 3]],
        cancelled: &AtomicBool,
        budget: &mut VerificationWorkBudget,
    ) -> Result<f64, MeshConversionError> {
        TriangleNearestQuery {
            index: self,
            vertices,
            faces,
            cancelled,
            budget,
        }
        .nearest_distance_from(self.root, point, f64::INFINITY)
    }
}

struct TriangleNearestQuery<'a> {
    index: &'a TriangleSpatialIndex,
    vertices: &'a [[f64; 3]],
    faces: &'a [[u32; 3]],
    cancelled: &'a AtomicBool,
    budget: &'a mut VerificationWorkBudget,
}

impl TriangleNearestQuery<'_> {
    fn nearest_distance_from(
        &mut self,
        node_index: usize,
        point: [f64; 3],
        mut nearest: f64,
    ) -> Result<f64, MeshConversionError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(MeshConversionError::Cancelled);
        }
        self.budget.consume(1)?;
        let node = &self.index.nodes[node_index];
        if node.bounds.distance_squared(point) > nearest * nearest {
            return Ok(nearest);
        }
        match &node.kind {
            TriangleBvhNodeKind::Leaf(face_indices) => {
                for face_index in face_indices {
                    if self.cancelled.load(Ordering::Acquire) {
                        return Err(MeshConversionError::Cancelled);
                    }
                    self.budget.consume(1)?;
                    let [a, b, c] =
                        self.faces[*face_index].map(|index| self.vertices[index as usize]);
                    let distance = point_triangle_distance(point, a, b, c);
                    if distance.total_cmp(&nearest).is_lt() {
                        nearest = distance;
                    }
                }
                Ok(nearest)
            }
            TriangleBvhNodeKind::Branch { left, right } => {
                let left_distance = self.index.nodes[*left].bounds.distance_squared(point);
                let right_distance = self.index.nodes[*right].bounds.distance_squared(point);
                let (first, second, second_distance) = if left_distance <= right_distance {
                    (*left, *right, right_distance)
                } else {
                    (*right, *left, left_distance)
                };
                nearest = self.nearest_distance_from(first, point, nearest)?;
                if second_distance <= nearest * nearest {
                    nearest = self.nearest_distance_from(second, point, nearest)?;
                }
                Ok(nearest)
            }
        }
    }
}

fn directed_mesh_sample_distance(
    sample_vertices: &[[f64; 3]],
    sample_faces: &[[u32; 3]],
    target_vertices: &[[f64; 3]],
    target_faces: &[[u32; 3]],
    cancelled: &AtomicBool,
    budget: &mut VerificationWorkBudget,
) -> Result<f64, MeshConversionError> {
    let target_index =
        TriangleSpatialIndex::build(target_vertices, target_faces, cancelled, budget)?;
    let mut maximum: f64 = 0.0;
    for face in sample_faces {
        if cancelled.load(Ordering::Acquire) {
            return Err(MeshConversionError::Cancelled);
        }
        budget.consume(1)?;
        let [a, b, c] = face.map(|index| sample_vertices[index as usize]);
        let samples = [
            a,
            b,
            c,
            midpoint(a, b),
            midpoint(b, c),
            midpoint(c, a),
            scale_3d(add_3d(add_3d(a, b), c), 1.0 / 3.0),
        ];
        for point in samples {
            let minimum = target_index.nearest_distance(
                point,
                target_vertices,
                target_faces,
                cancelled,
                budget,
            )?;
            if !minimum.is_finite() {
                return Err(MeshConversionError::ExactVerificationMismatch);
            }
            maximum = maximum.max(minimum);
        }
    }
    Ok(maximum)
}

fn point_triangle_distance(point: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let ab = subtract_3d(b, a);
    let ac = subtract_3d(c, a);
    let ap = subtract_3d(point, a);
    let d1 = dot_3d(ab, ap);
    let d2 = dot_3d(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return length_3d(ap);
    }
    let bp = subtract_3d(point, b);
    let d3 = dot_3d(ab, bp);
    let d4 = dot_3d(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return length_3d(bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return length_3d(subtract_3d(point, add_3d(a, scale_3d(ab, v))));
    }
    let cp = subtract_3d(point, c);
    let d5 = dot_3d(ab, cp);
    let d6 = dot_3d(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return length_3d(cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return length_3d(subtract_3d(point, add_3d(a, scale_3d(ac, w))));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let edge = subtract_3d(c, b);
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return length_3d(subtract_3d(point, add_3d(b, scale_3d(edge, w))));
    }
    let denominator = 1.0 / (va + vb + vc);
    let v = vb * denominator;
    let w = vc * denominator;
    length_3d(subtract_3d(
        point,
        add_3d(a, add_3d(scale_3d(ab, v), scale_3d(ac, w))),
    ))
}

fn exact_graph_error(error: ExactBRepGraphError) -> MeshConversionError {
    MeshConversionError::ExactGraph(error.to_string())
}

fn midpoint(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    scale_3d(add_3d(left, right), 0.5)
}

fn add_3d(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract_3d(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale_3d(value: [f64; 3], scale: f64) -> [f64; 3] {
    [value[0] * scale, value[1] * scale, value[2] * scale]
}

fn dot_3d(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross_3d(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn length_3d(value: [f64; 3]) -> f64 {
    dot_3d(value, value).sqrt()
}

fn determinant(u: [f64; 3], v: [f64; 3], w: [f64; 3]) -> f64 {
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
        + u[2] * (v[0] * w[1] - v[1] * w[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oblique_cylinder_axis_produces_an_orthonormal_right_handed_basis() {
        let half_sqrt_two = 0.5_f64.sqrt();
        let axis = [half_sqrt_two, half_sqrt_two, 0.0];
        let [basis_u, basis_v] = perpendicular_basis(axis).unwrap();

        assert!(dot_3d(axis, basis_u).abs() <= 1.0e-12);
        assert!(dot_3d(axis, basis_v).abs() <= 1.0e-12);
        assert!((length_3d(basis_u) - 1.0).abs() <= 1.0e-12);
        assert!((length_3d(basis_v) - 1.0).abs() <= 1.0e-12);
        assert!((determinant(basis_u, basis_v, axis) - 1.0).abs() <= 1.0e-12);
        assert_eq!(
            perpendicular_basis([0.0, 0.0, 0.0]),
            Err(MeshConversionError::InvalidCandidate)
        );
    }

    #[test]
    fn timed_out_wait_reaps_an_unresponsive_worker_asynchronously() {
        let document = DocumentStore::new();
        let source = MeshConversionSource::new(&document.current(), document.mutation_epoch());
        let cancelled = Arc::new(AtomicBool::new(false));
        let (_event_sender, receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let _ = release_receiver.recv();
        });
        let task = MeshConversionTask {
            source,
            cancelled,
            receiver,
            worker: Some(worker),
        };

        let started_at = Instant::now();
        let result = task.wait(Duration::ZERO);
        let elapsed = started_at.elapsed();
        let _ = release_sender.send(());

        assert!(matches!(result, Err(MeshConversionError::TimedOut)));
        assert!(elapsed < Duration::from_millis(500), "elapsed={elapsed:?}");
    }
}
