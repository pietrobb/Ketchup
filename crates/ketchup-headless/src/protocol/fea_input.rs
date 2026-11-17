use super::{Error, Result};
use ketchup_analysis::fea::{FeaMaterial, FeaSolveSettings};
use ketchup_application::fea_workflow::{
    ExactFeaFaceTraction, ExactFeaSetup, ExactVolumeMeshWireOptions, FeaReviewError,
    FeaReviewSummary, FeaStudyRequest,
};
use ketchup_model::document::{DefinitionId, FeatureId, InstancePath, OccurrenceId};
use ketchup_model::tolerance::limits::NAME_BYTES;
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeaSolveSettingsInput {
    maximum_nodes: usize,
    relative_pivot_tolerance: f64,
    maximum_small_deformation_ratio: f64,
    convergence_relative_tolerance: f64,
}

impl From<FeaSolveSettingsInput> for FeaSolveSettings {
    /// The factored matrix keeps the default memory budget.
    fn from(input: FeaSolveSettingsInput) -> Self {
        Self {
            maximum_nodes: input.maximum_nodes,
            relative_pivot_tolerance: input.relative_pivot_tolerance,
            maximum_small_deformation_ratio: input.maximum_small_deformation_ratio,
            convergence_relative_tolerance: input.convergence_relative_tolerance,
            ..Self::default()
        }
    }
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

impl From<FeaReviewError> for Error {
    fn from(error: FeaReviewError) -> Self {
        let code = match &error {
            FeaReviewError::ConfirmationRequired => "confirmation_required",
            FeaReviewError::WorkerUnavailable => "exact_worker_unavailable",
            FeaReviewError::InvalidRefinementLevels => "fea_refinement_invalid",
            FeaReviewError::InvalidTarget(..) => "fea_target_rejected",
            FeaReviewError::Meshing(..) => "fea_meshing_rejected",
            FeaReviewError::Setup(..) => "fea_setup_rejected",
            FeaReviewError::MeshNotRefined | FeaReviewError::Solve(..) => "fea_solve_rejected",
        };
        Self::caused_by(code, &error)
    }
}

pub(super) fn fea_review_request(p: &Map<String, Value>) -> Result<(FeaStudyRequest, bool)> {
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
        .map_err(|error| Error::invalid_because(&error))?;
    if input.definition_id == 0
        || input.feature_id == 0
        || input.occurrence_id == 0
        || input.material.id == 0
        || input.case_id.trim().is_empty()
        || input.case_id.len() > NAME_BYTES
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
                    density_kg_per_m3: None,
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
            solve_settings: input.solve_settings.into(),
        },
        input.confirmed,
    ))
}

pub(super) fn fea_review_value(review: &FeaReviewSummary) -> Value {
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
