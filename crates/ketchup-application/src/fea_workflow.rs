use ketchup_core::document::{DefinitionId, FeatureId, Snapshot};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::fea::FeaSolveSettings;
use ketchup_core::graph::sha256_bytes;
use ketchup_scheduler::ExactWorkerSupervisor;
pub use ketchup_scheduler::{ExactFeaFaceTraction, ExactFeaSetup, ExactVolumeMeshWireOptions};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub const MIN_FEA_REFINEMENT_LEVELS: usize = 2;
pub const MAX_FEA_REFINEMENT_LEVELS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct FeaStudyRequest {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub setup: ExactFeaSetup,
    pub mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    pub solve_settings: FeaSolveSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaMeshLevelSummary {
    pub node_count: usize,
    pub element_count: usize,
    pub mesh_fingerprint: String,
    pub relative_volume_error: f64,
    pub minimum_quality: f64,
    pub maximum_edge_ratio: f64,
    pub maximum_displacement_mm: f64,
    pub maximum_von_mises_stress_mpa: f64,
    pub maximum_free_dof_residual_n: f64,
    pub force_balance_n: [f64; 3],
    pub within_declared_limits: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaReviewSummary {
    pub review_digest: String,
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub graph_digest: String,
    pub case_id: String,
    pub units: &'static str,
    pub solver_version: &'static str,
    pub levels: Vec<FeaMeshLevelSummary>,
    pub final_displacement_relative_change: f64,
    pub final_energy_relative_change: f64,
    pub required_relative_tolerance: f64,
    pub converged: bool,
    pub all_levels_within_declared_limits: bool,
    pub excluded_physics: [&'static str; 4],
}

#[derive(Debug, PartialEq)]
pub enum FeaReviewError {
    ConfirmationRequired,
    WorkerUnavailable,
    InvalidRefinementLevels,
    InvalidTarget(String),
    Meshing(String),
    Setup(String),
    Solve(String),
}

impl std::fmt::Display for FeaReviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationRequired => {
                formatter.write_str("FEA solve requires explicit confirmation")
            }
            Self::WorkerUnavailable => formatter.write_str("exact worker is unavailable"),
            Self::InvalidRefinementLevels => {
                formatter.write_str("FEA requires two to four strictly refined bounded mesh levels")
            }
            Self::InvalidTarget(reason) => write!(formatter, "FEA target rejected: {reason}"),
            Self::Meshing(reason) => write!(formatter, "FEA meshing rejected: {reason}"),
            Self::Setup(reason) => write!(formatter, "FEA setup rejected: {reason}"),
            Self::Solve(reason) => write!(formatter, "FEA solve rejected: {reason}"),
        }
    }
}

impl std::error::Error for FeaReviewError {}

pub struct FeaReviewWorkflow {
    worker_path: Option<PathBuf>,
}

impl FeaReviewWorkflow {
    #[must_use]
    pub const fn new(worker_path: Option<PathBuf>) -> Self {
        Self { worker_path }
    }

    pub fn set_worker_path(&mut self, worker_path: PathBuf) {
        self.worker_path = Some(worker_path);
    }

