//! Steady heat conduction through one exact part: the worker meshes the
//! part at two to four ever finer levels, the faces carry held
//! temperatures, heat fluxes or convection, and each level is solved so the
//! last two show whether the answer has settled.
//!
//! The conductivity is the part's material's in the rules
//! (`validation_rules.json`) with the standard it comes from; a material the
//! rules give none for is refused, never given a default.

use crate::fea_workflow::{refinement_levels_are_valid, relative_change};
use crate::validation_rules::{ValidationRules, occurrence_materials};
use ketchup_analysis::fea_thermal::{
    ThermalError, ThermalMaterial, ThermalSolution, ThermalSolveSettings,
    solve_steady_conduction_with_cancellation,
};
use ketchup_model::document::{
    ClassificationError, DefinitionId, FeatureId, InstancePath, Snapshot,
};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError};
use ketchup_model::graph::sha256_bytes;
pub use ketchup_scheduler::ExactVolumeMeshWireOptions;
pub use ketchup_scheduler::thermal_setup::{
    ExactThermalFaceConvection, ExactThermalFaceFlux, ExactThermalFaceTemperature,
    ExactThermalSetup,
};
use ketchup_scheduler::{ExactFeaSetupError, ExactWorkerSupervisor, WorkerError};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalStudyRequest {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub case_id: String,
    pub instance_path: InstancePath,
    pub held_faces: Vec<ExactThermalFaceTemperature>,
    pub flux_faces: Vec<ExactThermalFaceFlux>,
    pub convection_faces: Vec<ExactThermalFaceConvection>,
    pub mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    pub solve_settings: ThermalSolveSettings,
    /// The largest relative change between the last two levels that counts
    /// as settled.
    pub convergence_relative_tolerance: f64,
}

/// The part's material and where its conductivity comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalMaterialSource {
    pub material: String,
    pub conductivity_w_per_m_k: f64,
    pub citation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalMeshLevelSummary {
    pub node_count: usize,
    pub element_count: usize,
    pub mesh_fingerprint: String,
    pub relative_volume_error: f64,
    pub minimum_quality: f64,
    pub minimum_temperature_c: f64,
    pub maximum_temperature_c: f64,
    pub heat_into_held_faces_w: f64,
    pub heat_out_of_held_faces_w: f64,
    pub flux_heat_in_w: f64,
    pub convection_heat_in_w: f64,
    pub heat_balance_w: f64,
    pub maximum_free_node_residual_w: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalReviewSummary {
    pub review_digest: String,
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub graph_digest: String,
    pub case_id: String,
    pub units: &'static str,
    pub solver_version: &'static str,
    pub material: ThermalMaterialSource,
    pub levels: Vec<ThermalMeshLevelSummary>,
    /// Change of the hottest and coldest temperatures between the last two
    /// levels, relative to the temperature span.
    pub final_temperature_relative_change: f64,
    /// Change of the heat flowing through the part between the last two levels.
    pub final_heat_flow_relative_change: f64,
    pub required_relative_tolerance: f64,
    pub converged: bool,
    pub excluded_physics: [&'static str; 5],
}

#[derive(Debug, PartialEq)]
pub enum ThermalReviewError {
    ConfirmationRequired,
    WorkerUnavailable,
    InvalidRefinementLevels,
    InvalidTolerance,
    InvalidTarget(ExactBRepGraphError),
    /// The part's material classification cannot be read.
    Classification(ClassificationError),
    /// The rules give the part's material no thermal conductivity.
    NoConductivity {
        material: String,
    },
    Meshing(WorkerError),
    Setup(ExactFeaSetupError),
    MeshNotRefined,
    Solve(ThermalError),
}

impl std::fmt::Display for ThermalReviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationRequired => {
                formatter.write_str("thermal solve requires explicit confirmation")
            }
            Self::WorkerUnavailable => formatter.write_str("exact worker is unavailable"),
            Self::InvalidRefinementLevels => formatter.write_str(
                "thermal analysis requires two to four strictly refined bounded mesh levels",
            ),
            Self::InvalidTolerance => formatter
                .write_str("the convergence tolerance must be a finite number between 0 and 1"),
            Self::InvalidTarget(cause) => write!(formatter, "thermal target rejected: {cause}"),
            Self::Classification(cause) => {
                write!(formatter, "the part's material cannot be read: {cause}")
            }
            Self::NoConductivity { material } => write!(
                formatter,
                "validation_rules.json gives material {material:?} no thermal conductivity: classify the part as a material that has one (ketchup.material.v1) or add thermal_conductivity with its source"
            ),
            Self::Meshing(cause) => write!(formatter, "thermal meshing rejected: {cause}"),
            Self::Setup(cause) => write!(formatter, "thermal setup rejected: {cause}"),
            Self::MeshNotRefined => {
                formatter.write_str("thermal solve rejected: the mesh levels are not finer")
            }
            Self::Solve(cause) => write!(formatter, "thermal solve rejected: {cause:?}"),
        }
    }
}

