use crate::exact_revolve::{
    controlled_bottle_profile, finish_amount_is_conservative, inner_shell_profile,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const ASSISTANT_PROTOCOL_VERSION: u16 = 3;
const MAX_ASSISTANT_MODEL_BYTES: usize = 128;
const MAX_ASSISTANT_BOXES: usize = 64;
const MAX_ASSISTANT_SUBTRACTIONS: usize = 64;
const MAX_ASSISTANT_TRANSLATIONS: usize = 100;
const MAX_ASSISTANT_ROTATIONS: usize = 100;
const MAX_ASSISTANT_PROFILE_TRANSLATIONS: usize = 1;
const MAX_ASSISTANT_ARRAYS: usize = 16;
const MAX_ASSISTANT_ARRAY_SOURCES: usize = 100;
const MAX_ASSISTANT_ARRAY_INSTANCES: u32 = 1_000;
const MAX_ASSISTANT_ARRAY_OUTPUTS: usize = 512;
const MAX_ASSISTANT_CAD_EDIT_OPERATIONS: usize = 64;
const MAX_ASSISTANT_CAD_SELECTOR_TARGETS: usize = 100;
const MAX_ASSISTANT_CAD_GENERATED_OCCURRENCES: usize = 512;
const MAX_ASSISTANT_BOTTLES: usize = 8;
const MAX_ASSISTANT_TEAPOT_DIMENSION_MM: f64 = 2_000.0;
const MAX_ASSISTANT_BALLOON_TEXTS: usize = 8;
const MAX_ASSISTANT_BALLOON_TEXT_CHARS: usize = 32;
const MAX_ASSISTANT_GABLE_ROOFS: usize = 16;
const MAX_ASSISTANT_STAIRCASES: usize = 16;
const MAX_ASSISTANT_ORIENTED_BEAMS: usize = 64;
const MAX_ASSISTANT_BEAM_NOTCHES: usize = 64;
const MAX_ASSISTANT_NAME_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_CODE_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_OPERATION_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_TARGET_BYTES: usize = 256;
const MAX_ASSISTANT_REJECTION_TEXT_BYTES: usize = 2_048;
const MAX_ASSISTANT_REJECTION_BYTES: usize = 8 * 1_024;
const MAX_ASSISTANT_ABS_MM: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantDistribution {
    PublicApi,
    PrivateOauth,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCapability {
    Chat,
    DebugObservability,
    LocalMemory,
    QueryDocument,
    ProposeWorkflowIntent,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSubtractionIntent {
    pub size_mm: [f64; 3],
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBoxIntent {
    pub name: String,
    pub size_mm: [f64; 3],
    pub origin_mm: [f64; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtract_boxes: Vec<AssistantSubtractionIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTranslationIntent {
    pub occurrence_id: u64,
    pub delta_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRotationIntent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u64>,
    pub pivot_mm: [f64; 3],
    pub axis: [f64; 3],
    pub angle_degrees: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantProfileTranslationIntent {
    pub definition_id: u64,
    pub body_id: u64,
    pub profile_id: u64,
    pub delta_mm: [f64; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantParameterEditIntent {
    pub definition_id: u64,
    pub body_id: u64,
    pub feature_id: u64,
    pub constraint_id: Option<u64>,
    pub value_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantLinearArrayIntent {
    pub occurrence_ids: Vec<u64>,
    pub instances: u32,
    pub step_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadEditProgram {
    pub operations: Vec<AssistantCadEditOperation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadEntitySelector {
    CurrentSelection {},
    Occurrences { occurrence_ids: Vec<u64> },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadDeletePolicy {
    RejectIfReferenced,
    RemoveReferences,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadRotation {
    pub pivot_mm: [f64; 3],
    pub axis: [f64; 3],
    pub angle_degrees: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantPrincipalPlane {
    Xy,
    Yz,
    Xz,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantWorkplaneSpec {
    Principal {
        plane: AssistantPrincipalPlane,
    },
    Offset {
        base_feature_id: u64,
        distance_mm: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadPartFeature {
    Extrusion {
        distance_mm: f64,
    },
    Revolve {
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    },
}

impl AssistantCadPartFeature {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Extrusion { distance_mm }
                if distance_mm.is_finite()
                    && *distance_mm > 0.0
                    && *distance_mm <= MAX_ASSISTANT_ABS_MM =>
            {
                Ok(())
            }
            Self::Extrusion { .. } => Err("assistant CAD part feature is invalid".to_owned()),
            Self::Revolve {
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } if axis_start_mm
                .iter()
                .chain(axis_end_mm)
                .all(|value| value.is_finite() && value.abs() <= MAX_ASSISTANT_ABS_MM)
                && (axis_end_mm[0] - axis_start_mm[0]).hypot(axis_end_mm[1] - axis_start_mm[1])
                    > 1.0e-9
                && angle_degrees.is_finite()
                && *angle_degrees > 0.0
                && *angle_degrees <= 360.0 =>
            {
                Ok(())
            }
            Self::Revolve { .. } => Err("assistant CAD part feature is invalid".to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadBooleanOperation {
    Cut,
    Union,
    Intersect,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadLoftSection {
    pub profile_feature_id: u64,
    pub elevation_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadBodyFeature {
    Boolean {
        operation: AssistantCadBooleanOperation,
        target_feature_id: u64,
        tool_feature_id: u64,
    },
    Pocket {
        target_feature_id: u64,
        profile_feature_id: u64,
        depth_mm: f64,
    },
    Sweep {
        profile_feature_id: u64,
        path_feature_id: u64,
    },
    Loft {
        sections: Vec<AssistantCadLoftSection>,
    },
    TopologyShell {
        target_feature_id: u64,
        removed_face_reference_ids: Vec<String>,
        thickness_mm: f64,
    },
    TopologyFillet {
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        radius_mm: f64,
    },
    TopologyChamfer {
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        distance_mm: f64,
    },
}

impl AssistantCadBodyFeature {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Boolean {
                target_feature_id,
                tool_feature_id,
                ..
            } if *target_feature_id != 0
                && *tool_feature_id != 0
                && target_feature_id != tool_feature_id =>
            {
                Ok(())
            }
            Self::Boolean { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::Pocket {
                target_feature_id,
                profile_feature_id,
                depth_mm,
            } if *target_feature_id != 0
                && *profile_feature_id != 0
                && target_feature_id != profile_feature_id
                && depth_mm.is_finite()
                && *depth_mm > 0.0
                && *depth_mm <= MAX_ASSISTANT_ABS_MM =>
            {
                Ok(())
            }
            Self::Pocket { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::Sweep {
                profile_feature_id,
                path_feature_id,
            } if *profile_feature_id != 0
                && *path_feature_id != 0
                && profile_feature_id != path_feature_id =>
            {
                Ok(())
            }
            Self::Sweep { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::Loft { sections }
                if (2..=16).contains(&sections.len())
                    && sections.iter().all(|section| {
                        section.profile_feature_id != 0
                            && section.elevation_mm.is_finite()
                            && section.elevation_mm.abs() <= MAX_ASSISTANT_ABS_MM
                    })
                    && sections
                        .iter()
                        .map(|section| section.profile_feature_id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == sections.len()
                    && sections
                        .windows(2)
                        .all(|pair| pair[0].elevation_mm < pair[1].elevation_mm) =>
            {
                Ok(())
            }
            Self::Loft { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::TopologyShell {
                target_feature_id,
                removed_face_reference_ids,
                thickness_mm,
            } if *target_feature_id != 0
                && (1..=64).contains(&removed_face_reference_ids.len())
                && removed_face_reference_ids.iter().all(|reference_id| {
                    reference_id.len() == 64
                        && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                && removed_face_reference_ids
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == removed_face_reference_ids.len()
                && thickness_mm.is_finite()
                && (0.01..=100_000.0).contains(thickness_mm) =>
            {
                Ok(())
            }
            Self::TopologyShell { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::TopologyFillet {
                target_feature_id,
                edge_reference_ids,
                radius_mm,
            } if *target_feature_id != 0
                && (1..=64).contains(&edge_reference_ids.len())
                && edge_reference_ids.iter().all(|reference_id| {
                    reference_id.len() == 64
                        && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                && edge_reference_ids.iter().collect::<BTreeSet<_>>().len()
                    == edge_reference_ids.len()
                && radius_mm.is_finite()
                && (0.01..=100_000.0).contains(radius_mm) =>
            {
                Ok(())
            }
            Self::TopologyFillet { .. } => Err("assistant CAD body feature is invalid".to_owned()),
            Self::TopologyChamfer {
                target_feature_id,
                edge_reference_ids,
                distance_mm,
            } if *target_feature_id != 0
                && (1..=64).contains(&edge_reference_ids.len())
                && edge_reference_ids.iter().all(|reference_id| {
                    reference_id.len() == 64
                        && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                && edge_reference_ids.iter().collect::<BTreeSet<_>>().len()
                    == edge_reference_ids.len()
                && distance_mm.is_finite()
                && (0.01..=100_000.0).contains(distance_mm) =>
            {
                Ok(())
            }
            Self::TopologyChamfer { .. } => Err("assistant CAD body feature is invalid".to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantSketchPointKind {
    Start,
    End,
    Center,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSketchPointRef {
    pub entity_id: u64,
    pub point: AssistantSketchPointKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantSketchEntity {
    Line {
        id: u64,
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    Arc {
        id: u64,
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    Circle {
        id: u64,
        center_mm: [f64; 2],
        radius_mm: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantSketchConstraint {
    Horizontal {
        id: u64,
        entity_id: u64,
    },
    Vertical {
        id: u64,
        entity_id: u64,
    },
    Coincident {
        id: u64,
        a: AssistantSketchPointRef,
        b: AssistantSketchPointRef,
    },
    Distance {
        id: u64,
        a: AssistantSketchPointRef,
        b: AssistantSketchPointRef,
        value_mm: f64,
    },
    Radius {
        id: u64,
        entity_id: u64,
        value_mm: f64,
    },
    FixedPoint {
        id: u64,
        point: AssistantSketchPointRef,
        position_mm: [f64; 2],
    },
    Parallel {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Perpendicular {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Tangent {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Angle {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
        angle_degrees: f64,
    },
    Equal {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Symmetric {
        id: u64,
        a: AssistantSketchPointRef,
        b: AssistantSketchPointRef,
        axis_entity_id: u64,
    },
    Concentric {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Collinear {
        id: u64,
        a_entity_id: u64,
        b_entity_id: u64,
    },
    Midpoint {
        id: u64,
        point: AssistantSketchPointRef,
        line_entity_id: u64,
    },
    PointOnCurve {
        id: u64,
        point: AssistantSketchPointRef,
        curve_entity_id: u64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadEditOperation {
    CreateSketch {
        definition_id: u64,
        name: String,
        workplane: AssistantWorkplaneSpec,
        entities: Vec<AssistantSketchEntity>,
        constraints: Vec<AssistantSketchConstraint>,
    },
    CreatePart {
        name: String,
        workplane: AssistantWorkplaneSpec,
        entities: Vec<AssistantSketchEntity>,
        constraints: Vec<AssistantSketchConstraint>,
        feature: AssistantCadPartFeature,
        translation_mm: [f64; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<AssistantCadRotation>,
    },
    AppendFeature {
        definition_id: u64,
        name: String,
        feature: AssistantCadBodyFeature,
    },
    SetDimension {
        feature_id: u64,
        constraint_id: Option<u64>,
        value_mm: f64,
    },
    Delete {
        selector: AssistantCadEntitySelector,
        dependency_policy: AssistantCadDeletePolicy,
    },
    Transform {
        selector: AssistantCadEntitySelector,
        translation_mm: [f64; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<AssistantCadRotation>,
    },
    Copy {
        selector: AssistantCadEntitySelector,
        translation_mm: [f64; 3],
    },
    LinearPattern {
        selector: AssistantCadEntitySelector,
        instances: u32,
        step_mm: [f64; 3],
    },
    Mirror {
        selector: AssistantCadEntitySelector,
        plane_origin_mm: [f64; 3],
        plane_normal: [f64; 3],
    },
}

fn assistant_cad_vector_is_bounded(vector: [f64; 3]) -> bool {
    vector
        .iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_ASSISTANT_ABS_MM)
}

fn assistant_cad_vector_is_nonzero(vector: [f64; 3]) -> bool {
    vector.iter().any(|value| value.abs() > f64::EPSILON)
}

impl AssistantCadEntitySelector {
    fn bounded_target_count(&self) -> Result<usize, String> {
        match self {
            Self::CurrentSelection {} => Ok(MAX_ASSISTANT_CAD_SELECTOR_TARGETS),
            Self::Occurrences { occurrence_ids } => {
                let unique = occurrence_ids.iter().copied().collect::<BTreeSet<_>>();
                if occurrence_ids.is_empty()
                    || occurrence_ids.len() > MAX_ASSISTANT_CAD_SELECTOR_TARGETS
                    || unique.len() != occurrence_ids.len()
                    || occurrence_ids.contains(&0)
                {
                    return Err("assistant CAD selector is invalid".to_owned());
                }
                Ok(occurrence_ids.len())
            }
        }
    }

    pub fn validate_resolved_target_count(&self, target_count: usize) -> Result<(), String> {
        self.bounded_target_count()?;
        if target_count == 0 || target_count > MAX_ASSISTANT_CAD_SELECTOR_TARGETS {
            return Err("assistant CAD resolved selector target count is invalid".to_owned());
        }
        Ok(())
    }
}

impl AssistantCadRotation {
    fn validate(&self) -> Result<(), String> {
        let axis_length_squared = self.axis.iter().map(|value| value * value).sum::<f64>();
        let normalized_angle = self.angle_degrees.rem_euclid(360.0);
        let shortest_angle = normalized_angle.min(360.0 - normalized_angle);
        if !assistant_cad_vector_is_bounded(self.pivot_mm)
            || !assistant_cad_vector_is_bounded(self.axis)
            || !axis_length_squared.is_finite()
            || axis_length_squared <= f64::EPSILON
            || !self.angle_degrees.is_finite()
            || self.angle_degrees.abs() > MAX_ASSISTANT_ABS_MM
            || shortest_angle < 0.01
        {
            return Err("assistant CAD rotation is invalid".to_owned());
        }
        Ok(())
    }
}

impl AssistantWorkplaneSpec {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Principal { .. } => Ok(()),
            Self::Offset {
                base_feature_id,
                distance_mm,
            } if *base_feature_id != 0
                && distance_mm.is_finite()
                && distance_mm.abs() <= MAX_ASSISTANT_ABS_MM =>
            {
                Ok(())
            }
            Self::Offset { .. } => Err("assistant workplane is invalid".to_owned()),
        }
    }
}

impl AssistantSketchEntity {
    fn id(&self) -> u64 {
        match self {
            Self::Line { id, .. } | Self::Arc { id, .. } | Self::Circle { id, .. } => *id,
        }
    }

    fn supports_point(&self, point: AssistantSketchPointKind) -> bool {
        matches!(
            (self, point),
            (
                Self::Line { .. } | Self::Arc { .. },
                AssistantSketchPointKind::Start | AssistantSketchPointKind::End
            ) | (
                Self::Arc { .. } | Self::Circle { .. },
                AssistantSketchPointKind::Center
            )
        )
    }

    fn is_line(&self) -> bool {
        matches!(self, Self::Line { .. })
    }

    fn is_circular(&self) -> bool {
        matches!(self, Self::Arc { .. } | Self::Circle { .. })
    }

    fn validate(&self) -> Result<(), String> {
        let point = |point: &[f64; 2]| {
            point
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_ASSISTANT_ABS_MM)
        };
        let valid = match self {
            Self::Line {
                start_mm, end_mm, ..
            } => point(start_mm) && point(end_mm),
            Self::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => point(start_mm) && point(end_mm) && point(center_mm),
            Self::Circle {
                center_mm,
                radius_mm,
                ..
            } => {
                point(center_mm)
                    && radius_mm.is_finite()
                    && *radius_mm > 0.0
                    && *radius_mm <= MAX_ASSISTANT_ABS_MM
            }
        };
        if self.id() == 0 || !valid {
            return Err("assistant sketch entity is invalid".to_owned());
        }
        Ok(())
    }
}

impl AssistantSketchConstraint {
    fn id(&self) -> u64 {
        match self {
            Self::Horizontal { id, .. }
            | Self::Vertical { id, .. }
            | Self::Coincident { id, .. }
            | Self::Distance { id, .. }
            | Self::Radius { id, .. }
            | Self::FixedPoint { id, .. }
            | Self::Parallel { id, .. }
            | Self::Perpendicular { id, .. }
            | Self::Tangent { id, .. }
            | Self::Angle { id, .. }
            | Self::Equal { id, .. }
            | Self::Symmetric { id, .. }
            | Self::Concentric { id, .. }
            | Self::Collinear { id, .. }
            | Self::Midpoint { id, .. }
            | Self::PointOnCurve { id, .. } => *id,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let valid_point_ref = |point: &AssistantSketchPointRef| point.entity_id != 0;
        let valid_point = |point: &[f64; 2]| {
            point
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_ASSISTANT_ABS_MM)
        };
        let valid = match self {
            Self::Horizontal { entity_id, .. } | Self::Vertical { entity_id, .. } => {
                *entity_id != 0
            }
            Self::Coincident { a, b, .. } => valid_point_ref(a) && valid_point_ref(b),
            Self::Distance { a, b, value_mm, .. } => {
                valid_point_ref(a)
                    && valid_point_ref(b)
                    && value_mm.is_finite()
                    && *value_mm > 0.0
                    && *value_mm <= MAX_ASSISTANT_ABS_MM
            }
            Self::Radius {
                entity_id,
                value_mm,
                ..
            } => {
                *entity_id != 0
                    && value_mm.is_finite()
                    && *value_mm > 0.0
                    && *value_mm <= MAX_ASSISTANT_ABS_MM
            }
            Self::FixedPoint {
                point, position_mm, ..
            } => valid_point_ref(point) && valid_point(position_mm),
            Self::Parallel {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Perpendicular {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Tangent {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Equal {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Concentric {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Collinear {
                a_entity_id,
                b_entity_id,
                ..
            } => *a_entity_id != 0 && *b_entity_id != 0 && a_entity_id != b_entity_id,
            Self::Angle {
                a_entity_id,
                b_entity_id,
                angle_degrees,
                ..
            } => {
                *a_entity_id != 0
                    && *b_entity_id != 0
                    && a_entity_id != b_entity_id
                    && angle_degrees.is_finite()
                    && *angle_degrees > 0.0
                    && *angle_degrees < 180.0
            }
            Self::Symmetric {
                a,
                b,
                axis_entity_id,
                ..
            } => valid_point_ref(a) && valid_point_ref(b) && *a != *b && *axis_entity_id != 0,
            Self::Midpoint {
                point,
                line_entity_id,
                ..
            } => valid_point_ref(point) && *line_entity_id != 0,
            Self::PointOnCurve {
                point,
                curve_entity_id,
                ..
            } => valid_point_ref(point) && *curve_entity_id != 0,
        };
        if self.id() == 0 || !valid {
            return Err("assistant sketch constraint is invalid".to_owned());
        }
        Ok(())
    }

    fn validate_references(
        &self,
        entities: &BTreeMap<u64, &AssistantSketchEntity>,
    ) -> Result<(), String> {
        let entity = |id: u64| entities.get(&id).copied();
        let point = |reference: &AssistantSketchPointRef| {
            entity(reference.entity_id).is_some_and(|entity| entity.supports_point(reference.point))
        };
        let line_pair = |a: u64, b: u64| {
            entity(a).is_some_and(AssistantSketchEntity::is_line)
                && entity(b).is_some_and(AssistantSketchEntity::is_line)
        };
        let circular_pair = |a: u64, b: u64| {
            entity(a).is_some_and(AssistantSketchEntity::is_circular)
                && entity(b).is_some_and(AssistantSketchEntity::is_circular)
        };
        let valid = match self {
            Self::Horizontal { entity_id, .. } | Self::Vertical { entity_id, .. } => {
                entity(*entity_id).is_some_and(AssistantSketchEntity::is_line)
            }
            Self::Coincident { a, b, .. } | Self::Distance { a, b, .. } => point(a) && point(b),
            Self::Radius { entity_id, .. } => {
                entity(*entity_id).is_some_and(AssistantSketchEntity::is_circular)
            }
            Self::FixedPoint {
                point: reference, ..
            } => point(reference),
            Self::Parallel {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Perpendicular {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Angle {
                a_entity_id,
                b_entity_id,
                ..
            }
            | Self::Collinear {
                a_entity_id,
                b_entity_id,
                ..
            } => line_pair(*a_entity_id, *b_entity_id),
            Self::Tangent {
                a_entity_id,
                b_entity_id,
                ..
            } => {
                let a = entity(*a_entity_id);
                let b = entity(*b_entity_id);
                a.is_some()
                    && b.is_some()
                    && (a.is_some_and(AssistantSketchEntity::is_circular)
                        || b.is_some_and(AssistantSketchEntity::is_circular))
            }
            Self::Equal {
                a_entity_id,
                b_entity_id,
                ..
            } => line_pair(*a_entity_id, *b_entity_id) || circular_pair(*a_entity_id, *b_entity_id),
            Self::Symmetric {
                a,
                b,
                axis_entity_id,
                ..
            } => {
                point(a)
                    && point(b)
                    && entity(*axis_entity_id).is_some_and(AssistantSketchEntity::is_line)
            }
            Self::Concentric {
                a_entity_id,
                b_entity_id,
                ..
            } => circular_pair(*a_entity_id, *b_entity_id),
            Self::Midpoint {
                point: reference,
                line_entity_id,
                ..
            } => {
                point(reference)
                    && reference.entity_id != *line_entity_id
                    && entity(*line_entity_id).is_some_and(AssistantSketchEntity::is_line)
            }
            Self::PointOnCurve {
                point: reference,
                curve_entity_id,
                ..
            } => {
                point(reference)
                    && reference.entity_id != *curve_entity_id
                    && entity(*curve_entity_id).is_some()
            }
        };
        if valid {
            Ok(())
        } else {
            Err("assistant sketch constraint reference is invalid".to_owned())
        }
    }
}

fn validate_assistant_sketch_payload(
    name: &str,
    workplane: &AssistantWorkplaneSpec,
    entities: &[AssistantSketchEntity],
    constraints: &[AssistantSketchConstraint],
) -> Result<(), String> {
    if name.trim().is_empty()
        || name.len() > MAX_ASSISTANT_NAME_BYTES
        || name.chars().any(char::is_control)
        || entities.is_empty()
        || entities.len() > crate::sketch::MAX_SKETCH_ENTITIES
        || constraints.len() > crate::sketch::MAX_SKETCH_CONSTRAINTS
    {
        return Err("assistant sketch creation is invalid".to_owned());
    }
    workplane.validate()?;
    let mut entities_by_id = BTreeMap::new();
    for entity in entities {
        entity.validate()?;
        if entities_by_id.insert(entity.id(), entity).is_some() {
            return Err("assistant sketch entity IDs are invalid".to_owned());
        }
    }
    let mut constraint_ids = BTreeSet::new();
    for constraint in constraints {
        constraint.validate()?;
        constraint.validate_references(&entities_by_id)?;
        if !constraint_ids.insert(constraint.id()) {
            return Err("assistant sketch constraint IDs are invalid".to_owned());
        }
    }
    Ok(())
}

impl AssistantCadEditProgram {
    pub fn validate(&self) -> Result<(), String> {
        if self.operations.is_empty() || self.operations.len() > MAX_ASSISTANT_CAD_EDIT_OPERATIONS {
            return Err("assistant CAD edit program operation count is invalid".to_owned());
        }
        let mut generated_occurrences = 0usize;
        for operation in &self.operations {
            let bounded_targets = match operation {
                AssistantCadEditOperation::CreateSketch { .. }
                | AssistantCadEditOperation::AppendFeature { .. }
                | AssistantCadEditOperation::SetDimension { .. } => 0,
                AssistantCadEditOperation::CreatePart { .. } => 1,
                AssistantCadEditOperation::Delete { selector, .. }
                | AssistantCadEditOperation::Transform { selector, .. }
                | AssistantCadEditOperation::Copy { selector, .. }
                | AssistantCadEditOperation::LinearPattern { selector, .. }
                | AssistantCadEditOperation::Mirror { selector, .. } => {
                    selector.bounded_target_count()?
                }
            };
            let generated_per_target = match operation {
                AssistantCadEditOperation::CreateSketch {
                    definition_id,
                    name,
                    workplane,
                    entities,
                    constraints,
                } => {
                    if *definition_id == 0 {
                        return Err("assistant sketch creation is invalid".to_owned());
                    }
                    validate_assistant_sketch_payload(name, workplane, entities, constraints)?;
                    0
                }
                AssistantCadEditOperation::CreatePart {
                    name,
                    workplane,
                    entities,
                    constraints,
                    feature,
                    translation_mm,
                    rotation,
                } => {
                    validate_assistant_sketch_payload(name, workplane, entities, constraints)?;
                    feature.validate()?;
                    if !assistant_cad_vector_is_bounded(*translation_mm) {
                        return Err("assistant CAD part placement is invalid".to_owned());
                    }
                    if let Some(rotation) = rotation {
                        rotation.validate()?;
                    }
                    1
                }
                AssistantCadEditOperation::AppendFeature {
                    definition_id,
                    name,
                    feature,
                } => {
                    if *definition_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err("assistant CAD feature append is invalid".to_owned());
                    }
                    feature.validate()?;
                    0
                }
                AssistantCadEditOperation::SetDimension {
                    feature_id,
                    constraint_id,
                    value_mm,
                } => {
                    if *feature_id == 0
                        || constraint_id == &Some(0)
                        || !value_mm.is_finite()
                        || *value_mm <= 0.0
                        || *value_mm > MAX_ASSISTANT_ABS_MM
                    {
                        return Err("assistant CAD dimension edit is invalid".to_owned());
                    }
                    0
                }
                AssistantCadEditOperation::Delete { .. } => 0,
                AssistantCadEditOperation::Transform {
                    translation_mm,
                    rotation,
                    ..
                } => {
                    if !assistant_cad_vector_is_bounded(*translation_mm)
                        || (!assistant_cad_vector_is_nonzero(*translation_mm) && rotation.is_none())
                    {
                        return Err("assistant CAD transform is invalid".to_owned());
                    }
                    if let Some(rotation) = rotation {
                        rotation.validate()?;
                    }
                    0
                }
                AssistantCadEditOperation::Copy { translation_mm, .. } => {
                    if !assistant_cad_vector_is_bounded(*translation_mm)
                        || !assistant_cad_vector_is_nonzero(*translation_mm)
                    {
                        return Err("assistant CAD copy is invalid".to_owned());
                    }
                    1
                }
                AssistantCadEditOperation::LinearPattern {
                    instances, step_mm, ..
                } => {
                    if !(2..=MAX_ASSISTANT_ARRAY_INSTANCES).contains(instances)
                        || !assistant_cad_vector_is_bounded(*step_mm)
                        || !assistant_cad_vector_is_nonzero(*step_mm)
                        || step_mm.iter().any(|value| {
                            (*value * f64::from(instances.saturating_sub(1))).abs()
                                > MAX_ASSISTANT_ABS_MM
                        })
                    {
                        return Err("assistant CAD linear pattern is invalid".to_owned());
                    }
                    instances.saturating_sub(1) as usize
                }
                AssistantCadEditOperation::Mirror {
                    plane_origin_mm,
                    plane_normal,
                    ..
                } => {
                    let normal_length_squared =
                        plane_normal.iter().map(|value| value * value).sum::<f64>();
                    if !assistant_cad_vector_is_bounded(*plane_origin_mm)
                        || !assistant_cad_vector_is_bounded(*plane_normal)
                        || !normal_length_squared.is_finite()
                        || normal_length_squared <= f64::EPSILON
                    {
                        return Err("assistant CAD mirror is invalid".to_owned());
                    }
                    1
                }
            };
            generated_occurrences = generated_occurrences
                .checked_add(
                    bounded_targets
                        .checked_mul(generated_per_target)
                        .ok_or_else(|| {
                            "assistant CAD generated occurrence count is invalid".to_owned()
                        })?,
                )
                .ok_or_else(|| "assistant CAD generated occurrence count is invalid".to_owned())?;
            if generated_occurrences > MAX_ASSISTANT_CAD_GENERATED_OCCURRENCES {
                return Err("assistant CAD edit program creates too many occurrences".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantBottleFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTeapotIntent {
    pub handle_clearance_mm: f64,
    pub handle_tube_radius_mm: f64,
    pub spout_length_mm: f64,
    pub spout_radius_mm: f64,
    pub lid_height_mm: f64,
    pub lid_knob_radius_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantKetchupBottleIntent {
    pub body_depth_ratio: f64,
    pub cap_radius_mm: f64,
    pub cap_height_mm: f64,
    pub label_width_mm: f64,
    pub label_height_mm: f64,
    pub label_relief_mm: f64,
    pub grip_rib_count: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBottleIntent {
    pub name: String,
    pub body_radius_mm: f64,
    pub body_height_mm: f64,
    pub shoulder_rise_mm: f64,
    pub neck_radius_mm: f64,
    pub neck_height_mm: f64,
    pub wall_thickness_mm: f64,
    pub finish_kind: AssistantBottleFinishKind,
    pub finish_amount_mm: f64,
    pub origin_mm: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teapot: Option<AssistantTeapotIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ketchup_bottle: Option<AssistantKetchupBottleIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBalloonTextIntent {
    pub name: String,
    pub text: String,
    pub height_mm: f64,
    pub depth_mm: f64,
    pub stroke_width_mm: f64,
    pub letter_spacing_mm: f64,
    pub origin_mm: [f64; 3],
}

impl AssistantBottleIntent {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_ASSISTANT_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("assistant bottle name is invalid".to_owned());
        }
        if self
            .origin_mm
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
        {
            return Err("assistant bottle origin is outside the envelope".to_owned());
        }
        let dimensions = [
            self.body_radius_mm,
            self.body_height_mm,
            self.shoulder_rise_mm,
            self.neck_radius_mm,
            self.neck_height_mm,
            self.wall_thickness_mm,
            self.finish_amount_mm,
        ];
        if dimensions
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
            || self.neck_radius_mm >= self.body_radius_mm
        {
            return Err("assistant bottle dimensions are outside the envelope".to_owned());
        }
        let source_profile = vec![
            [0.0, 0.0],
            [self.body_radius_mm, 0.0],
            [self.body_radius_mm, self.body_height_mm],
            [
                self.neck_radius_mm,
                self.body_height_mm + self.shoulder_rise_mm,
            ],
            [
                self.neck_radius_mm,
                self.body_height_mm + self.shoulder_rise_mm + self.neck_height_mm,
            ],
            [
                0.0,
                self.body_height_mm + self.shoulder_rise_mm + self.neck_height_mm,
            ],
        ];
        let profile = controlled_bottle_profile(
            &source_profile,
            self.body_radius_mm,
            self.body_height_mm,
            self.shoulder_rise_mm,
        )
        .map_err(|_| "assistant bottle profile is unsupported".to_owned())?;
        inner_shell_profile(&profile, self.wall_thickness_mm)
            .map_err(|_| "assistant bottle wall thickness is unsupported".to_owned())?;
        if !finish_amount_is_conservative(&profile, self.finish_amount_mm) {
            return Err("assistant bottle edge finish is unsupported".to_owned());
        }
        if let Some(teapot) = &self.teapot {
            let dimensions = [
                teapot.handle_clearance_mm,
                teapot.handle_tube_radius_mm,
                teapot.spout_length_mm,
                teapot.spout_radius_mm,
                teapot.lid_height_mm,
                teapot.lid_knob_radius_mm,
            ];
            if dimensions.iter().any(|value| {
                !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_TEAPOT_DIMENSION_MM
            }) || teapot.handle_clearance_mm < teapot.handle_tube_radius_mm * 2.0
                || teapot.handle_tube_radius_mm >= self.body_radius_mm * 0.35
                || teapot.spout_length_mm < self.body_radius_mm * 0.75
                || teapot.spout_length_mm > self.body_radius_mm * 4.0
                || teapot.spout_radius_mm <= self.wall_thickness_mm
                || teapot.spout_radius_mm >= self.body_radius_mm * 0.5
                || teapot.lid_height_mm >= self.body_height_mm * 0.5
                || teapot.lid_knob_radius_mm >= self.neck_radius_mm * 0.75
            {
                return Err("assistant teapot dimensions are outside the envelope".to_owned());
            }
        }
        if self.teapot.is_some() && self.ketchup_bottle.is_some() {
            return Err("assistant bottle cannot combine vessel styles".to_owned());
        }
        if let Some(ketchup) = &self.ketchup_bottle {
            let dimensions = [
                ketchup.cap_radius_mm,
                ketchup.cap_height_mm,
                ketchup.label_width_mm,
                ketchup.label_height_mm,
                ketchup.label_relief_mm,
            ];
            if !ketchup.body_depth_ratio.is_finite()
                || !(0.5..=1.0).contains(&ketchup.body_depth_ratio)
                || dimensions.iter().any(|value| {
                    !value.is_finite()
                        || *value <= 0.0
                        || *value > MAX_ASSISTANT_TEAPOT_DIMENSION_MM
                })
                || ketchup.cap_radius_mm <= self.neck_radius_mm + self.wall_thickness_mm * 1.75
                || ketchup.cap_radius_mm >= self.body_radius_mm * 0.55
                || ketchup.cap_height_mm <= self.neck_height_mm + self.wall_thickness_mm * 2.0
                || ketchup.cap_height_mm >= self.body_height_mm * 0.35
                || ketchup.label_width_mm >= self.body_radius_mm * 1.8
                || ketchup.label_height_mm >= self.body_height_mm * 0.7
                || ketchup.label_relief_mm >= self.body_radius_mm * 0.1
                || !(8..=48).contains(&ketchup.grip_rib_count)
            {
                return Err(
                    "assistant ketchup bottle dimensions are outside the envelope".to_owned(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantGableRoofIntent {
    pub name: String,
    pub length_mm: f64,
    pub span_mm: f64,
    pub rise_mm: f64,
    pub thickness_mm: f64,
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantStaircaseIntent {
    pub name: String,
    pub run_mm: f64,
    pub width_mm: f64,
    pub rise_mm: f64,
    pub step_count: u32,
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBeamNotchIntent {
    pub from_start_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOrientedBeamIntent {
    pub name: String,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub up_hint: [f64; 3],
    pub width_mm: f64,
    pub depth_mm: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bottom_notches: Vec<AssistantBeamNotchIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantModelIntent {
    pub replace_scene: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<AssistantBoxIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<AssistantTranslationIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rotations: Vec<AssistantRotationIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_translations: Vec<AssistantProfileTranslationIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_edits: Vec<AssistantParameterEditIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linear_arrays: Vec<AssistantLinearArrayIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bottles: Vec<AssistantBottleIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub balloon_texts: Vec<AssistantBalloonTextIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gable_roofs: Vec<AssistantGableRoofIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staircases: Vec<AssistantStaircaseIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oriented_beams: Vec<AssistantOrientedBeamIntent>,
}

fn boxes_overlap(left: &AssistantSubtractionIntent, right: &AssistantSubtractionIntent) -> bool {
    (0..3).all(|axis| {
        left.origin_mm[axis] < right.origin_mm[axis] + right.size_mm[axis]
            && right.origin_mm[axis] < left.origin_mm[axis] + left.size_mm[axis]
    })
}

impl AssistantOrientedBeamIntent {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_ASSISTANT_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("assistant oriented beam name is invalid".to_owned());
        }
        if self
            .start_mm
            .iter()
            .chain(self.end_mm.iter())
            .chain(self.up_hint.iter())
            .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            || !self.width_mm.is_finite()
            || self.width_mm <= 0.0
            || self.width_mm > MAX_ASSISTANT_ABS_MM
            || !self.depth_mm.is_finite()
            || self.depth_mm <= 0.0
            || self.depth_mm > MAX_ASSISTANT_ABS_MM
        {
            return Err("assistant oriented beam dimensions are outside the envelope".to_owned());
        }
        let axis = [
            self.end_mm[0] - self.start_mm[0],
            self.end_mm[1] - self.start_mm[1],
            self.end_mm[2] - self.start_mm[2],
        ];
        let axis_length = axis.iter().map(|value| value * value).sum::<f64>().sqrt();
        let up_length = self
            .up_hint
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        let cross = [
            axis[1] * self.up_hint[2] - axis[2] * self.up_hint[1],
            axis[2] * self.up_hint[0] - axis[0] * self.up_hint[2],
            axis[0] * self.up_hint[1] - axis[1] * self.up_hint[0],
        ];
        let cross_length = cross.iter().map(|value| value * value).sum::<f64>().sqrt();
        if axis_length <= f64::EPSILON
            || axis_length > MAX_ASSISTANT_ABS_MM
            || up_length <= f64::EPSILON
            || cross_length <= axis_length * up_length * 1.0e-6
        {
            return Err("assistant oriented beam axis or up hint is invalid".to_owned());
        }
        if self.bottom_notches.len() > MAX_ASSISTANT_BEAM_NOTCHES {
            return Err("assistant oriented beam contains too many notches".to_owned());
        }
        for notch in &self.bottom_notches {
            if !notch.from_start_mm.is_finite()
                || notch.from_start_mm < 0.0
                || !notch.length_mm.is_finite()
                || notch.length_mm <= 0.0
                || notch.from_start_mm + notch.length_mm > axis_length
                || !notch.depth_mm.is_finite()
                || notch.depth_mm <= 0.0
                || notch.depth_mm >= self.depth_mm
            {
                return Err("assistant oriented beam notch is invalid".to_owned());
            }
        }
        if self.bottom_notches.iter().enumerate().any(|(index, left)| {
            self.bottom_notches[index + 1..].iter().any(|right| {
                left.from_start_mm < right.from_start_mm + right.length_mm
                    && right.from_start_mm < left.from_start_mm + left.length_mm
            })
        }) {
            return Err("assistant oriented beam notches overlap".to_owned());
        }
        Ok(())
    }
}

impl AssistantModelIntent {
    pub fn validate(&self) -> Result<(), String> {
        if self.boxes.is_empty()
            && self.translations.is_empty()
            && self.rotations.is_empty()
            && self.profile_translations.is_empty()
            && self.parameter_edits.is_empty()
            && self.linear_arrays.is_empty()
            && self.bottles.is_empty()
            && self.balloon_texts.is_empty()
            && self.gable_roofs.is_empty()
            && self.staircases.is_empty()
            && self.oriented_beams.is_empty()
        {
            return Err(
                "assistant proposal must contain geometry, translations, rotations, profile translations, parameter edits, linear arrays, bottles, balloon text, roofs, staircases, or oriented beams"
                    .to_owned(),
            );
        }
        if self.boxes.len() > MAX_ASSISTANT_BOXES {
            return Err("assistant proposal contains more than 64 boxes".to_owned());
        }
        if self.translations.len() > MAX_ASSISTANT_TRANSLATIONS {
            return Err("assistant proposal contains more than 100 translations".to_owned());
        }
        if self.rotations.len() > MAX_ASSISTANT_ROTATIONS {
            return Err("assistant proposal contains more than 100 rotations".to_owned());
        }
        if self.profile_translations.len() > MAX_ASSISTANT_PROFILE_TRANSLATIONS {
            return Err("assistant proposal contains more than one profile translation".to_owned());
        }
        if self.parameter_edits.len() > 1 {
            return Err("assistant proposal contains more than one parameter edit".to_owned());
        }
        if self.linear_arrays.len() > MAX_ASSISTANT_ARRAYS {
            return Err("assistant proposal contains too many linear arrays".to_owned());
        }
        if self.bottles.len() > MAX_ASSISTANT_BOTTLES {
            return Err("assistant proposal contains too many bottles".to_owned());
        }
        if self.balloon_texts.len() > MAX_ASSISTANT_BALLOON_TEXTS {
            return Err("assistant proposal contains too many balloon texts".to_owned());
        }
        if self.gable_roofs.len() > MAX_ASSISTANT_GABLE_ROOFS {
            return Err("assistant proposal contains too many gable roofs".to_owned());
        }
        if self.staircases.len() > MAX_ASSISTANT_STAIRCASES {
            return Err("assistant proposal contains too many staircases".to_owned());
        }
        if self.oriented_beams.len() > MAX_ASSISTANT_ORIENTED_BEAMS {
            return Err("assistant proposal contains too many oriented beams".to_owned());
        }
        for beam in &self.oriented_beams {
            beam.validate()?;
        }
        for bottle in &self.bottles {
            bottle.validate()?;
        }
        for text in &self.balloon_texts {
            let characters = text.text.chars().collect::<Vec<_>>();
            if text.name.trim().is_empty()
                || text.name.len() > MAX_ASSISTANT_NAME_BYTES
                || text.name.chars().any(char::is_control)
                || characters.is_empty()
                || characters.len() > MAX_ASSISTANT_BALLOON_TEXT_CHARS
                || characters.iter().all(|character| *character == ' ')
                || characters
                    .iter()
                    .any(|character| !matches!(character, 'A'..='Z' | '0'..='9' | ' ' | 'ˇ'))
                || !text.height_mm.is_finite()
                || !(10.0..=MAX_ASSISTANT_TEAPOT_DIMENSION_MM).contains(&text.height_mm)
                || !text.depth_mm.is_finite()
                || !(text.height_mm * 0.1..=text.height_mm * 0.8).contains(&text.depth_mm)
                || !text.stroke_width_mm.is_finite()
                || !(text.height_mm * 0.08..=text.height_mm * 0.24).contains(&text.stroke_width_mm)
                || !text.letter_spacing_mm.is_finite()
                || !(0.0..=text.height_mm).contains(&text.letter_spacing_mm)
                || text
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant balloon text is invalid".to_owned());
            }
        }
        for roof in &self.gable_roofs {
            if roof.name.trim().is_empty()
                || roof.name.len() > MAX_ASSISTANT_NAME_BYTES
                || roof.name.chars().any(char::is_control)
                || [
                    roof.length_mm,
                    roof.span_mm,
                    roof.rise_mm,
                    roof.thickness_mm,
                ]
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
                || roof.thickness_mm >= roof.rise_mm
                || roof
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant gable roof is invalid".to_owned());
            }
        }
        for stairs in &self.staircases {
            let tread_mm = stairs.run_mm / f64::from(stairs.step_count.max(1));
            let riser_mm = stairs.rise_mm / f64::from(stairs.step_count.max(1));
            if stairs.name.trim().is_empty()
                || stairs.name.len() > MAX_ASSISTANT_NAME_BYTES
                || stairs.name.chars().any(char::is_control)
                || [stairs.run_mm, stairs.width_mm, stairs.rise_mm]
                    .iter()
                    .any(|value| {
                        !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM
                    })
                || !(2..=64).contains(&stairs.step_count)
                || !(150.0..=450.0).contains(&tread_mm)
                || !(100.0..=250.0).contains(&riser_mm)
                || stairs.width_mm < 500.0
                || stairs
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant staircase is invalid".to_owned());
            }
        }
        if self.replace_scene
            && (!self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty())
        {
            return Err("assistant edits of existing geometry cannot replace the scene".to_owned());
        }
        if !self.profile_translations.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty()
                || !self.bottles.is_empty()
                || !self.balloon_texts.is_empty()
                || !self.gable_roofs.is_empty()
                || !self.staircases.is_empty()
                || !self.oriented_beams.is_empty())
        {
            return Err("assistant profile translation cannot mix geometry mutations".to_owned());
        }
        if !self.parameter_edits.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.linear_arrays.is_empty()
                || !self.bottles.is_empty()
                || !self.balloon_texts.is_empty()
                || !self.gable_roofs.is_empty()
                || !self.staircases.is_empty()
                || !self.oriented_beams.is_empty())
        {
            return Err("assistant parameter edit cannot mix geometry mutations".to_owned());
        }
        let mut translated_occurrences = BTreeSet::new();
        for translation in &self.translations {
            if translation.occurrence_id == 0
                || !translated_occurrences.insert(translation.occurrence_id)
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant translation is invalid".to_owned());
            }
        }
        let mut rotated_occurrences = BTreeSet::new();
        let mut rotated_groups = BTreeSet::new();
        for rotation in &self.rotations {
            let target_is_valid = match (rotation.occurrence_id, rotation.group_id) {
                (Some(id), None) => {
                    id != 0
                        && !translated_occurrences.contains(&id)
                        && rotated_occurrences.insert(id)
                }
                (None, Some(id)) => id != 0 && rotated_groups.insert(id),
                _ => false,
            };
            let axis_length_squared = rotation.axis.iter().map(|value| value * value).sum::<f64>();
            let normalized_angle = rotation.angle_degrees.rem_euclid(360.0);
            let shortest_angle = normalized_angle.min(360.0 - normalized_angle);
            if !target_is_valid
                || rotation
                    .pivot_mm
                    .iter()
                    .chain(rotation.axis.iter())
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
                || !axis_length_squared.is_finite()
                || axis_length_squared <= f64::EPSILON
                || !rotation.angle_degrees.is_finite()
                || rotation.angle_degrees.abs() > MAX_ASSISTANT_ABS_MM
                || shortest_angle < 0.01
            {
                return Err("assistant rotation is invalid".to_owned());
            }
        }
        if !rotated_occurrences.is_empty() && !rotated_groups.is_empty() {
            return Err("assistant rotation cannot mix occurrence and group targets".to_owned());
        }
        for translation in &self.profile_translations {
            if translation.definition_id == 0
                || translation.body_id == 0
                || translation.profile_id == 0
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
                || translation.delta_mm.iter().all(|value| *value == 0.0)
            {
                return Err("assistant profile translation is invalid".to_owned());
            }
        }
        for edit in &self.parameter_edits {
            if edit.definition_id == 0
                || edit.body_id == 0
                || edit.feature_id == 0
                || edit.constraint_id == Some(0)
                || !edit.value_mm.is_finite()
                || edit.value_mm <= 0.0
                || edit.value_mm > MAX_ASSISTANT_ABS_MM
            {
                return Err("assistant parameter edit is invalid".to_owned());
            }
        }
        let mut array_outputs = 0usize;
        for array in &self.linear_arrays {
            let mut occurrence_ids = BTreeSet::new();
            if array.occurrence_ids.is_empty()
                || array.occurrence_ids.len() > MAX_ASSISTANT_ARRAY_SOURCES
                || array.instances < 2
                || array.instances > MAX_ASSISTANT_ARRAY_INSTANCES
                || array
                    .occurrence_ids
                    .iter()
                    .any(|id| *id == 0 || !occurrence_ids.insert(*id))
                || array
                    .step_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
                || array.step_mm.iter().all(|value| *value == 0.0)
                || array.step_mm.iter().any(|value| {
                    (*value * f64::from(array.instances.saturating_sub(1))).abs()
                        > MAX_ASSISTANT_ABS_MM
                })
            {
                return Err("assistant linear array is invalid".to_owned());
            }
            let Some(outputs) = array
                .occurrence_ids
                .len()
                .checked_mul(array.instances.saturating_sub(1) as usize)
            else {
                return Err("assistant linear array output count is invalid".to_owned());
            };
            let Some(total_outputs) = array_outputs.checked_add(outputs) else {
                return Err("assistant linear array output count is invalid".to_owned());
            };
            if total_outputs > MAX_ASSISTANT_ARRAY_OUTPUTS {
                return Err("assistant proposal creates too many array occurrences".to_owned());
            }
            array_outputs = total_outputs;
        }
        for item in &self.boxes {
            if item.name.trim().is_empty()
                || item.name.len() > MAX_ASSISTANT_NAME_BYTES
                || item.name.chars().any(char::is_control)
            {
                return Err("assistant box name is invalid".to_owned());
            }
            if item
                .size_mm
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
                || item
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err(
                    "assistant box dimensions or origin are outside the envelope".to_owned(),
                );
            }
            if item.subtract_boxes.len() > MAX_ASSISTANT_SUBTRACTIONS {
                return Err("assistant body contains more than 64 subtractions".to_owned());
            }
            let [width, depth, height] = item.size_mm;
            for subtraction in &item.subtract_boxes {
                let [cut_width, cut_depth, cut_height] = subtraction.size_mm;
                let [cut_x, cut_y, cut_z] = subtraction.origin_mm;
                let retained_through_opening = cut_z == 0.0
                    && cut_height == height
                    && cut_x > 0.0
                    && cut_y > 0.0
                    && cut_x + cut_width < width
                    && cut_y + cut_depth < depth;
                if subtraction
                    .size_mm
                    .iter()
                    .any(|value| !value.is_finite() || *value <= 0.0)
                    || subtraction.origin_mm.iter().any(|value| !value.is_finite())
                    || cut_x < 0.0
                    || cut_y < 0.0
                    || cut_z < 0.0
                    || cut_x + cut_width > width
                    || cut_y + cut_depth > depth
                    || cut_z + cut_height > height
                    || (cut_height >= height && !retained_through_opening)
                {
                    return Err("assistant subtraction is outside its body".to_owned());
                }
            }
            if item.subtract_boxes.iter().enumerate().any(|(index, left)| {
                item.subtract_boxes[index + 1..]
                    .iter()
                    .any(|right| boxes_overlap(left, right))
            }) {
                return Err("assistant subtractions overlap".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantRejectionPhase {
    IntentValidation,
    ProposalPlanning,
    CanonicalValidation,
    ExactValidation,
    DomainValidation,
    CommitValidation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantRejectionDiagnostic {
    pub phase: AssistantRejectionPhase,
    pub code: String,
    pub operation: String,
    pub target: String,
    pub failed_invariant: String,
    pub repair_hint: String,
    pub retryable: bool,
}

impl AssistantRejectionDiagnostic {
    pub fn validate(&self) -> Result<(), String> {
        let machine_identifier_is_valid = |value: &str, max_bytes: usize| {
            !value.is_empty()
                && value.len() <= max_bytes
                && value.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
        };
        let bounded_text_is_valid = |value: &str, max_bytes: usize| {
            !value.trim().is_empty()
                && value.len() <= max_bytes
                && !value.chars().any(char::is_control)
        };
        if !machine_identifier_is_valid(&self.code, MAX_ASSISTANT_REJECTION_CODE_BYTES)
            || !machine_identifier_is_valid(
                &self.operation,
                MAX_ASSISTANT_REJECTION_OPERATION_BYTES,
            )
            || !bounded_text_is_valid(&self.target, MAX_ASSISTANT_REJECTION_TARGET_BYTES)
            || !bounded_text_is_valid(&self.failed_invariant, MAX_ASSISTANT_REJECTION_TEXT_BYTES)
            || !bounded_text_is_valid(&self.repair_hint, MAX_ASSISTANT_REJECTION_TEXT_BYTES)
            || serde_json::to_vec(self)
                .map_or(true, |bytes| bytes.len() > MAX_ASSISTANT_REJECTION_BYTES)
        {
            return Err("assistant rejection diagnostic is invalid or too large".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantApiDiagnostics {
    pub provider: String,
    pub model: String,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub stop_reason: String,
    pub system_prompt: String,
    pub request_payload: serde_json::Value,
    pub response_text: String,
}

impl AssistantApiDiagnostics {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.is_empty()
            || self.provider.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.model.is_empty()
            || self.model.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.system_prompt.len() > 64 * 1024
            || self.response_text.len() > 64 * 1024
            || serde_json::to_vec(&self.request_payload)
                .map_or(true, |bytes| bytes.len() > 128 * 1024)
        {
            return Err("assistant API diagnostics exceed their bounded envelope".to_owned());
        }
        Ok(())
    }

    #[must_use]
    pub const fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantChatResult {
    pub message: String,
    pub model_intent: Option<AssistantModelIntent>,
}

impl AssistantChatResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.message.trim().is_empty() {
            return Err("assistant returned an empty message".to_owned());
        }
        if let Some(intent) = &self.model_intent {
            intent.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantHandshake {
    pub protocol_version: u16,
    pub distribution: AssistantDistribution,
    pub provider: String,
    pub model: String,
    pub capabilities: BTreeSet<AssistantCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantHandshakeError {
    InvalidJson(String),
    UnsupportedProtocolVersion(u16),
    UnsupportedDistribution(AssistantDistribution),
    UnsupportedProvider(String),
    UnsupportedModel(String),
    UnsupportedCapabilities(BTreeSet<AssistantCapability>),
}

impl fmt::Display for AssistantHandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid assistant handshake: {error}"),
            Self::UnsupportedProtocolVersion(version) => {
                write!(
                    formatter,
                    "unsupported assistant protocol version: {version}"
                )
            }
            Self::UnsupportedDistribution(distribution) => {
                write!(
                    formatter,
                    "unsupported assistant distribution: {distribution:?}"
                )
            }
            Self::UnsupportedProvider(provider) => {
                write!(formatter, "unsupported assistant provider: {provider}")
            }
            Self::UnsupportedModel(model) => {
                write!(formatter, "unsupported assistant model: {model}")
            }
            Self::UnsupportedCapabilities(capabilities) => {
                write!(
                    formatter,
                    "unsupported assistant capabilities: {capabilities:?}"
                )
            }
        }
    }
}

impl std::error::Error for AssistantHandshakeError {}

impl AssistantHandshake {
    pub fn parse_and_validate(line: &str) -> Result<Self, AssistantHandshakeError> {
        let handshake: Self = serde_json::from_str(line)
            .map_err(|error| AssistantHandshakeError::InvalidJson(error.to_string()))?;
        handshake.validate()?;
        Ok(handshake)
    }

    pub fn validate(&self) -> Result<(), AssistantHandshakeError> {
        if self.protocol_version != ASSISTANT_PROTOCOL_VERSION {
            return Err(AssistantHandshakeError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }

        if !distribution_is_enabled(self.distribution) {
            return Err(AssistantHandshakeError::UnsupportedDistribution(
                self.distribution,
            ));
        }

        let provider_supported = match self.distribution {
            AssistantDistribution::PublicApi => {
                matches!(self.provider.as_str(), "anthropic-api" | "openai-api")
            }
            AssistantDistribution::PrivateOauth => {
                matches!(self.provider.as_str(), "claude-code-oauth" | "codex-oauth")
            }
        };
        if !provider_supported {
            return Err(AssistantHandshakeError::UnsupportedProvider(
                self.provider.clone(),
            ));
        }
        if self.model.is_empty()
            || self.model.len() > MAX_ASSISTANT_MODEL_BYTES
            || !self.model.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
        {
            return Err(AssistantHandshakeError::UnsupportedModel(
                self.model.clone(),
            ));
        }

        let allowed = BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::DebugObservability,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]);
        if !self.capabilities.is_subset(&allowed) {
            return Err(AssistantHandshakeError::UnsupportedCapabilities(
                self.capabilities.difference(&allowed).copied().collect(),
            ));
        }

        Ok(())
    }
}

#[must_use]
pub const fn distribution_is_enabled(distribution: AssistantDistribution) -> bool {
    match distribution {
        AssistantDistribution::PublicApi => true,
        AssistantDistribution::PrivateOauth => cfg!(feature = "private-oauth"),
    }
}