    pub fn review(
        &self,
        snapshot: &Snapshot,
        request: &FeaStudyRequest,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<FeaReviewSummary, FeaReviewError> {
        if !confirmed {
            return Err(FeaReviewError::ConfirmationRequired);
        }
        validate_refinement_levels(&request.mesh_levels)?;
        let graph =
            ExactBRepGraph::from_snapshot(snapshot, request.definition_id, request.feature_id)
                .map_err(|error| FeaReviewError::InvalidTarget(error.to_string()))?;
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(
            self.worker_path
                .as_deref()
                .ok_or(FeaReviewError::WorkerUnavailable)?,
            cancelled,
        )
        .map_err(|error| FeaReviewError::Meshing(error.to_string()))?;

        let mut models = Vec::with_capacity(request.mesh_levels.len());
        let mut packages = Vec::with_capacity(request.mesh_levels.len());
        for options in &request.mesh_levels {
            let package = worker
                .evaluate_exact_brep_graph_volume_mesh_with_cancellation(
                    &graph, *options, cancelled,
                )
                .map_err(|error| FeaReviewError::Meshing(error.to_string()))?;
            let bound = package
                .occurrence_bound_model(snapshot, &request.setup)
                .map_err(|error| FeaReviewError::Setup(format!("{error:?}")))?;
            models.push(bound.model);
            packages.push(package);
        }
        if packages.windows(2).any(|pair| {
            pair[1].mesh.tetrahedra.len() <= pair[0].mesh.tetrahedra.len()
                || pair[1].mesh.mesh_fingerprint == pair[0].mesh.mesh_fingerprint
        }) {
            return Err(FeaReviewError::Solve("MeshNotRefined".to_owned()));
        }
        let solutions = models
            .iter()
            .map(|model| {
                model
                    .solve(request.solve_settings)
                    .map_err(|error| FeaReviewError::Solve(format!("{error:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let previous = &solutions[solutions.len() - 2];
        let final_solution = solutions
            .last()
            .expect("a reviewed study has at least two solutions");
        let final_displacement_relative_change = relative_change(
            previous.maximum_displacement_mm,
            final_solution.maximum_displacement_mm,
        );
        let final_energy_relative_change = relative_change(
            previous.total_strain_energy_n_mm,
            final_solution.total_strain_energy_n_mm,
        );
        let converged = final_displacement_relative_change
            <= request.solve_settings.convergence_relative_tolerance
            && final_energy_relative_change
                <= request.solve_settings.convergence_relative_tolerance;
        let levels = packages
            .iter()
            .zip(&solutions)
            .map(|(package, solution)| FeaMeshLevelSummary {
                node_count: package.mesh.vertices_mm.len(),
                element_count: package.mesh.tetrahedra.len(),
                mesh_fingerprint: package.mesh.mesh_fingerprint.clone(),
                relative_volume_error: package.mesh.relative_volume_error,
                minimum_quality: package.mesh.minimum_quality,
                maximum_edge_ratio: package.mesh.maximum_edge_ratio,
                maximum_displacement_mm: solution.maximum_displacement_mm,
                maximum_von_mises_stress_mpa: solution.maximum_von_mises_stress_mpa,
                maximum_free_dof_residual_n: solution.maximum_free_dof_residual_n,
                force_balance_n: solution.force_balance_n,
                within_declared_limits: solution.is_within_declared_limits(),
            })
            .collect::<Vec<_>>();
        let all_levels_within_declared_limits =
            levels.iter().all(|level| level.within_declared_limits);
        let excluded_physics = final_solution.validity.excluded_physics;
        let canonical_digest = snapshot.canonical_digest();
        let review_digest = review_digest(
            snapshot.document_id().0,
            snapshot.revision_id(),
            &canonical_digest,
            &graph.graph_digest,
            &request.setup.case_id,
            &packages
                .iter()
                .map(|package| package.mesh.mesh_fingerprint.as_str())
                .collect::<Vec<_>>(),
            final_displacement_relative_change,
            final_energy_relative_change,
        );
        Ok(FeaReviewSummary {
            review_digest,
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest,
            graph_digest: graph.graph_digest,
            case_id: request.setup.case_id.clone(),
            units: "mm,N,MPa",
            solver_version: solutions[0].solver_version,
            levels,
            final_displacement_relative_change,
            final_energy_relative_change,
            required_relative_tolerance: request.solve_settings.convergence_relative_tolerance,
            converged,
            all_levels_within_declared_limits,
            excluded_physics,
        })
    }
}

fn validate_refinement_levels(levels: &[ExactVolumeMeshWireOptions]) -> Result<(), FeaReviewError> {
    if !(MIN_FEA_REFINEMENT_LEVELS..=MAX_FEA_REFINEMENT_LEVELS).contains(&levels.len())
        || levels.iter().any(|level| {
            !level.surface_deflection_mm.is_finite()
                || level.surface_deflection_mm <= 0.0
                || !level.angular_deflection_rad.is_finite()
                || level.angular_deflection_rad <= 0.0
                || level.max_tetrahedra == 0
                || level.max_tetrahedra > 4096
                || !level.max_relative_volume_error.is_finite()
                || !(0.0..1.0).contains(&level.max_relative_volume_error)
                || !level.min_tetrahedron_quality.is_finite()
                || !(0.0..=1.0).contains(&level.min_tetrahedron_quality)
        })
        || levels.windows(2).any(|pair| {
            pair[1].surface_deflection_mm >= pair[0].surface_deflection_mm
                || pair[1].angular_deflection_rad >= pair[0].angular_deflection_rad
                || pair[1].max_tetrahedra < pair[0].max_tetrahedra
        })
    {
        return Err(FeaReviewError::InvalidRefinementLevels);
    }
    Ok(())
}

fn relative_change(previous: f64, current: f64) -> f64 {
    (current - previous).abs() / current.abs().max(previous.abs()).max(1.0e-15)
}

fn review_digest(
    document_id: u64,
    revision: u64,
    canonical_digest: &str,
    graph_digest: &str,
    case_id: &str,
    mesh_fingerprints: &[&str],
    displacement_change: f64,
    energy_change: f64,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&document_id.to_le_bytes());
    bytes.extend_from_slice(&revision.to_le_bytes());
    for value in [canonical_digest, graph_digest, case_id] {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    for value in mesh_fingerprints {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes.extend_from_slice(&displacement_change.to_bits().to_le_bytes());
    bytes.extend_from_slice(&energy_change.to_bits().to_le_bytes());
    sha256_bytes(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
