//! Validation rules as data.
//!
//! The validators in [`crate::validation`] compute physics and geometry; the
//! numbers they compare against (material stiffness, design loads, passage
//! widths, hole material, ...) are norms that differ by country, product and
//! material. They come from the program library's `validation_rules.json`
//! and travel into every report, so a result always names the rule values it
//! was judged against.

use ketchup_model::document::{ClassificationError, OccurrenceId, Snapshot};
use ketchup_model::validation::MATERIAL_DIMENSION_V1;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialProperties {
    pub elastic_modulus_n_mm2: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeamDeflectionRule {
    pub design_load_n: f64,
    pub span_ratio: f64,
    pub maximum_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TippingRule {
    pub minimum_tip_angle_degrees: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoringRule {
    pub minimum_height_mm: f64,
    pub minimum_height_depth_ratio: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HardwareManufacturingRule {
    pub minimum_hole_edge_material_mm: f64,
    pub minimum_hole_spacing_material_mm: f64,
    pub minimum_cup_diameter_mm: f64,
    pub minimum_cup_depth_mm: f64,
    pub maximum_slide_pair_length_mismatch_mm: f64,
    pub maximum_slide_pair_vertical_mismatch_mm: f64,
    pub minimum_panel_thickness_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomPlacementRule {
    pub boundary_tolerance_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassageClearanceRule {
    pub minimum_width_mm: f64,
    pub minimum_headroom_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRules {
    pub materials: BTreeMap<String, MaterialProperties>,
    /// Material of a part that no `ketchup.material.v1` classification names.
    pub default_material: String,
    pub beam_deflection: BeamDeflectionRule,
    pub tipping: TippingRule,
    pub anchoring: AnchoringRule,
    pub hardware_manufacturing: HardwareManufacturingRule,
    pub room_placement: RoomPlacementRule,
    pub passage_clearance: PassageClearanceRule,
}

impl ValidationRules {
    /// Parses and checks a rule set: every number is finite and positive and
    /// the default material has properties.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let rules: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let value = serde_json::to_value(&rules).map_err(|error| error.to_string())?;
        check_positive("", &value)?;
        if !rules.materials.contains_key(&rules.default_material) {
            return Err(format!(
                "default_material {:?} has no entry in materials",
                rules.default_material
            ));
        }
        Ok(rules)
    }

    /// The rule set shipped with the program library.
    #[must_use]
    pub fn library() -> &'static Self {
        static RULES: OnceLock<ValidationRules> = OnceLock::new();
        RULES.get_or_init(|| {
            Self::from_json(ketchup_program::library_validation_rules())
                .expect("the library validation rules are valid")
        })
    }
}

fn check_positive(path: &str, value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Object(fields) => fields.iter().try_for_each(|(key, field)| {
            check_positive(
                &format!("{path}{}{key}", if path.is_empty() { "" } else { "." }),
                field,
            )
        }),
        serde_json::Value::Number(number) => match number.as_f64() {
            Some(number) if number.is_finite() && number > 0.0 => Ok(()),
            _ => Err(format!("{path} must be a finite positive number")),
        },
        _ => Ok(()),
    }
}

/// The material each occurrence declares through the `ketchup.material.v1`
/// classification dimension. A document without that dimension declares
/// none, so every part takes the rules' default material.
pub fn occurrence_materials(
    snapshot: &Snapshot,
) -> Result<BTreeMap<OccurrenceId, String>, ClassificationError> {
    let Some(dimension) = snapshot.classification_dimension_named(MATERIAL_DIMENSION_V1)? else {
        return Ok(BTreeMap::new());
    };
    snapshot
        .occurrence_category_names(dimension)
        .map(|assigned| assigned.map(|(occurrence_id, _, name)| (occurrence_id, name.to_owned())))
        .collect()
}
