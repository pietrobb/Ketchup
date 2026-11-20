use super::{Error, Result};
use ketchup_analysis::fea_thermal::ThermalSolveSettings;
use ketchup_application::thermal_workflow::{
    ExactThermalFaceConvection, ExactThermalFaceFlux, ExactThermalFaceTemperature,
    ExactVolumeMeshWireOptions, ThermalReviewError, ThermalReviewSummary, ThermalStudyRequest,
};
use ketchup_model::document::{DefinitionId, FeatureId, InstancePath, OccurrenceId};
use ketchup_model::tolerance::limits::NAME_BYTES;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The parameters `thermal_review` reads; the face lists and the solve
/// settings may be left out.
pub(super) const FIELDS: &[&str] = &[
    "definition_id",
    "feature_id",
    "occurrence_id",
    "case_id",
    "held_faces",
    "flux_faces",
    "convection_faces",
    "mesh_levels",
    "solve_settings",
    "convergence_relative_tolerance",
    "confirmed",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThermalReviewInput {
    definition_id: u64,
    feature_id: u64,
    occurrence_id: u64,
    case_id: String,
    #[serde(default)]
    held_faces: Vec<HeldFaceInput>,
    #[serde(default)]
    flux_faces: Vec<FluxFaceInput>,
    #[serde(default)]
    convection_faces: Vec<ConvectionFaceInput>,
    mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    #[serde(default)]
    solve_settings: Option<ThermalSolveSettingsInput>,
    convergence_relative_tolerance: f64,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeldFaceInput {
    face_ordinal: u32,
    temperature_c: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FluxFaceInput {
    face_ordinal: u32,
    flux_w_per_m2: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvectionFaceInput {
    face_ordinal: u32,
    film_coefficient_w_per_m2_k: f64,
    ambient_c: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThermalSolveSettingsInput {
    maximum_nodes: usize,
    relative_pivot_tolerance: f64,
}

impl From<ThermalSolveSettingsInput> for ThermalSolveSettings {
    /// The factored matrix keeps the default memory budget.
    fn from(input: ThermalSolveSettingsInput) -> Self {
        Self {
            maximum_nodes: input.maximum_nodes,
            relative_pivot_tolerance: input.relative_pivot_tolerance,
            ..Self::default()
        }
    }
}

impl From<ThermalReviewError> for Error {
    fn from(error: ThermalReviewError) -> Self {
        let code = match &error {
            ThermalReviewError::ConfirmationRequired => "confirmation_required",
            ThermalReviewError::WorkerUnavailable => "exact_worker_unavailable",
            ThermalReviewError::InvalidRefinementLevels | ThermalReviewError::InvalidTolerance => {
                "thermal_refinement_invalid"
            }
            ThermalReviewError::InvalidTarget(..) | ThermalReviewError::Classification(..) => {
                "thermal_target_rejected"
            }
            ThermalReviewError::NoConductivity { .. } => "thermal_material_unknown",
            ThermalReviewError::Meshing(..) => "thermal_meshing_rejected",
            ThermalReviewError::Setup(..) => "thermal_setup_rejected",
            ThermalReviewError::MeshNotRefined | ThermalReviewError::Solve(..) => {
                "thermal_solve_rejected"
            }
        };
        Self::caused_by(code, &error)
    }
}

pub(super) fn thermal_review_request(
    p: &Map<String, Value>,
) -> Result<(ThermalStudyRequest, bool)> {
    let payload: Map<String, Value> = p
        .iter()
        .filter(|(key, _)| FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let input: ThermalReviewInput = serde_json::from_value(Value::Object(payload))
        .map_err(|error| Error::invalid_because(&error))?;
    if input.definition_id == 0
        || input.feature_id == 0
        || input.occurrence_id == 0
        || input.case_id.trim().is_empty()
        || input.case_id.len() > NAME_BYTES
    {
        return Err(Error::invalid(
            "thermal target and case IDs must be bounded and nonzero",
        ));
    }
    Ok((
        ThermalStudyRequest {
            definition_id: DefinitionId(input.definition_id),
            feature_id: FeatureId(input.feature_id),
            case_id: input.case_id,
            instance_path: InstancePath::root(OccurrenceId(input.occurrence_id)),
            held_faces: input
                .held_faces
                .into_iter()
                .map(|face| ExactThermalFaceTemperature {
                    face_ordinal: face.face_ordinal,
                    temperature_c: face.temperature_c,
                })
                .collect(),
            flux_faces: input
                .flux_faces
                .into_iter()
                .map(|face| ExactThermalFaceFlux {
                    face_ordinal: face.face_ordinal,
                    flux_w_per_m2: face.flux_w_per_m2,
                })
                .collect(),
            convection_faces: input
                .convection_faces
                .into_iter()
                .map(|face| ExactThermalFaceConvection {
                    face_ordinal: face.face_ordinal,
                    film_coefficient_w_per_m2_k: face.film_coefficient_w_per_m2_k,
                    ambient_c: face.ambient_c,
                })
                .collect(),
            mesh_levels: input.mesh_levels,
            solve_settings: match input.solve_settings {
                Some(settings) => settings.into(),
                None => ThermalSolveSettings::default(),
            },
            convergence_relative_tolerance: input.convergence_relative_tolerance,
        },
        input.confirmed,
    ))
}

pub(super) fn thermal_review_value(review: &ThermalReviewSummary) -> Value {
    json!({
        "review_digest":review.review_digest,
        "source":{"document_id":review.document_id,"revision":review.revision,"canonical_digest":review.canonical_digest},
        "graph_digest":review.graph_digest,
        "case_id":review.case_id,
        "units":review.units,
        "solver_version":review.solver_version,
        "material":{
            "material":review.material.material,
            "conductivity_w_per_m_k":review.material.conductivity_w_per_m_k,
            "source":review.material.citation,
        },
        "levels":review.levels.iter().map(|level| json!({
            "node_count":level.node_count,
            "element_count":level.element_count,
            "mesh_fingerprint":level.mesh_fingerprint,
            "relative_volume_error":level.relative_volume_error,
            "minimum_quality":level.minimum_quality,
            "minimum_temperature_c":level.minimum_temperature_c,
            "maximum_temperature_c":level.maximum_temperature_c,
            "heat_into_held_faces_w":level.heat_into_held_faces_w,
            "heat_out_of_held_faces_w":level.heat_out_of_held_faces_w,
            "flux_heat_in_w":level.flux_heat_in_w,
            "convection_heat_in_w":level.convection_heat_in_w,
            "heat_balance_w":level.heat_balance_w,
            "maximum_free_node_residual_w":level.maximum_free_node_residual_w,
        })).collect::<Vec<_>>(),
        "convergence":{
            "final_temperature_relative_change":review.final_temperature_relative_change,
            "final_heat_flow_relative_change":review.final_heat_flow_relative_change,
            "required_relative_tolerance":review.required_relative_tolerance,
            "converged":review.converged,
        },
        "excluded_physics":review.excluded_physics,
        "document_mutated":false,
    })
}