impl std::error::Error for ThermalReviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTarget(cause) => Some(cause),
            Self::Classification(cause) => Some(cause),
            Self::Meshing(cause) => Some(cause),
            Self::Solve(_)
            | Self::Setup(_)
            | Self::ConfirmationRequired
            | Self::WorkerUnavailable
            | Self::InvalidRefinementLevels
            | Self::InvalidTolerance
            | Self::NoConductivity { .. }
            | Self::MeshNotRefined => None,
        }
    }
}

/// The conductivity of the occurrence's material: its `ketchup.material.v1`
/// class, or the rules' default material when it has none.
///
/// # Errors
/// The classification cannot be read, or the rules give the material no
/// thermal conductivity.
pub fn thermal_material(
    snapshot: &Snapshot,
    instance_path: &InstancePath,
    rules: &ValidationRules,
) -> Result<ThermalMaterialSource, ThermalReviewError> {
    let materials = occurrence_materials(snapshot).map_err(ThermalReviewError::Classification)?;
    let material = materials
        .get(&instance_path.root_occurrence())
        .cloned()
        .unwrap_or_else(|| rules.default_material.clone());
    match rules
        .materials
        .get(&material)
        .and_then(|properties| properties.thermal_conductivity.as_ref())
    {
        Some(conductivity) => Ok(ThermalMaterialSource {
            conductivity_w_per_m_k: conductivity.w_m_k,
            citation: conductivity.source.clone(),
            material,
        }),
        None => Err(ThermalReviewError::NoConductivity { material }),
    }
}

pub struct ThermalReviewWorkflow {
    worker_path: Option<PathBuf>,
}

impl ThermalReviewWorkflow {
    #[must_use]
    pub const fn new(worker_path: Option<PathBuf>) -> Self {
        Self { worker_path }
    }

