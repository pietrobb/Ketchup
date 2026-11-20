pub use ketchup_analysis::fea::FeaConvergenceJudgement;
use ketchup_analysis::fea::{FeaError, FeaModel, FeaModes, FeaSolveSettings};
use ketchup_model::document::{DefinitionId, FeatureId, Snapshot};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::graph::sha256_bytes;
use ketchup_model::tolerance::NEGLIGIBLE;
use ketchup_scheduler::{
    ExactBRepVolumeMeshPackage, ExactFeaSetupError, ExactWorkerSupervisor, assembly_bound_model,
};
pub use ketchup_scheduler::{
    ExactFeaAssemblyPart, ExactFeaAssemblySetup, ExactFeaFaceTraction, ExactFeaJoint,
    ExactFeaJointFace, ExactFeaJointKind, ExactFeaMateJoint, ExactFeaSetup,
    ExactVolumeMeshWireOptions,
};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub const MIN_FEA_REFINEMENT_LEVELS: usize = 2;
pub const MAX_FEA_REFINEMENT_LEVELS: usize = 4;
/// The most tetrahedra the exact volume mesher makes for one level; the
/// sparse solver takes them all.
pub const MAX_FEA_MESH_TETRAHEDRA: u32 = 65_536;

#[derive(Clone, Debug, PartialEq)]
pub struct FeaStudyRequest {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
    pub setup: ExactFeaSetup,
    pub mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    pub solve_settings: FeaSolveSettings,
    /// How many of the lowest natural frequencies to find on the finest mesh;
    /// none when zero. Every material then needs a density.
    pub mode_count: usize,
}

