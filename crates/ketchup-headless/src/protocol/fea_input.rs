//! The `fea_review` and `fea_assembly_review` requests and their answer.

use super::{Error, Result};
use ketchup_analysis::fea::{FeaCouplingKind, FeaMaterial, FeaSolveSettings};
use ketchup_application::fea_workflow::{
    ExactFeaAssemblyPart, ExactFeaAssemblySetup, ExactFeaFaceTraction, ExactFeaJoint,
    ExactFeaJointFace, ExactFeaMateJoint, ExactFeaSetup, ExactVolumeMeshWireOptions,
    FeaAssemblyStudyRequest, FeaReviewSummary, FeaStudyRequest,
};
use ketchup_model::assembly::AssemblyMateId;
use ketchup_model::document::{DefinitionId, FeatureId, InstancePath, OccurrenceId};
use ketchup_model::tolerance::limits;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

pub(super) const REVIEW_FIELDS: &[&str] = &[
    "definition_id",
    "feature_id",
    "occurrence_id",
    "case_id",
    "material",
    "constrained_face_ordinals",
    "face_tractions",
    "mesh_levels",
    "solve_settings",
    "mode_count",
    "confirmed",
];
pub(super) const ASSEMBLY_REVIEW_FIELDS: &[&str] = &[
    "case_id",
    "parts",
    "joints",
    "mate_joints",
    "mesh_levels",
    "solve_settings",
    "mode_count",
    "confirmed",
];
/// Fields a request may leave out.
const OPTIONAL_FIELDS: &[&str] = &["mode_count", "mate_joints"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaMaterialInput {
    id: u64,
    youngs_modulus_mpa: f64,
    poisson_ratio: f64,
    yield_strength_mpa: Option<f64>,
    /// Needed only to find natural frequencies.
    #[serde(default)]
    density_kg_per_m3: Option<f64>,
}

impl From<FeaMaterialInput> for FeaMaterial {
    fn from(input: FeaMaterialInput) -> Self {
        Self {
            id: input.id,
            youngs_modulus_mpa: input.youngs_modulus_mpa,
            poisson_ratio: input.poisson_ratio,
            yield_strength_mpa: input.yield_strength_mpa,
            density_kg_per_m3: input.density_kg_per_m3,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaSolveSettingsInput {
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
struct FeaFaceTractionInput {
    face_ordinal: u32,
    traction_local_n_per_mm2: [f64; 3],
}

impl From<FeaFaceTractionInput> for ExactFeaFaceTraction {
    fn from(input: FeaFaceTractionInput) -> Self {
        Self {
            face_ordinal: input.face_ordinal,
            traction_local_n_per_mm2: input.traction_local_n_per_mm2,
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
    #[serde(default)]
    mode_count: usize,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaAssemblyPartInput {
    occurrence_id: u64,
    /// The solid feature of the occurrence's definition.
    feature_id: u64,
    material: FeaMaterialInput,
    constrained_face_ordinals: Vec<u32>,
    face_tractions: Vec<FeaFaceTractionInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaJointFaceInput {
    part: usize,
    face_ordinal: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum FeaJointKindInput {
    Rigid,
    Spring {
        translation_n_per_mm: [f64; 3],
        rotation_n_mm_per_rad: [f64; 3],
    },
}

impl From<FeaJointKindInput> for FeaCouplingKind {
    fn from(input: FeaJointKindInput) -> Self {
        match input {
            FeaJointKindInput::Rigid => Self::Rigid,
            FeaJointKindInput::Spring {
                translation_n_per_mm,
                rotation_n_mm_per_rad,
            } => Self::Spring {
                translation_n_per_mm,
                rotation_n_mm_per_rad,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaJointInput {
    faces: [FeaJointFaceInput; 2],
    kind: FeaJointKindInput,
}

/// A face-to-face mate of the document joined at the faces it names.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaMateJointInput {
    mate_id: u64,
    kind: FeaJointKindInput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaAssemblyReviewInput {
    case_id: String,
    parts: Vec<FeaAssemblyPartInput>,
    joints: Vec<FeaJointInput>,
    #[serde(default)]
    mate_joints: Vec<FeaMateJointInput>,
    mesh_levels: Vec<ExactVolumeMeshWireOptions>,
    solve_settings: FeaSolveSettingsInput,
    #[serde(default)]
    mode_count: usize,
    confirmed: bool,
}

/// Reads `fields` of `p`, every one required but those that may be left out.
fn input<T: DeserializeOwned>(p: &Map<String, Value>, fields: &[&str]) -> Result<T> {
    let mut payload = Map::new();
    for field in fields {
        match p.get(*field) {
            Some(value) => {
                payload.insert((*field).to_owned(), value.clone());
            }
            None if OPTIONAL_FIELDS.contains(field) => {}
            None => return Err(Error::invalid(format!("missing {field}"))),
        }
    }
    serde_json::from_value(Value::Object(payload)).map_err(|error| Error::invalid_because(&error))
}

fn bounded_case(case_id: &str) -> bool {
    !case_id.trim().is_empty() && case_id.len() <= limits::NAME_BYTES
}

pub(super) fn review_request(p: &Map<String, Value>) -> Result<(FeaStudyRequest, bool)> {
    let input: FeaReviewInput = input(p, REVIEW_FIELDS)?;
    if input.definition_id == 0
        || input.feature_id == 0
        || input.occurrence_id == 0
        || input.material.id == 0
        || !bounded_case(&input.case_id)
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
                material: input.material.into(),
                constrained_face_ordinals: input.constrained_face_ordinals,
                face_tractions: input.face_tractions.into_iter().map(Into::into).collect(),
            },
            mesh_levels: input.mesh_levels,
            solve_settings: input.solve_settings.into(),
            mode_count: input.mode_count,
        },
        input.confirmed,
    ))
}

pub(super) fn assembly_review_request(
    p: &Map<String, Value>,
) -> Result<(FeaAssemblyStudyRequest, bool)> {
    let input: FeaAssemblyReviewInput = input(p, ASSEMBLY_REVIEW_FIELDS)?;
    if !bounded_case(&input.case_id)
        || input
            .parts
            .iter()
            .any(|part| part.occurrence_id == 0 || part.feature_id == 0 || part.material.id == 0)
    {
        return Err(Error::invalid(
            "FEA part, material and case IDs must be bounded and nonzero",
        ));
    }
    let part_features = input
        .parts
        .iter()
        .map(|part| FeatureId(part.feature_id))
        .collect();
    let parts = input
        .parts
        .into_iter()
        .map(|part| ExactFeaAssemblyPart {
            instance_path: InstancePath::root(OccurrenceId(part.occurrence_id)),
            material: part.material.into(),
            constrained_face_ordinals: part.constrained_face_ordinals,
            face_tractions: part.face_tractions.into_iter().map(Into::into).collect(),
        })
        .collect();
    let joints = input
        .joints
        .into_iter()
        .map(|joint| ExactFeaJoint {
            faces: joint.faces.map(|face| ExactFeaJointFace {
                part: face.part,
                face_ordinal: face.face_ordinal,
            }),
            kind: joint.kind.into(),
        })
        .collect();
    let mate_joints = input
        .mate_joints
        .into_iter()
        .map(|joint| ExactFeaMateJoint {
            mate_id: AssemblyMateId(joint.mate_id),
            kind: joint.kind.into(),
        })
        .collect();
    Ok((
        FeaAssemblyStudyRequest {
            part_features,
            setup: ExactFeaAssemblySetup {
                case_id: input.case_id,
                parts,
                joints,
                mate_joints,
            },
            mesh_levels: input.mesh_levels,
            solve_settings: input.solve_settings.into(),
            mode_count: input.mode_count,
        },
        input.confirmed,
    ))
}

pub(super) fn review_value(review: &FeaReviewSummary) -> Value {
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
        "modes":review.modes.as_ref().map(|modes| json!({
            "solver_version":modes.solver_version,
            "total_mass_kg":modes.total_mass_kg,
            "frequencies_hz":modes.frequencies_hz,
            "maximum_relative_residual":modes.maximum_relative_residual,
        })),
        "document_mutated":false,
    })
}