    /// Meshes, binds and solves every level of the study.
    ///
    /// # Errors
    /// An unconfirmed study, bad levels, a part without a conductivity, a
    /// meshing, setup or solve refusal, or levels that are not finer.
    pub fn review(
        &self,
        snapshot: &Snapshot,
        request: &ThermalStudyRequest,
        rules: &ValidationRules,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<ThermalReviewSummary, ThermalReviewError> {
        if !confirmed {
            return Err(ThermalReviewError::ConfirmationRequired);
        }
        if !refinement_levels_are_valid(&request.mesh_levels) {
            return Err(ThermalReviewError::InvalidRefinementLevels);
        }
        let tolerance = request.convergence_relative_tolerance;
        if !tolerance.is_finite() || tolerance <= 0.0 || tolerance >= 1.0 {
            return Err(ThermalReviewError::InvalidTolerance);
        }
        let material = thermal_material(snapshot, &request.instance_path, rules)?;
        let graph =
            ExactBRepGraph::from_snapshot(snapshot, request.definition_id, request.feature_id)
                .map_err(ThermalReviewError::InvalidTarget)?;
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(
            self.worker_path
                .as_deref()
                .ok_or(ThermalReviewError::WorkerUnavailable)?,
            cancelled,
        )
        .map_err(ThermalReviewError::Meshing)?;
        let setup = ExactThermalSetup {
            case_id: request.case_id.clone(),
            instance_path: request.instance_path.clone(),
            material: ThermalMaterial {
                id: 1,
                conductivity_w_per_m_k: material.conductivity_w_per_m_k,
            },
            held_faces: request.held_faces.clone(),
            flux_faces: request.flux_faces.clone(),
            convection_faces: request.convection_faces.clone(),
        };

        let mut levels = Vec::with_capacity(request.mesh_levels.len());
        let mut solutions = Vec::with_capacity(request.mesh_levels.len());
        for options in &request.mesh_levels {
            let package = worker
                .evaluate_exact_brep_graph_volume_mesh_with_cancellation(
                    &graph, *options, cancelled,
                )
                .map_err(ThermalReviewError::Meshing)?;
            if levels
                .last()
                .is_some_and(|previous: &ThermalMeshLevelSummary| {
                    package.mesh.tetrahedra.len() <= previous.element_count
                        || package.mesh.mesh_fingerprint == previous.mesh_fingerprint
                })
            {
                return Err(ThermalReviewError::MeshNotRefined);
            }
            let bound = package
                .occurrence_bound_thermal_model(snapshot, &setup)
                .map_err(ThermalReviewError::Setup)?;
            let solution = solve_steady_conduction_with_cancellation(
                &bound.model,
                request.solve_settings,
                cancelled,
            )
            .map_err(ThermalReviewError::Solve)?;
            levels.push(level_summary(&package.mesh, &solution));
            solutions.push(solution);
        }
        let [.., previous, last] = levels.as_slice() else {
            return Err(ThermalReviewError::InvalidRefinementLevels);
        };
        let span = levels
            .iter()
            .map(|level| level.maximum_temperature_c - level.minimum_temperature_c)
            .fold(0.0, f64::max);
        let final_temperature_relative_change = if span > 0.0 {
            (last.maximum_temperature_c - previous.maximum_temperature_c)
                .abs()
                .max((last.minimum_temperature_c - previous.minimum_temperature_c).abs())
                / span
        } else {
            0.0
        };
        let final_heat_flow_relative_change = relative_change(heat_flow(previous), heat_flow(last));
        let converged = final_temperature_relative_change <= tolerance
            && final_heat_flow_relative_change <= tolerance;
        let canonical_digest = snapshot.canonical_digest();
        let review_digest = review_digest(
            snapshot,
            &canonical_digest,
            &graph.graph_digest,
            request,
            &material,
            &levels,
        );
        let final_solution = &solutions[solutions.len() - 1];
        Ok(ThermalReviewSummary {
            review_digest,
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest,
            graph_digest: graph.graph_digest,
            case_id: request.case_id.clone(),
            units: "mm,°C,W",
            solver_version: final_solution.solver_version,
            material,
            final_temperature_relative_change,
            final_heat_flow_relative_change,
            required_relative_tolerance: tolerance,
            converged,
            excluded_physics: final_solution.excluded_physics,
            levels,
        })
    }
}

fn level_summary(
    mesh: &ketchup_scheduler::WorkerExactVolumeMesh,
    solution: &ThermalSolution,
) -> ThermalMeshLevelSummary {
    ThermalMeshLevelSummary {
        node_count: mesh.vertices_mm.len(),
        element_count: mesh.tetrahedra.len(),
        mesh_fingerprint: mesh.mesh_fingerprint.clone(),
        relative_volume_error: mesh.relative_volume_error,
        minimum_quality: mesh.minimum_quality,
        minimum_temperature_c: solution.minimum_temperature_c,
        maximum_temperature_c: solution.maximum_temperature_c,
        heat_into_held_faces_w: solution.heat_into_held_nodes_w,
        heat_out_of_held_faces_w: solution.heat_out_of_held_nodes_w,
        flux_heat_in_w: solution.flux_heat_in_w,
        convection_heat_in_w: solution.convection_heat_in_w,
        heat_balance_w: solution.heat_balance_w,
        maximum_free_node_residual_w: solution.maximum_free_node_residual_w,
    }
}

/// The heat passing through the part: half of all heat in and out.
fn heat_flow(level: &ThermalMeshLevelSummary) -> f64 {
    (level.heat_into_held_faces_w
        + level.heat_out_of_held_faces_w
        + level.flux_heat_in_w.abs()
        + level.convection_heat_in_w.abs())
        / 2.0
}

fn review_digest(
    snapshot: &Snapshot,
    canonical_digest: &str,
    graph_digest: &str,
    request: &ThermalStudyRequest,
    material: &ThermalMaterialSource,
    levels: &[ThermalMeshLevelSummary],
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&snapshot.document_id().0.to_le_bytes());
    bytes.extend_from_slice(&snapshot.revision_id().to_le_bytes());
    let texts = [
        canonical_digest,
        graph_digest,
        request.case_id.as_str(),
        material.material.as_str(),
        material.citation.as_str(),
    ];
    for value in texts
        .into_iter()
        .chain(levels.iter().map(|level| level.mesh_fingerprint.as_str()))
    {
        bytes.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    for level in levels {
        for value in [
            level.minimum_temperature_c,
            level.maximum_temperature_c,
            heat_flow(level),
        ] {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    bytes.extend_from_slice(&material.conductivity_w_per_m_k.to_bits().to_le_bytes());
    let faces = request
        .held_faces
        .iter()
        .map(|face| (0_u8, face.face_ordinal, [face.temperature_c, 0.0]))
        .chain(
            request
                .flux_faces
                .iter()
                .map(|face| (1, face.face_ordinal, [face.flux_w_per_m2, 0.0])),
        )
        .chain(request.convection_faces.iter().map(|face| {
            (
                2,
                face.face_ordinal,
                [face.film_coefficient_w_per_m2_k, face.ambient_c],
            )
        }));
    for (kind, ordinal, values) in faces {
        bytes.push(kind);
        bytes.extend_from_slice(&ordinal.to_le_bytes());
        for value in values {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    sha256_bytes(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