/// Several placed parts joined face to face, studied as one model.
#[derive(Clone, Debug, PartialEq)]
pub struct FeaAssemblyStudyRequest {
    /// The solid feature meshed for each part, in the order of `setup.parts`.
    pub part_features: Vec<FeatureId>,
    pub setup: ExactFeaAssemblySetup,
    pub mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    pub solve_settings: FeaSolveSettings,
    pub mode_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaModalSummary {
    pub solver_version: &'static str,
    pub total_mass_kg: f64,
    /// From the lowest up.
    pub frequencies_hz: Vec<f64>,
    /// The largest backward error of the modes found.
    pub maximum_relative_residual: f64,
    /// The same modes on the level before the finest (R1120ac-FEA-900).
    pub previous_frequencies_hz: Vec<f64>,
    /// The largest relative change of a frequency between the two levels.
    pub maximum_relative_frequency_change: f64,
    /// Every frequency changed by at most the study's tolerance, and both
    /// levels found as many modes.
    pub converged: bool,
    pub not_checked: [&'static str; 4],
}

/// What natural frequencies leave out (R1120ac-FEA-900).
pub const FEA_MODES_NOT_CHECKED: [&str; 4] = [
    "linear tetrahedra are too stiff: frequencies fall as the mesh is refined, so a coarse mesh overstates them",
    "supports are rigid: a softer support lowers the frequencies",
    "no damping",
    "no prestress or stress stiffening from the static load",
];

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
    /// The finest level against the one before (og:R1119og-FEA-803).
    pub judgement: FeaConvergenceJudgement,
    pub required_relative_tolerance: f64,
    pub all_levels_within_declared_limits: bool,
    pub excluded_physics: [&'static str; 5],
    /// The natural frequencies of the finest mesh against the level before,
    /// when the study asked.
    pub modes: Option<FeaModalSummary>,
}

/// What one mesh level gave and the model built on it.
struct StudyLevel {
    node_count: usize,
    mesh_fingerprint: String,
    relative_volume_error: f64,
    minimum_quality: f64,
    maximum_edge_ratio: f64,
    model: FeaModel,
}

#[derive(Debug, PartialEq)]
pub enum FeaReviewError {
    ConfirmationRequired,
    WorkerUnavailable,
    InvalidRefinementLevels,
    InvalidTarget(String),
    Meshing(String),
    Setup(ExactFeaSetupError),
    MeshNotRefined,
    Solve(FeaError),
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
            Self::Setup(cause) => write!(formatter, "FEA setup rejected: {cause}"),
            Self::MeshNotRefined => formatter
                .write_str("FEA solve rejected: each mesh level must be finer than the one before"),
            Self::Solve(cause) => write!(formatter, "FEA solve rejected: {cause}"),
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
        let graph = solid(snapshot, request.definition_id, request.feature_id)?;
        let mut worker = self.worker(cancelled)?;
        let mut levels = Vec::with_capacity(request.mesh_levels.len());
        for options in &request.mesh_levels {
            let package = mesh(&mut worker, &graph, *options, cancelled)?;
            let bound = package
                .occurrence_bound_model(snapshot, &request.setup)
                .map_err(FeaReviewError::Setup)?;
            let mesh = &package.mesh;
            levels.push(StudyLevel {
                node_count: mesh.vertices_mm.len(),
                mesh_fingerprint: mesh.mesh_fingerprint.clone(),
                relative_volume_error: mesh.relative_volume_error,
                minimum_quality: mesh.minimum_quality,
                maximum_edge_ratio: mesh.maximum_edge_ratio,
                model: bound.model,
            });
        }
        conclude(
            snapshot,
            graph.graph_digest,
            &request.setup.case_id,
            &levels,
            request.solve_settings,
            request.mode_count,
            cancelled,
        )
    }

    /// Meshes each part's solid at every level (occurrences of one solid
    /// share its mesh), places and joins them as the setup says, and solves
    /// the whole assembly as one model.
    pub fn review_assembly(
        &self,
        snapshot: &Snapshot,
        request: &FeaAssemblyStudyRequest,
        confirmed: bool,
        cancelled: &AtomicBool,
    ) -> Result<FeaReviewSummary, FeaReviewError> {
        if !confirmed {
            return Err(FeaReviewError::ConfirmationRequired);
        }
        validate_refinement_levels(&request.mesh_levels)?;
        if request.part_features.len() != request.setup.parts.len() {
            return Err(FeaReviewError::Setup(ExactFeaSetupError::IncompleteSetup));
        }
        let mut solids = Vec::<((DefinitionId, FeatureId), ExactBRepGraph)>::new();
        let mut part_solids = Vec::with_capacity(request.setup.parts.len());
        for (part, feature_id) in request.setup.parts.iter().zip(&request.part_features) {
            let definition_id = snapshot
                .resolve_instance_path(&part.instance_path)
                .map_err(|error| {
                    FeaReviewError::Setup(ExactFeaSetupError::InvalidInstancePath(error))
                })?
                .definition_id;
            let key = (definition_id, *feature_id);
            let index = match solids.iter().position(|(known, _)| *known == key) {
                Some(index) => index,
                None => {
                    let graph = solid(snapshot, definition_id, *feature_id)?;
                    solids.push((key, graph));
                    solids.len() - 1
                }
            };
            part_solids.push(index);
        }
        let graph_digest = joined_digest(
            part_solids
                .iter()
                .map(|index| solids[*index].1.graph_digest.as_str()),
        );
        let mut worker = self.worker(cancelled)?;
        let mut levels = Vec::with_capacity(request.mesh_levels.len());
        for options in &request.mesh_levels {
            let packages = solids
                .iter()
                .map(|(_, graph)| mesh(&mut worker, graph, *options, cancelled))
                .collect::<Result<Vec<_>, _>>()?;
            let parts = part_solids
                .iter()
                .map(|index| &packages[*index])
                .collect::<Vec<_>>();
            let bound = assembly_bound_model(snapshot, &parts, &request.setup)
                .map_err(FeaReviewError::Setup)?;
            let meshes = || packages.iter().map(|package| &package.mesh);
            levels.push(StudyLevel {
                node_count: bound.parts.iter().map(|part| part.node_count).sum(),
                mesh_fingerprint: joined_digest(
                    parts.iter().map(|part| part.mesh.mesh_fingerprint.as_str()),
                ),
                relative_volume_error: meshes()
                    .map(|mesh| mesh.relative_volume_error)
                    .fold(0.0, f64::max),
                minimum_quality: meshes()
                    .map(|mesh| mesh.minimum_quality)
                    .fold(f64::INFINITY, f64::min),
                maximum_edge_ratio: meshes()
                    .map(|mesh| mesh.maximum_edge_ratio)
                    .fold(0.0, f64::max),
                model: bound.model,
            });
        }
        conclude(
            snapshot,
            graph_digest,
            &request.setup.case_id,
            &levels,
            request.solve_settings,
            request.mode_count,
            cancelled,
        )
    }

    fn worker(&self, cancelled: &AtomicBool) -> Result<ExactWorkerSupervisor, FeaReviewError> {
        ExactWorkerSupervisor::spawn_with_cancellation(
            self.worker_path
                .as_deref()
                .ok_or(FeaReviewError::WorkerUnavailable)?,
            cancelled,
        )
        .map_err(|error| FeaReviewError::Meshing(error.to_string()))
    }
}

fn solid(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    feature_id: FeatureId,
) -> Result<ExactBRepGraph, FeaReviewError> {
    ExactBRepGraph::from_snapshot(snapshot, definition_id, feature_id)
        .map_err(|error| FeaReviewError::InvalidTarget(error.to_string()))
}

fn mesh(
    worker: &mut ExactWorkerSupervisor,
    graph: &ExactBRepGraph,
    options: ExactVolumeMeshWireOptions,
    cancelled: &AtomicBool,
) -> Result<ExactBRepVolumeMeshPackage, FeaReviewError> {
    worker
        .evaluate_exact_brep_graph_volume_mesh_with_cancellation(graph, options, cancelled)
        .map_err(|error| FeaReviewError::Meshing(error.to_string()))
}

/// Solves every level, judges the convergence between the last two, and
/// finds the natural frequencies of the last two when asked.
fn conclude(
    snapshot: &Snapshot,
    graph_digest: String,
    case_id: &str,
    levels: &[StudyLevel],
    settings: FeaSolveSettings,
    mode_count: usize,
    cancelled: &AtomicBool,
) -> Result<FeaReviewSummary, FeaReviewError> {
    if levels.windows(2).any(|pair| {
        pair[1].model.elements.len() <= pair[0].model.elements.len()
            || pair[1].mesh_fingerprint == pair[0].mesh_fingerprint
    }) {
        return Err(FeaReviewError::MeshNotRefined);
    }
    let solutions = levels
        .iter()
        .map(|level| {
            level
                .model
                .solve_with_cancellation(settings, cancelled)
                .map_err(FeaReviewError::Solve)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ([.., previous_level, finest_level], [.., previous, final_solution]) =
        (levels, solutions.as_slice())
    else {
        return Err(FeaReviewError::InvalidRefinementLevels);
    };
    let judgement = FeaConvergenceJudgement::between(
        (previous, previous_level.model.elements.len()),
        (final_solution, finest_level.model.elements.len()),
        settings.convergence_relative_tolerance,
    );
    // Modes of the finest level alone were given as plain numbers, though
    // linear tetrahedra overstate them on a coarse mesh by tens of per cent
    // (R1120ac-FEA-900): the level before is solved too and compared.
    let modes = match mode_count {
        0 => None,
        count => {
            let [previous_found, found] = [previous_level, finest_level].map(|level| {
                level
                    .model
                    .modes_with_cancellation(settings, count, cancelled)
                    .map_err(FeaReviewError::Solve)
            });
            Some(modal_summary(
                &previous_found?,
                &found?,
                settings.convergence_relative_tolerance,
            ))
        }
    };
    let summaries = levels
        .iter()
        .zip(&solutions)
        .map(|(level, solution)| FeaMeshLevelSummary {
            node_count: level.node_count,
            element_count: level.model.elements.len(),
            mesh_fingerprint: level.mesh_fingerprint.clone(),
            relative_volume_error: level.relative_volume_error,
            minimum_quality: level.minimum_quality,
            maximum_edge_ratio: level.maximum_edge_ratio,
            maximum_displacement_mm: solution.maximum_displacement_mm,
            maximum_von_mises_stress_mpa: solution.maximum_von_mises_stress_mpa,
            maximum_free_dof_residual_n: solution.maximum_free_dof_residual_n,
            force_balance_n: solution.force_balance_n,
            within_declared_limits: solution.is_within_declared_limits(),
        })
        .collect::<Vec<_>>();
    let all_levels_within_declared_limits =
        summaries.iter().all(|level| level.within_declared_limits);
    let canonical_digest = snapshot.canonical_digest();
    let review_digest = review_digest(
        snapshot.document_id().0,
        snapshot.revision_id(),
        &canonical_digest,
        &graph_digest,
        case_id,
        &levels
            .iter()
            .map(|level| level.mesh_fingerprint.as_str())
            .collect::<Vec<_>>(),
        &judgement,
        modes.as_ref(),
    );
    Ok(FeaReviewSummary {
        review_digest,
        document_id: snapshot.document_id().0,
        revision: snapshot.revision_id(),
        canonical_digest,
        graph_digest,
        case_id: case_id.to_owned(),
        units: "mm,N,MPa",
        solver_version: final_solution.solver_version,
        levels: summaries,
        judgement,
        required_relative_tolerance: settings.convergence_relative_tolerance,
        all_levels_within_declared_limits,
        excluded_physics: final_solution.validity.excluded_physics,
        modes,
    })
}

/// The finest level's modes against those of the level before: converged when
/// both found as many and none moved by more than `tolerance`.
fn modal_summary(previous: &FeaModes, found: &FeaModes, tolerance: f64) -> FeaModalSummary {
    let frequencies = |modes: &FeaModes| -> Vec<f64> {
        modes.modes.iter().map(|mode| mode.frequency_hz).collect()
    };
    let (previous_frequencies_hz, frequencies_hz) = (frequencies(previous), frequencies(found));
    let maximum_relative_frequency_change = frequencies_hz
        .iter()
        .zip(&previous_frequencies_hz)
        .map(|(current, previous)| relative_change(*previous, *current))
        .fold(0.0, f64::max);
    FeaModalSummary {
        solver_version: found.solver_version,
        total_mass_kg: found.total_mass_kg,
        converged: previous_frequencies_hz.len() == frequencies_hz.len()
            && maximum_relative_frequency_change <= tolerance,
        frequencies_hz,
        maximum_relative_residual: found
            .modes
            .iter()
            .map(|mode| mode.relative_residual)
            .fold(0.0, f64::max),
        previous_frequencies_hz,
        maximum_relative_frequency_change,
        not_checked: FEA_MODES_NOT_CHECKED,
    }
}

fn validate_refinement_levels(levels: &[ExactVolumeMeshWireOptions]) -> Result<(), FeaReviewError> {
    if !refinement_levels_are_valid(levels) {
        return Err(FeaReviewError::InvalidRefinementLevels);
    }
    Ok(())
}

/// Two to four bounded mesh levels, each strictly finer than the one before.
pub(crate) fn refinement_levels_are_valid(levels: &[ExactVolumeMeshWireOptions]) -> bool {
    !(!(MIN_FEA_REFINEMENT_LEVELS..=MAX_FEA_REFINEMENT_LEVELS).contains(&levels.len())
        || levels.iter().any(|level| {
            !level.surface_deflection_mm.is_finite()
                || level.surface_deflection_mm <= 0.0
                || !level.angular_deflection_rad.is_finite()
                || level.angular_deflection_rad <= 0.0
                || level.max_tetrahedra == 0
                || level.max_tetrahedra > MAX_FEA_MESH_TETRAHEDRA
                || !level.max_relative_volume_error.is_finite()
                || !(0.0..1.0).contains(&level.max_relative_volume_error)
                || !level.min_tetrahedron_quality.is_finite()
                || !(0.0..=1.0).contains(&level.min_tetrahedron_quality)
        })
        || levels.windows(2).any(|pair| {
            pair[1].surface_deflection_mm >= pair[0].surface_deflection_mm
                || pair[1].angular_deflection_rad >= pair[0].angular_deflection_rad
                || pair[1].max_tetrahedra < pair[0].max_tetrahedra
        }))
}

pub(crate) fn relative_change(previous: f64, current: f64) -> f64 {
    (current - previous).abs() / current.abs().max(previous.abs()).max(NEGLIGIBLE)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `value` after its length, so no two lists of values give the same bytes.
fn push_prefixed(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

/// One digest of several.
fn joined_digest<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut bytes = Vec::new();
    for value in values {
        push_prefixed(&mut bytes, value.as_bytes());
    }
    hex(&sha256_bytes(&bytes))
}

#[allow(clippy::too_many_arguments)]
fn review_digest(
    document_id: u64,
    revision: u64,
    canonical_digest: &str,
    graph_digest: &str,
    case_id: &str,
    mesh_fingerprints: &[&str],
    judgement: &FeaConvergenceJudgement,
    modes: Option<&FeaModalSummary>,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&document_id.to_le_bytes());
    bytes.extend_from_slice(&revision.to_le_bytes());
    for value in [canonical_digest, graph_digest, case_id]
        .iter()
        .chain(mesh_fingerprints)
    {
        push_prefixed(&mut bytes, value.as_bytes());
    }
    for change in [
        judgement.final_displacement_relative_change,
        judgement.final_energy_relative_change,
        judgement.final_stress_relative_change,
    ] {
        bytes.extend_from_slice(&change.to_bits().to_le_bytes());
    }
    // A study without modes keeps the digest it had before modes existed.
    if let Some(modes) = modes {
        let values = modes
            .frequencies_hz
            .iter()
            .chain([&modes.total_mass_kg])
            .chain(&modes.previous_frequencies_hz)
            .flat_map(|value| value.to_bits().to_le_bytes())
            .collect::<Vec<_>>();
        push_prefixed(&mut bytes, &values);
    }
    hex(&sha256_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(surface_deflection_mm: f64, max_tetrahedra: u32) -> ExactVolumeMeshWireOptions {
        ExactVolumeMeshWireOptions {
            surface_deflection_mm,
            angular_deflection_rad: surface_deflection_mm,
            max_tetrahedra,
            max_relative_volume_error: 0.05,
            min_tetrahedron_quality: 1.0e-6,
        }
    }

    fn found(frequencies_hz: &[f64]) -> FeaModes {
        FeaModes {
            schema: "test",
            solver_version: "test",
            case_id: "beam".into(),
            model_digest: String::new(),
            total_mass_kg: 1.0,
            iterations: 1,
            modes: frequencies_hz
                .iter()
                .map(|frequency_hz| ketchup_analysis::fea::FeaMode {
                    frequency_hz: *frequency_hz,
                    angular_frequency_rad_s: 2.0 * std::f64::consts::PI * frequency_hz,
                    displacements_mm: Vec::new(),
                    rotations_rad: Vec::new(),
                    relative_residual: 1.0e-12,
                })
                .collect(),
        }
    }

    /// R1120ac-FEA-900: a beam of 40 x 2 x 4 linear tetrahedra cells gave
    /// 59.10 Hz where the finer mesh gives 41.78; the frequencies are reported
    /// against the level before and are not converged.
    #[test]
    fn modes_are_judged_against_the_level_before() {
        let coarse = modal_summary(&found(&[59.10, 120.0]), &found(&[41.78, 118.0]), 0.05);
        assert_eq!(coarse.previous_frequencies_hz, [59.10, 120.0]);
        assert!(
            (coarse.maximum_relative_frequency_change - (59.10 - 41.78) / 59.10).abs() <= 1.0e-12,
            "{coarse:?}"
        );
        assert!(!coarse.converged);
        assert_eq!(coarse.not_checked, FEA_MODES_NOT_CHECKED);

        let settled = modal_summary(&found(&[41.90, 118.2]), &found(&[41.78, 118.0]), 0.05);
        assert!(settled.converged, "{settled:?}");
        // The level before found fewer modes: not converged.
        let fewer = modal_summary(&found(&[41.90]), &found(&[41.78, 118.0]), 0.05);
        assert!(!fewer.converged, "{fewer:?}");
    }

    /// The solver no longer stops at 4 096 elements; the mesher's own limit
    /// is the bound.
    #[test]
    fn refinement_levels_reach_the_mesher_limit_and_no_further() {
        assert_eq!(
            validate_refinement_levels(&[level(0.3, 8_192), level(0.1, MAX_FEA_MESH_TETRAHEDRA)]),
            Ok(())
        );
        assert_eq!(
            validate_refinement_levels(&[
                level(0.3, 8_192),
                level(0.1, MAX_FEA_MESH_TETRAHEDRA + 1)
            ]),
            Err(FeaReviewError::InvalidRefinementLevels)
        );
    }

    /// The user reads what to change, not the Rust name of the error.
    #[test]
    fn refusals_read_as_advice_not_rust_names() {
        let singular = FeaReviewError::Solve(FeaError::SingularSystem).to_string();
        assert!(!singular.contains("SingularSystem"), "{singular}");
        assert!(singular.contains("add a support") && singular.contains("orders of magnitude"));
        let joint =
            FeaReviewError::Setup(ExactFeaSetupError::InvalidJoint { joint: 0 }).to_string();
        assert!(
            joint.contains("joint 1") && !joint.contains("InvalidJoint"),
            "{joint}"
        );
        let mesh = FeaReviewError::MeshNotRefined.to_string();
        assert!(!mesh.contains("MeshNotRefined"), "{mesh}");
    }
}
