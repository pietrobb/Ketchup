use ketchup_geometry::linalg::{cross, dot};
use ketchup_model::document::{
    Dimension, SpatialPathSegment, WeldmentJointPolicy, WeldmentJointPrimary,
    is_valid_spatial_sweep_path,
};
use ketchup_model::exact_product::EXACT_MIN_LENGTH_MM;
use ketchup_model::pin_joint::PinSpec;
use ketchup_model::sheet_metal::{SheetMetalEdge, SheetMetalFlange, SheetMetalSpec};
use ketchup_model::tolerance::{
    APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, MAX_COORDINATE_MM, ROUNDING,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::request_invalid::{AssistantRequestInvalid, AssistantRequestProblem};

pub const ASSISTANT_PROTOCOL_VERSION: u16 = 3;
const MAX_ASSISTANT_MODEL_BYTES: usize = 128;
const MAX_ASSISTANT_BOXES: usize = 64;
const MAX_ASSISTANT_SUBTRACTIONS: usize = 64;
const MAX_ASSISTANT_TRANSLATIONS: usize = 100;
const MAX_ASSISTANT_ROTATIONS: usize = 100;
const MAX_ASSISTANT_PROFILE_TRANSLATIONS: usize = 1;
const MAX_ASSISTANT_PARAMETER_EDITS: usize = 1;
const MAX_ASSISTANT_PANEL_POCKETS: usize = 128;
const MAX_ASSISTANT_ARRAYS: usize = 16;
const MAX_ASSISTANT_ARRAY_SOURCES: usize = 100;
const MAX_ASSISTANT_ARRAY_INSTANCES: u32 = 1_000;
const MAX_ASSISTANT_ARRAY_OUTPUTS: usize = 512;
const MAX_ASSISTANT_CAD_EDIT_OPERATIONS: usize = 64;
const MAX_ASSISTANT_CAD_SELECTOR_TARGETS: usize = 100;
const MAX_ASSISTANT_CAD_GENERATED_OCCURRENCES: usize = 512;
const MAX_ASSISTANT_NAME_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_CODE_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_OPERATION_BYTES: usize = 128;
const MAX_ASSISTANT_REJECTION_TARGET_BYTES: usize = 256;
const MAX_ASSISTANT_REJECTION_TEXT_BYTES: usize = 2_048;
const MAX_ASSISTANT_REJECTION_BYTES: usize = 8 * 1_024;
const MAX_ASSISTANT_HELIX_TURNS: f64 = 16.0;
const MAX_ASSISTANT_HELIX_SEGMENTS: usize = 64;
const MAX_ASSISTANT_SPATIAL_PATH_SEGMENTS: usize = 64;
const MAX_ASSISTANT_PROFILE_COPIES: usize = 16;
const MAX_ASSISTANT_INSTANCE_PATH_STEPS: usize = 256;
// Four quarter-ellipse cubics using kappa have peak normalized radial error < 0.000273.
const ELLIPSE_CUBIC_MAX_NORMALIZED_RADIAL_DEVIATION: f64 = 0.000_273;

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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantInstancePath {
    pub root_occurrence_id: u64,
    pub steps: Vec<AssistantInstancePathStep>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantInstancePathStep {
    Group {
        owner_definition_id: u64,
        local_id: u64,
    },
    Occurrence {
        owner_definition_id: u64,
        local_id: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantAssemblyJointAxis {
    pub direction_in_parent: [f64; 3],
    pub pivot_in_parent_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantAssemblyJointLimits {
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantAssemblyJointKind {
    Fixed,
    Revolute {
        axis: AssistantAssemblyJointAxis,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<AssistantAssemblyJointLimits>,
        position_degrees: f64,
    },
    Prismatic {
        axis: AssistantAssemblyJointAxis,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<AssistantAssemblyJointLimits>,
        position_mm: f64,
    },
    Helical {
        axis: AssistantAssemblyJointAxis,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<AssistantAssemblyJointLimits>,
        lead_mm_per_revolution: f64,
        position_degrees: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantAxisSpec {
    OriginDirection {
        origin_mm: [f64; 3],
        direction: [f64; 3],
    },
    TwoPoints {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    ConstructionAxis {
        axis: AssistantCadFeatureReference,
    },
    Edge {
        edge_reference_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instance_path: Option<AssistantInstancePath>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantHelixHandedness {
    Right,
    Left,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantHelixParameters {
    pub axis: AssistantAxisSpec,
    pub radius_mm: f64,
    pub pitch_mm: f64,
    pub turns: f64,
    pub start_angle_degrees: f64,
    pub handedness: AssistantHelixHandedness,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantSpatialPathSegment {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    CircularArc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        normal: [f64; 3],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 3],
        control_1_mm: [f64; 3],
        control_2_mm: [f64; 3],
        end_mm: [f64; 3],
    },
}

impl AssistantSpatialPathSegment {
    fn canonical(&self) -> SpatialPathSegment {
        match self {
            Self::Line { start_mm, end_mm } => SpatialPathSegment::Line {
                start_mm: *start_mm,
                end_mm: *end_mm,
            },
            Self::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                normal,
                clockwise,
            } => SpatialPathSegment::CircularArc {
                start_mm: *start_mm,
                end_mm: *end_mm,
                center_mm: *center_mm,
                normal: *normal,
                clockwise: *clockwise,
            },
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => SpatialPathSegment::CubicBezier {
                start_mm: *start_mm,
                control_1_mm: *control_1_mm,
                control_2_mm: *control_2_mm,
                end_mm: *end_mm,
            },
        }
    }
}

pub fn validated_spatial_path_segments(
    segments: &[AssistantSpatialPathSegment],
    tolerance_mm: f64,
) -> Result<Vec<SpatialPathSegment>, AssistantRequestInvalid> {
    if !(1..=MAX_ASSISTANT_SPATIAL_PATH_SEGMENTS).contains(&segments.len()) {
        return Err(AssistantRequestInvalid::invalid(
            "spatial path segment count",
        ));
    }
    let segments = segments
        .iter()
        .map(AssistantSpatialPathSegment::canonical)
        .collect::<Vec<_>>();
    if !is_valid_spatial_sweep_path(&segments, tolerance_mm) {
        return Err(AssistantRequestInvalid::invalid("spatial path"));
    }
    Ok(segments)
}

impl Default for AssistantHelixParameters {
    fn default() -> Self {
        Self {
            axis: AssistantAxisSpec::OriginDirection {
                origin_mm: [0.0, 0.0, 0.0],
                direction: [0.0, 0.0, 1.0],
            },
            radius_mm: 10.0,
            pitch_mm: 5.0,
            turns: 3.0,
            start_angle_degrees: 0.0,
            handedness: AssistantHelixHandedness::Right,
        }
    }
}

impl AssistantHelixHandedness {
    const fn sign(self) -> f64 {
        match self {
            Self::Right => 1.0,
            Self::Left => -1.0,
        }
    }
}

impl AssistantAxisSpec {
    pub fn origin_and_direction(&self) -> Result<([f64; 3], [f64; 3]), AssistantRequestInvalid> {
        let (origin_mm, direction) = match self {
            Self::OriginDirection {
                origin_mm,
                direction,
            } => (*origin_mm, *direction),
            Self::TwoPoints { start_mm, end_mm }
                if assistant_cad_vector_is_bounded(*start_mm)
                    && assistant_cad_vector_is_bounded(*end_mm) =>
            {
                (*start_mm, assistant_sub(*end_mm, *start_mm))
            }
            Self::TwoPoints { .. } => return Err(AssistantRequestInvalid::invalid("axis")),
            Self::ConstructionAxis { .. } | Self::Edge { .. } => {
                return Err(AssistantRequestInvalid::new(
                    "referenced axis",
                    AssistantRequestProblem::NeedsDocumentResolution,
                ));
            }
        };
        let direction_length_squared = direction.iter().map(|value| value * value).sum::<f64>();
        if !assistant_cad_vector_is_bounded(origin_mm)
            || !assistant_cad_vector_is_bounded(direction)
            || !direction_length_squared.is_finite()
            || direction_length_squared <= f64::EPSILON
        {
            return Err(AssistantRequestInvalid::invalid("axis"));
        }
        Ok((origin_mm, direction))
    }

    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        match self {
            Self::ConstructionAxis { axis } => (*axis).validate(),
            Self::Edge {
                edge_reference_id,
                instance_path,
            } if edge_reference_id.len() == 64
                && edge_reference_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                && instance_path.as_ref().is_none_or(|path| {
                    path.root_occurrence_id > 0
                        && path.steps.len() <= MAX_ASSISTANT_INSTANCE_PATH_STEPS
                        && path.steps.iter().all(|step| match step {
                            AssistantInstancePathStep::Group {
                                owner_definition_id,
                                local_id,
                            }
                            | AssistantInstancePathStep::Occurrence {
                                owner_definition_id,
                                local_id,
                            } => *owner_definition_id > 0 && *local_id > 0,
                        })
                }) =>
            {
                Ok(())
            }
            Self::Edge { .. } => Err(AssistantRequestInvalid::invalid("edge axis reference")),
            direct => direct.origin_and_direction().map(|_| ()),
        }
    }

    fn validate_reference_for_operation(
        &self,
        operation_index: usize,
        operations: &[AssistantCadEditOperation],
    ) -> Result<(), AssistantRequestInvalid> {
        let Self::ConstructionAxis {
            axis: AssistantCadFeatureReference::ProgramOutput(reference),
        } = self
        else {
            return Ok(());
        };
        reference.validate_for(
            operation_index,
            operations,
            AssistantCadProgramFeatureOutput::ConstructionFeature,
        )?;
        if matches!(
            operations.get(reference.operation_index as usize),
            Some(AssistantCadEditOperation::CreateConstructionAxis { .. })
        ) {
            Ok(())
        } else {
            Err(AssistantRequestInvalid::invalid(
                "construction-axis reference",
            ))
        }
    }
}

impl AssistantHelixParameters {
    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        self.axis.validate()?;
        let axial_length = self.pitch_mm * self.turns;
        if !self.radius_mm.is_finite()
            || !(EXACT_MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&self.radius_mm)
            || !self.pitch_mm.is_finite()
            || !(EXACT_MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&self.pitch_mm)
            || !self.turns.is_finite()
            || !(0.01..=MAX_ASSISTANT_HELIX_TURNS).contains(&self.turns)
            || !self.start_angle_degrees.is_finite()
            || !axial_length.is_finite()
            || axial_length > MAX_COORDINATE_MM
        {
            return Err(AssistantRequestInvalid::invalid("helix parameter set"));
        }
        let segment_count =
            (std::f64::consts::TAU * self.turns / std::f64::consts::FRAC_PI_2).ceil() as usize;
        if !(1..=MAX_ASSISTANT_HELIX_SEGMENTS).contains(&segment_count) {
            return Err(AssistantRequestInvalid::invalid("helix segment count"));
        }
        Ok(())
    }

    pub fn spatial_path_segments(
        &self,
    ) -> Result<Vec<SpatialPathSegment>, AssistantRequestInvalid> {
        let axis = self.axis.origin_and_direction()?;
        self.spatial_path_segments_for_axis(axis)
    }

    pub fn spatial_path_segments_for_axis(
        &self,
        (origin_mm, direction): ([f64; 3], [f64; 3]),
    ) -> Result<Vec<SpatialPathSegment>, AssistantRequestInvalid> {
        self.validate()?;
        AssistantAxisSpec::OriginDirection {
            origin_mm,
            direction,
        }
        .origin_and_direction()?;
        let axis =
            assistant_unit(direction).ok_or(AssistantRequestInvalid::invalid("helix axis"))?;
        let reference = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .into_iter()
            .min_by(|left, right| dot(*left, axis).abs().total_cmp(&dot(*right, axis).abs()))
            .expect("three reference axes exist");
        let frame_u = assistant_unit(assistant_sub(
            reference,
            assistant_scale(axis, dot(reference, axis)),
        ))
        .ok_or(AssistantRequestInvalid::invalid("helix reference frame"))?;
        let frame_v = cross(axis, frame_u);
        let start_angle = self.start_angle_degrees.to_radians();
        let total_angle = std::f64::consts::TAU * self.turns;
        let segment_count = (total_angle / std::f64::consts::FRAC_PI_2).ceil() as usize;
        let handedness = self.handedness.sign();
        let rise_per_radian = self.pitch_mm / std::f64::consts::TAU;
        let point = |angle: f64| {
            let phase = start_angle + handedness * angle;
            assistant_add(
                origin_mm,
                assistant_add(
                    assistant_scale(
                        assistant_add(
                            assistant_scale(frame_u, phase.cos()),
                            assistant_scale(frame_v, phase.sin()),
                        ),
                        self.radius_mm,
                    ),
                    assistant_scale(axis, rise_per_radian * angle),
                ),
            )
        };
        let derivative = |angle: f64| {
            let phase = start_angle + handedness * angle;
            assistant_add(
                assistant_scale(
                    assistant_add(
                        assistant_scale(frame_u, -phase.sin()),
                        assistant_scale(frame_v, phase.cos()),
                    ),
                    handedness * self.radius_mm,
                ),
                assistant_scale(axis, rise_per_radian),
            )
        };
        let mut segments = Vec::with_capacity(segment_count);
        for index in 0..segment_count {
            let start = total_angle * index as f64 / segment_count as f64;
            let end = total_angle * (index + 1) as f64 / segment_count as f64;
            let delta = end - start;
            let start_mm = point(start);
            let end_mm = point(end);
            segments.push(SpatialPathSegment::CubicBezier {
                start_mm,
                control_1_mm: assistant_add(
                    start_mm,
                    assistant_scale(derivative(start), delta / 3.0),
                ),
                control_2_mm: assistant_sub(end_mm, assistant_scale(derivative(end), delta / 3.0)),
                end_mm,
            });
        }
        Ok(segments)
    }
}

fn assistant_unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    if vector.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let length = dot(vector, vector).sqrt();
    (length > ROUNDING).then(|| assistant_scale(vector, length.recip()))
}

fn assistant_add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] + right[axis])
}

fn assistant_sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

fn assistant_scale(vector: [f64; 3], factor: f64) -> [f64; 3] {
    vector.map(|value| value * factor)
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
    Frame {
        origin_mm: [f64; 3],
        x_axis: [f64; 3],
        y_axis: [f64; 3],
    },
    Principal {
        plane: AssistantPrincipalPlane,
    },
    Offset {
        base_feature_id: u64,
        distance_mm: f64,
    },
    ConstructionPlane {
        plane: AssistantCadFeatureReference,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadPartFeature {
    Extrusion {
        distance_mm: f64,
    },
    Revolve {
        axis: AssistantAxisSpec,
        angle_degrees: f64,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPanelHole {
    pub id: String,
    pub entry_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub diameter_mm: f64,
    pub depth_mm: f64,
}

/// A rectangular pocket milled into one face of a panel: the removed box in
/// panel-local coordinates. It must start on the face that `inward_unit_local`
/// points away from and may run off the face edges (grooves, rabbets).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPanelPocket {
    pub id: String,
    pub min_local_mm: [f64; 3],
    pub max_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
}

impl AssistantPanelPocket {
    /// Axis index of the pocket direction, if `inward_unit_local` is a unit axis.
    #[must_use]
    pub fn axis(&self) -> Option<usize> {
        let axis = self
            .inward_unit_local
            .iter()
            .position(|value| (value.abs() - 1.0).abs() <= ROUNDING)?;
        self.inward_unit_local
            .iter()
            .enumerate()
            .all(|(index, value)| index == axis || value.abs() <= ROUNDING)
            .then_some(axis)
    }

    fn validate(&self, dimensions_mm: [f64; 3]) -> Result<(), AssistantRequestInvalid> {
        let invalid = || {
            Err(AssistantRequestInvalid::new(
                "panel pocket",
                AssistantRequestProblem::Violates(
                    "it must be a box that starts on the face opposite to inward_unit_local, stays within the panel thickness and overlaps the face",
                ),
            )
            .item(&self.id))
        };
        let Some(axis) = self.axis() else {
            return invalid();
        };
        if self.id.trim().is_empty()
            || self.id.len() > MAX_ASSISTANT_NAME_BYTES
            || self.id.chars().any(char::is_control)
            || !assistant_cad_vector_is_bounded(self.min_local_mm)
            || !assistant_cad_vector_is_bounded(self.max_local_mm)
            || (0..3)
                .any(|index| self.max_local_mm[index] - self.min_local_mm[index] <= APPROXIMATION)
        {
            return invalid();
        }
        let entry_ok = if self.inward_unit_local[axis] > 0.0 {
            self.min_local_mm[axis].abs() <= ROUNDING
        } else {
            (self.max_local_mm[axis] - dimensions_mm[axis]).abs() <= ROUNDING
        };
        let depth = self.max_local_mm[axis] - self.min_local_mm[axis];
        let overlaps_face = (0..3).filter(|index| *index != axis).all(|index| {
            self.min_local_mm[index] < dimensions_mm[index] && self.max_local_mm[index] > 0.0
        });
        if !entry_ok || depth > dimensions_mm[axis] + ROUNDING || !overlaps_face {
            return invalid();
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPinJointFace {
    pub instance_path: AssistantInstancePath,
    pub face_origin_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub bounds_min_local_mm: [f64; 3],
    pub bounds_max_local_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPinPhysicalHolePair {
    pub first_pocket_feature_id: u64,
    pub second_pocket_feature_id: u64,
}

/// A cylindrical pin: its size and how much longer than the pin end each
/// drilled hole is. Named sizes (such as the 8x30 pin) are data in the
/// program library, not a closed list here.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantPin {
    pub diameter_mm: f64,
    pub length_mm: f64,
    pub hole_clearance_mm: f64,
}

impl AssistantPin {
    /// The pin split half and half between both parts.
    #[must_use]
    pub fn spec(self) -> PinSpec {
        PinSpec::symmetric(self.diameter_mm, self.length_mm, self.hole_clearance_mm)
    }
}

impl AssistantCadPartFeature {
    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        match self {
            Self::Extrusion { distance_mm }
                if distance_mm.is_finite()
                    && *distance_mm > 0.0
                    && *distance_mm <= MAX_COORDINATE_MM =>
            {
                Ok(())
            }
            Self::Extrusion { .. } => Err(AssistantRequestInvalid::invalid("CAD part feature")),
            Self::Revolve {
                axis,
                angle_degrees,
            } if axis.validate().is_ok()
                && angle_degrees.is_finite()
                && *angle_degrees > 0.0
                && *angle_degrees <= 360.0 =>
            {
                Ok(())
            }
            Self::Revolve { .. } => Err(AssistantRequestInvalid::invalid("CAD part feature")),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Ord, PartialOrd, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadProgramFeatureOutput {
    Definition,
    Occurrence,
    SketchFeature,
    ConstructionFeature,
    BodyFeature,
    AssemblyJoint,
    PinJoint,
}

#[derive(Clone, Copy, Debug, Deserialize, Ord, PartialOrd, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadProgramFeatureReference {
    pub operation_index: u32,
    pub output: AssistantCadProgramFeatureOutput,
}

impl AssistantCadProgramFeatureReference {
    fn validate_for(
        self,
        operation_index: usize,
        operations: &[AssistantCadEditOperation],
        expected_output: AssistantCadProgramFeatureOutput,
    ) -> Result<(), AssistantRequestInvalid> {
        if self.output != expected_output || self.operation_index as usize >= operation_index {
            return Err(AssistantRequestInvalid::invalid(
                "CAD program feature reference",
            ));
        }
        let Some(producer) = operations.get(self.operation_index as usize) else {
            return Err(AssistantRequestInvalid::invalid(
                "CAD program feature reference",
            ));
        };
        let available = match expected_output {
            AssistantCadProgramFeatureOutput::Definition => {
                matches!(
                    producer,
                    AssistantCadEditOperation::CreatePart { .. }
                        | AssistantCadEditOperation::CreatePanel { .. }
                        | AssistantCadEditOperation::CreateSpatialPath { .. }
                        | AssistantCadEditOperation::CreateConstructionPoint { .. }
                        | AssistantCadEditOperation::CreateConstructionAxis { .. }
                        | AssistantCadEditOperation::CreateConstructionPlane { .. }
                        | AssistantCadEditOperation::CreateHelix { .. }
                )
            }
            AssistantCadProgramFeatureOutput::Occurrence => matches!(
                producer,
                AssistantCadEditOperation::CreatePart { .. }
                    | AssistantCadEditOperation::CreatePanel { .. }
            ),
            AssistantCadProgramFeatureOutput::SketchFeature => matches!(
                producer,
                AssistantCadEditOperation::CreatePart { .. }
                    | AssistantCadEditOperation::CreatePanel { .. }
                    | AssistantCadEditOperation::CreateProgramSketch { .. }
            ),
            AssistantCadProgramFeatureOutput::ConstructionFeature => {
                matches!(
                    producer,
                    AssistantCadEditOperation::CreateSpatialPath { .. }
                        | AssistantCadEditOperation::CreateConstructionPoint { .. }
                        | AssistantCadEditOperation::CreateConstructionAxis { .. }
                        | AssistantCadEditOperation::CreateConstructionPlane { .. }
                ) || matches!(
                    producer,
                    AssistantCadEditOperation::CreateHelix { profile, .. } if profile.is_empty()
                )
            }
            AssistantCadProgramFeatureOutput::BodyFeature => match producer {
                AssistantCadEditOperation::CreateHelix { profile, .. } => !profile.is_empty(),
                AssistantCadEditOperation::CreatePart { .. }
                | AssistantCadEditOperation::CreatePanel { .. }
                | AssistantCadEditOperation::FilletEdges { .. }
                | AssistantCadEditOperation::ChamferEdges { .. }
                | AssistantCadEditOperation::AppendProgramPocket { .. } => true,
                AssistantCadEditOperation::AppendFeature { feature, .. } => {
                    feature.produces_body_feature_output()
                }
                _ => false,
            },
            AssistantCadProgramFeatureOutput::AssemblyJoint => matches!(
                producer,
                AssistantCadEditOperation::CreateAssemblyJoint { .. }
            ),
            AssistantCadProgramFeatureOutput::PinJoint => matches!(
                producer,
                AssistantCadEditOperation::CreatePinJoint { .. }
                    | AssistantCadEditOperation::CreateProgramPinJoint { .. }
                    | AssistantCadEditOperation::CreatePhysicalPinJoint { .. }
            ),
        };
        if available {
            Ok(())
        } else {
            Err(AssistantRequestInvalid::invalid(
                "CAD program feature reference",
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Ord, PartialOrd, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum AssistantCadFeatureReference {
    Existing(u64),
    ProgramOutput(AssistantCadProgramFeatureReference),
}

impl From<u64> for AssistantCadFeatureReference {
    fn from(value: u64) -> Self {
        Self::Existing(value)
    }
}

#[derive(Clone, Debug, Deserialize, Ord, PartialOrd, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadNamedProgramOutputReference {
    pub name: String,
    pub output: AssistantCadProgramFeatureOutput,
}

impl AssistantCadNamedProgramOutputReference {
    fn validate_name(&self) -> Result<(), AssistantRequestInvalid> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_ASSISTANT_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            Err(AssistantRequestInvalid::invalid(
                "CAD named program output reference",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantProgramPinJointFace {
    pub occurrence: AssistantCadNamedProgramOutputReference,
    pub face_origin_local_mm: [f64; 3],
    pub inward_unit_local: [f64; 3],
    pub bounds_min_local_mm: [f64; 3],
    pub bounds_max_local_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantProgramPinPhysicalHolePair {
    pub first_pocket_feature: AssistantCadNamedProgramOutputReference,
    pub second_pocket_feature: AssistantCadNamedProgramOutputReference,
}

impl AssistantCadFeatureReference {
    pub fn existing_id(self) -> Option<u64> {
        match self {
            Self::Existing(id) => Some(id),
            Self::ProgramOutput(_) => None,
        }
    }

    fn validate(self) -> Result<(), AssistantRequestInvalid> {
        match self {
            Self::Existing(id) if id != 0 => Ok(()),
            Self::ProgramOutput(_) => Ok(()),
            Self::Existing(_) => Err(AssistantRequestInvalid::invalid("CAD feature reference")),
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadLoftContinuity {
    Position,
    Tangent,
    Curvature,
}

#[allow(clippy::derivable_impls)]
impl Default for AssistantCadLoftContinuity {
    fn default() -> Self {
        Self::Position
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadShellDirection {
    Inward,
    Outward,
    Symmetric,
}

#[allow(clippy::derivable_impls)]
impl Default for AssistantCadShellDirection {
    fn default() -> Self {
        Self::Inward
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadLoftSection {
    pub profile_feature_id: AssistantCadFeatureReference,
    pub elevation_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadSurfaceBodySource {
    Planar {
        profile_feature_id: AssistantCadFeatureReference,
    },
    Loft {
        sections: Vec<AssistantCadLoftSection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guide_feature_id: Option<AssistantCadFeatureReference>,
        #[serde(default)]
        continuity: AssistantCadLoftContinuity,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadChamferMode {
    Symmetric,
    TwoDistance { second_distance_mm: f64 },
    DistanceAngle { angle_degrees: f64 },
}

#[allow(clippy::derivable_impls)]
impl Default for AssistantCadChamferMode {
    fn default() -> Self {
        Self::Symmetric
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadFilletRadiusStation {
    pub position: f64,
    pub radius_mm: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadSheetMetalEdge {
    MinX,
    MaxX,
    MinY,
    MaxY,
}

impl From<AssistantCadSheetMetalEdge> for SheetMetalEdge {
    fn from(value: AssistantCadSheetMetalEdge) -> Self {
        match value {
            AssistantCadSheetMetalEdge::MinX => Self::MinX,
            AssistantCadSheetMetalEdge::MaxX => Self::MaxX,
            AssistantCadSheetMetalEdge::MinY => Self::MinY,
            AssistantCadSheetMetalEdge::MaxY => Self::MaxY,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadSheetMetalFlange {
    pub edge: AssistantCadSheetMetalEdge,
    pub length_mm: f64,
    pub angle_degrees: f64,
    pub inner_radius_mm: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadWeldmentJointPolicy {
    Butt,
    Miter,
}

impl From<AssistantCadWeldmentJointPolicy> for WeldmentJointPolicy {
    fn from(value: AssistantCadWeldmentJointPolicy) -> Self {
        match value {
            AssistantCadWeldmentJointPolicy::Butt => Self::Butt,
            AssistantCadWeldmentJointPolicy::Miter => Self::Miter,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadWeldmentJointPrimary {
    First,
    Second,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCadParameterValueType {
    Length,
    Angle,
    Scalar,
}

impl From<AssistantCadWeldmentJointPrimary> for WeldmentJointPrimary {
    fn from(value: AssistantCadWeldmentJointPrimary) -> Self {
        match value {
            AssistantCadWeldmentJointPrimary::First => Self::First,
            AssistantCadWeldmentJointPrimary::Second => Self::Second,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantCadBodyFeature {
    Boolean {
        operation: AssistantCadBooleanOperation,
        target_feature_id: AssistantCadFeatureReference,
        tool_feature_id: AssistantCadFeatureReference,
    },
    Pocket {
        target_feature_id: u64,
        profile_feature_id: u64,
        depth_mm: f64,
    },
    PlanarOffset {
        profile_feature_id: u64,
        distance_mm: f64,
    },
    Sweep {
        profile_feature_id: u64,
        path_feature_id: u64,
    },
    WeldmentMember {
        profile_feature_id: u64,
        path_feature_id: u64,
        orientation_degrees: f64,
    },
    WeldmentJoint {
        first_member_id: AssistantCadFeatureReference,
        second_member_id: AssistantCadFeatureReference,
        policy: AssistantCadWeldmentJointPolicy,
        primary: AssistantCadWeldmentJointPrimary,
    },
    Loft {
        sections: Vec<AssistantCadLoftSection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guide_feature_id: Option<AssistantCadFeatureReference>,
        #[serde(default)]
        continuity: AssistantCadLoftContinuity,
    },
    SurfaceBody {
        source: AssistantCadSurfaceBodySource,
    },
    SurfaceTrim {
        target_feature_id: AssistantCadFeatureReference,
        cutter_feature_id: AssistantCadFeatureReference,
    },
    SurfaceExtend {
        target_feature_id: AssistantCadFeatureReference,
        distance_mm: f64,
    },
    SurfaceKnit {
        surface_feature_ids: Vec<AssistantCadFeatureReference>,
        tolerance_mm: f64,
        make_solid: bool,
    },
    SurfaceThicken {
        target_feature_id: AssistantCadFeatureReference,
        thickness_mm: f64,
        direction: AssistantCadShellDirection,
    },
    SheetMetal {
        width_mm: f64,
        depth_mm: f64,
        thickness_mm: f64,
        k_factor: f64,
        flanges: Vec<AssistantCadSheetMetalFlange>,
    },
    TopologyShell {
        target_feature_id: u64,
        #[serde(default)]
        removed_face_reference_ids: Vec<String>,
        thickness_mm: f64,
        #[serde(default)]
        direction: AssistantCadShellDirection,
    },
    TopologyFillet {
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        radius_mm: f64,
        #[serde(default)]
        radius_stations: Vec<AssistantCadFilletRadiusStation>,
    },
    TopologyChamfer {
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        distance_mm: f64,
        #[serde(default)]
        mode: AssistantCadChamferMode,
        #[serde(default)]
        side_face_reference_ids: Vec<String>,
    },
}

impl AssistantCadBodyFeature {
    #[must_use]
    pub fn sheet_metal_spec(&self) -> Option<SheetMetalSpec> {
        let Self::SheetMetal {
            width_mm,
            depth_mm,
            thickness_mm,
            k_factor,
            flanges,
        } = self
        else {
            return None;
        };
        Some(SheetMetalSpec {
            width: Dimension::new(width_mm.to_string(), *width_mm).ok()?,
            depth: Dimension::new(depth_mm.to_string(), *depth_mm).ok()?,
            thickness: Dimension::new(thickness_mm.to_string(), *thickness_mm).ok()?,
            k_factor: *k_factor,
            flanges: flanges
                .iter()
                .map(|flange| {
                    Some(SheetMetalFlange {
                        edge: flange.edge.into(),
                        length: Dimension::new(flange.length_mm.to_string(), flange.length_mm)
                            .ok()?,
                        angle_degrees: flange.angle_degrees,
                        inner_radius: Dimension::new(
                            flange.inner_radius_mm.to_string(),
                            flange.inner_radius_mm,
                        )
                        .ok()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        })
    }

    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        match self {
            Self::Boolean {
                target_feature_id,
                tool_feature_id,
                ..
            } if target_feature_id.validate().is_ok()
                && tool_feature_id.validate().is_ok()
                && target_feature_id != tool_feature_id =>
            {
                Ok(())
            }
            Self::Boolean { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::Pocket {
                target_feature_id,
                profile_feature_id,
                depth_mm,
            } if *target_feature_id != 0
                && *profile_feature_id != 0
                && target_feature_id != profile_feature_id
                && depth_mm.is_finite()
                && *depth_mm > 0.0
                && *depth_mm <= MAX_COORDINATE_MM =>
            {
                Ok(())
            }
            Self::Pocket { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::PlanarOffset {
                profile_feature_id,
                distance_mm,
            } if *profile_feature_id != 0
                && distance_mm.is_finite()
                && distance_mm.abs() >= EXACT_MIN_LENGTH_MM
                && distance_mm.abs() <= MAX_COORDINATE_MM =>
            {
                Ok(())
            }
            Self::PlanarOffset { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::Sweep {
                profile_feature_id,
                path_feature_id,
            } if *profile_feature_id != 0
                && *path_feature_id != 0
                && profile_feature_id != path_feature_id =>
            {
                Ok(())
            }
            Self::Sweep { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::WeldmentMember {
                profile_feature_id,
                path_feature_id,
                orientation_degrees,
            } if *profile_feature_id != 0
                && *path_feature_id != 0
                && profile_feature_id != path_feature_id
                && orientation_degrees.is_finite()
                && (-180.0..180.0).contains(orientation_degrees) =>
            {
                Ok(())
            }
            Self::WeldmentMember { .. } => {
                Err(AssistantRequestInvalid::invalid("CAD weldment member"))
            }
            Self::WeldmentJoint {
                first_member_id,
                second_member_id,
                ..
            } if first_member_id.validate().is_ok()
                && second_member_id.validate().is_ok()
                && first_member_id != second_member_id =>
            {
                Ok(())
            }
            Self::WeldmentJoint { .. } => {
                Err(AssistantRequestInvalid::invalid("CAD weldment joint"))
            }
            Self::Loft {
                sections,
                guide_feature_id,
                continuity,
            } if (2..=16).contains(&sections.len())
                && guide_feature_id.is_none_or(|guide| guide.validate().is_ok())
                && !(guide_feature_id.is_some()
                    && *continuity == AssistantCadLoftContinuity::Curvature)
                && sections.iter().all(|section| {
                    section.profile_feature_id.validate().is_ok()
                        && section.elevation_mm.is_finite()
                        && section.elevation_mm.abs() <= MAX_COORDINATE_MM
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
            Self::Loft { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar { profile_feature_id },
            } if profile_feature_id.validate().is_ok() => Ok(()),
            Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Loft {
                        sections,
                        guide_feature_id,
                        continuity,
                    },
            } if (2..=16).contains(&sections.len())
                && guide_feature_id.is_none_or(|guide| guide.validate().is_ok())
                && !(guide_feature_id.is_some()
                    && *continuity == AssistantCadLoftContinuity::Curvature)
                && sections.iter().all(|section| {
                    section.profile_feature_id.validate().is_ok()
                        && section.elevation_mm.is_finite()
                        && section.elevation_mm.abs() <= MAX_COORDINATE_MM
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
            Self::SurfaceBody { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::SurfaceTrim {
                target_feature_id,
                cutter_feature_id,
            } if target_feature_id.validate().is_ok()
                && cutter_feature_id.validate().is_ok()
                && target_feature_id != cutter_feature_id =>
            {
                Ok(())
            }
            Self::SurfaceTrim { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::SurfaceExtend {
                target_feature_id,
                distance_mm,
            } if target_feature_id.validate().is_ok()
                && distance_mm.is_finite()
                && (EXACT_MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(distance_mm) =>
            {
                Ok(())
            }
            Self::SurfaceExtend { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::SurfaceKnit {
                surface_feature_ids,
                tolerance_mm,
                ..
            } if (2..=256).contains(&surface_feature_ids.len())
                && surface_feature_ids
                    .iter()
                    .all(|reference| reference.validate().is_ok())
                && surface_feature_ids.iter().collect::<BTreeSet<_>>().len()
                    == surface_feature_ids.len()
                && tolerance_mm.is_finite()
                && (DEFAULT_LINEAR_TOLERANCE_MM..=10.0).contains(tolerance_mm) =>
            {
                Ok(())
            }
            Self::SurfaceKnit { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::SurfaceThicken {
                target_feature_id,
                thickness_mm,
                ..
            } if target_feature_id.validate().is_ok()
                && thickness_mm.is_finite()
                && (EXACT_MIN_LENGTH_MM..=100_000.0).contains(thickness_mm) =>
            {
                Ok(())
            }
            Self::SurfaceThicken { .. } => {
                Err(AssistantRequestInvalid::invalid("CAD body feature"))
            }
            Self::SheetMetal { .. } => self
                .sheet_metal_spec()
                .filter(|spec| spec.validate().is_ok())
                .map(|_| ())
                .ok_or(AssistantRequestInvalid::invalid("CAD sheet-metal feature")),
            Self::TopologyShell {
                target_feature_id,
                removed_face_reference_ids,
                thickness_mm,
                ..
            } if *target_feature_id != 0
                && removed_face_reference_ids.len() <= 64
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
            Self::TopologyShell { .. } => Err(AssistantRequestInvalid::invalid("CAD body feature")),
            Self::TopologyFillet {
                target_feature_id,
                edge_reference_ids,
                radius_mm,
                radius_stations,
            } if *target_feature_id != 0
                && (1..=64).contains(&edge_reference_ids.len())
                && edge_reference_ids.iter().all(|reference_id| {
                    reference_id.len() == 64
                        && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                && edge_reference_ids.iter().collect::<BTreeSet<_>>().len()
                    == edge_reference_ids.len()
                && radius_mm.is_finite()
                && (0.01..=100_000.0).contains(radius_mm)
                && radius_stations.len() <= 32
                && (radius_stations.is_empty()
                    || radius_stations
                        .last()
                        .is_some_and(|station| station.position == 1.0))
                && radius_stations.iter().all(|station| {
                    station.position.is_finite()
                        && station.position > 0.0
                        && station.position <= 1.0
                        && station.radius_mm.is_finite()
                        && (0.01..=100_000.0).contains(&station.radius_mm)
                })
                && radius_stations
                    .windows(2)
                    .all(|pair| pair[0].position < pair[1].position) =>
            {
                Ok(())
            }
            Self::TopologyFillet { .. } => {
                Err(AssistantRequestInvalid::invalid("CAD body feature"))
            }
            Self::TopologyChamfer {
                target_feature_id,
                edge_reference_ids,
                distance_mm,
                mode,
                side_face_reference_ids,
            } if *target_feature_id != 0
                && (1..=64).contains(&edge_reference_ids.len())
                && edge_reference_ids.iter().all(|reference_id| {
                    reference_id.len() == 64
                        && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                && edge_reference_ids.iter().collect::<BTreeSet<_>>().len()
                    == edge_reference_ids.len()
                && distance_mm.is_finite()
                && (0.01..=100_000.0).contains(distance_mm)
                && match mode {
                    AssistantCadChamferMode::Symmetric => side_face_reference_ids.is_empty(),
                    AssistantCadChamferMode::TwoDistance { second_distance_mm } => {
                        second_distance_mm.is_finite()
                            && (0.01..=100_000.0).contains(second_distance_mm)
                            && side_face_reference_ids.len() == edge_reference_ids.len()
                            && side_face_reference_ids.iter().all(|reference_id| {
                                reference_id.len() == 64
                                    && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                            })
                    }
                    AssistantCadChamferMode::DistanceAngle { angle_degrees } => {
                        angle_degrees.is_finite()
                            && *angle_degrees > 0.1
                            && *angle_degrees < 89.9
                            && side_face_reference_ids.len() == edge_reference_ids.len()
                            && side_face_reference_ids.iter().all(|reference_id| {
                                reference_id.len() == 64
                                    && reference_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                            })
                    }
                } =>
            {
                Ok(())
            }
            Self::TopologyChamfer { .. } => {
                Err(AssistantRequestInvalid::invalid("CAD body feature"))
            }
        }
    }

    pub fn produces_body_feature_output(&self) -> bool {
        !matches!(self, Self::PlanarOffset { .. })
    }

    fn validate_program_references(
        &self,
        operation_index: usize,
        operations: &[AssistantCadEditOperation],
    ) -> Result<(), AssistantRequestInvalid> {
        let validate_reference = |reference: AssistantCadFeatureReference| match reference {
            AssistantCadFeatureReference::Existing(_) => Ok(()),
            AssistantCadFeatureReference::ProgramOutput(reference) => reference.validate_for(
                operation_index,
                operations,
                AssistantCadProgramFeatureOutput::BodyFeature,
            ),
        };
        match self {
            Self::Boolean {
                target_feature_id,
                tool_feature_id,
                ..
            } => {
                validate_reference(*target_feature_id)?;
                validate_reference(*tool_feature_id)?;
            }
            Self::WeldmentJoint {
                first_member_id,
                second_member_id,
                ..
            } => {
                validate_reference(*first_member_id)?;
                validate_reference(*second_member_id)?;
            }
            Self::Loft {
                sections,
                guide_feature_id,
                ..
            }
            | Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Loft {
                        sections,
                        guide_feature_id,
                        ..
                    },
            } => {
                for section in sections {
                    if let AssistantCadFeatureReference::ProgramOutput(reference) =
                        section.profile_feature_id
                    {
                        reference.validate_for(
                            operation_index,
                            operations,
                            AssistantCadProgramFeatureOutput::SketchFeature,
                        )?;
                    }
                }
                if let Some(AssistantCadFeatureReference::ProgramOutput(reference)) =
                    guide_feature_id
                {
                    reference.validate_for(
                        operation_index,
                        operations,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    )?;
                }
            }
            Self::SurfaceBody {
                source:
                    AssistantCadSurfaceBodySource::Planar {
                        profile_feature_id: AssistantCadFeatureReference::ProgramOutput(reference),
                    },
            } => {
                reference.validate_for(
                    operation_index,
                    operations,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                )?;
            }
            Self::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar { .. },
            } => {}
            Self::SurfaceTrim {
                target_feature_id,
                cutter_feature_id,
            } => {
                validate_reference(*target_feature_id)?;
                validate_reference(*cutter_feature_id)?;
            }
            Self::SurfaceExtend {
                target_feature_id, ..
            }
            | Self::SurfaceThicken {
                target_feature_id, ..
            } => validate_reference(*target_feature_id)?,
            Self::SurfaceKnit {
                surface_feature_ids,
                ..
            } => {
                for reference in surface_feature_ids {
                    validate_reference(*reference)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantSketchPointKind {
    Start,
    End,
    Center,
    Control1,
    Control2,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSketchPointRef {
    pub entity_id: u64,
    pub point: AssistantSketchPointKind,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSketchProfileCopy {
    pub entity_ids: Vec<u64>,
    pub translation_mm: [f64; 2],
    pub rotation_degrees: f64,
    pub uniform_scale: f64,
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
    Ellipse {
        segment_ids: [u64; 4],
        center_mm: [f64; 2],
        radius_x_mm: f64,
        radius_y_mm: f64,
        rotation_degrees: f64,
        maximum_deviation_mm: f64,
    },
    RoundedRectangle {
        segment_ids: [u64; 8],
        center_mm: [f64; 2],
        width_mm: f64,
        height_mm: f64,
        corner_radius_mm: f64,
        rotation_degrees: f64,
    },
    ProfileCopies {
        source_entities: Vec<AssistantSketchEntity>,
        copies: Vec<AssistantSketchProfileCopy>,
    },
    CubicBezier {
        id: u64,
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCadClassificationCategory {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCamToolKind {
    FlatEndMill,
    BallEndMill,
    Drill,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCamWorkOffset {
    G54,
    G55,
    G56,
    G57,
    G58,
    G59,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantFeaReviewRequest {
    pub definition_id: u64,
    pub feature_id: u64,
    pub occurrence_id: u64,
    pub case_id: String,
    pub youngs_modulus_mpa: f64,
    pub poisson_ratio: f64,
    pub yield_strength_mpa: f64,
    pub constrained_face_ordinals: Vec<u32>,
    pub loaded_face_ordinal: u32,
    pub traction_local_n_per_mm2: [f64; 3],
    pub coarse_deflection_mm: f64,
    pub fine_deflection_mm: f64,
}

impl AssistantFeaReviewRequest {
    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        let unique_constraints = self
            .constrained_face_ordinals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if self.definition_id == 0
            || self.feature_id == 0
            || self.occurrence_id == 0
            || self.case_id.trim().is_empty()
            || self.case_id.len() > MAX_ASSISTANT_NAME_BYTES
            || self.case_id.chars().any(char::is_control)
            || !self.youngs_modulus_mpa.is_finite()
            || self.youngs_modulus_mpa <= 0.0
            || self.youngs_modulus_mpa > 1.0e9
            || !self.poisson_ratio.is_finite()
            || !(-1.0..0.5).contains(&self.poisson_ratio)
            || !self.yield_strength_mpa.is_finite()
            || self.yield_strength_mpa <= 0.0
            || self.yield_strength_mpa > 1.0e9
            || self.constrained_face_ordinals.is_empty()
            || self.constrained_face_ordinals.len() > 64
            || unique_constraints.len() != self.constrained_face_ordinals.len()
            || unique_constraints.contains(&self.loaded_face_ordinal)
            || !self
                .traction_local_n_per_mm2
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1.0e6)
            || !assistant_cad_vector_is_nonzero(self.traction_local_n_per_mm2)
            || !self.coarse_deflection_mm.is_finite()
            || !self.fine_deflection_mm.is_finite()
            || self.fine_deflection_mm <= 0.0
            || self.coarse_deflection_mm <= self.fine_deflection_mm
            || self.coarse_deflection_mm > MAX_COORDINATE_MM
        {
            return Err(AssistantRequestInvalid::invalid("FEA review request"));
        }
        Ok(())
    }
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
    CreateProgramSketch {
        definition: AssistantCadProgramFeatureReference,
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
    CreatePanel {
        name: String,
        dimensions_mm: [f64; 3],
        holes: Vec<AssistantPanelHole>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pockets: Vec<AssistantPanelPocket>,
        translation_mm: [f64; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rotation: Option<AssistantCadRotation>,
    },
    CreateSpatialPath {
        name: String,
        segments: Vec<AssistantSpatialPathSegment>,
    },
    CreateConstructionPoint {
        name: String,
        position_mm: [f64; 3],
    },
    CreateConstructionAxis {
        name: String,
        origin_mm: [f64; 3],
        direction: [f64; 3],
    },
    CreateConstructionPlane {
        name: String,
        origin_mm: [f64; 3],
        normal: [f64; 3],
        x_direction: [f64; 3],
    },
    /// A helix around an axis. Without a profile it is a path other features can use;
    /// with one, that closed profile (drawn in the plane across the helix start) is swept
    /// along it into a body: a wire, a spring, a thread of any cross-section.
    CreateHelix {
        name: String,
        parameters: AssistantHelixParameters,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        profile: Vec<AssistantSketchEntity>,
    },
    FilletEdges {
        definition_id: u64,
        name: String,
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        radius_mm: f64,
    },
    ChamferEdges {
        definition_id: u64,
        name: String,
        target_feature_id: u64,
        edge_reference_ids: Vec<String>,
        distance_mm: f64,
    },
    AppendFeature {
        definition_id: u64,
        name: String,
        feature: AssistantCadBodyFeature,
    },
    AppendProgramPocket {
        definition: AssistantCadProgramFeatureReference,
        name: String,
        target_feature: AssistantCadProgramFeatureReference,
        profile_feature: AssistantCadProgramFeatureReference,
        depth_mm: f64,
    },
    BindProgramOutput {
        name: String,
        source: AssistantCadProgramFeatureReference,
    },
    SetDimension {
        feature_id: u64,
        constraint_id: Option<u64>,
        value_mm: f64,
    },
    SetFeatureParameter {
        feature_id: u64,
        parameter_path: String,
        value_type: AssistantCadParameterValueType,
        value: f64,
    },
    MakeOccurrenceUnique {
        occurrence_id: u64,
    },
    CreateAssemblyJoint {
        parent_instance_path: AssistantInstancePath,
        child_instance_path: AssistantInstancePath,
        kind: AssistantAssemblyJointKind,
    },
    CreatePinJoint {
        name: String,
        first: AssistantPinJointFace,
        second: AssistantPinJointFace,
        first_center_local_mm: [f64; 3],
        row_unit_first_local: [f64; 3],
        count: u32,
        spacing_mm: f64,
        pin: AssistantPin,
        #[serde(default)]
        physical_hole_pairs: Vec<AssistantPinPhysicalHolePair>,
    },
    CreateProgramPinJoint {
        name: String,
        first: AssistantProgramPinJointFace,
        second: AssistantProgramPinJointFace,
        first_center_local_mm: [f64; 3],
        row_unit_first_local: [f64; 3],
        count: u32,
        spacing_mm: f64,
        pin: AssistantPin,
        physical_hole_pairs: Vec<AssistantProgramPinPhysicalHolePair>,
    },
    CreatePhysicalPinJoint {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        joint_id: Option<u64>,
        name: String,
        first: AssistantPinJointFace,
        second: AssistantPinJointFace,
        first_center_local_mm: [f64; 3],
        row_unit_first_local: [f64; 3],
        count: u32,
        spacing_mm: f64,
        pin: AssistantPin,
        /// Pin length inserted into the first part; the rest goes into the
        /// second. Omitted means half each. Lets a pin go shallow into a thin
        /// board face and deep into the mating board's end.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_insertion_mm: Option<f64>,
    },
    DeletePhysicalPinJoint {
        joint_id: u64,
    },
    MovePhysicalPinPair {
        joint_id: u64,
        pair_index: u32,
        offset_first_local_mm: [f64; 3],
    },
    SetAssemblyJointPosition {
        joint_id: u64,
        position: f64,
    },
    CreateDrawing {
        name: String,
        instance_paths: Vec<AssistantInstancePath>,
    },
    UpsertCamPlan {
        plan_id: u64,
        name: String,
        target_definition_id: u64,
        target_feature_id: u64,
        stock_minimum_mm: [f64; 3],
        stock_maximum_mm: [f64; 3],
        tool_number: u32,
        tool_kind: AssistantCamToolKind,
        tool_diameter_mm: f64,
        flute_length_mm: f64,
        overall_length_mm: f64,
        holder_diameter_mm: f64,
        holder_length_mm: f64,
        spindle_rpm: u32,
        feed_mm_per_min: f64,
        plunge_mm_per_min: f64,
        work_offset: AssistantCamWorkOffset,
        origin_mm: [f64; 3],
        x_axis: [f64; 3],
        y_axis: [f64; 3],
        safe_height_mm: f64,
        maximum_stepdown_mm: f64,
        stepover_ratio: f64,
        radial_allowance_mm: f64,
        axial_allowance_mm: f64,
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
    SetColor {
        selector: AssistantCadEntitySelector,
        color: Option<[u8; 3]>,
    },
    SetGrounded {
        selector: AssistantCadEntitySelector,
        grounded: bool,
    },
    CreateTag {
        tag_id: u64,
        name: String,
        visible: bool,
    },
    SetOccurrenceTag {
        selector: AssistantCadEntitySelector,
        tag_id: Option<u64>,
    },
    SetTagVisibility {
        tag_id: u64,
        visible: bool,
    },
    UpsertClassificationDimension {
        dimension_id: u64,
        name: String,
        categories: Vec<AssistantCadClassificationCategory>,
    },
    SetOccurrenceClassification {
        selector: AssistantCadEntitySelector,
        dimension_id: u64,
        category_id: Option<u64>,
    },
    CreateEvaluatorInput {
        node_id: u64,
        name: String,
        value: f64,
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
    CircularPattern {
        selector: AssistantCadEntitySelector,
        instances: u32,
        axis: AssistantAxisSpec,
        angle_step_degrees: f64,
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
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
}

fn assistant_cad_vector_is_nonzero(vector: [f64; 3]) -> bool {
    vector.iter().any(|value| value.abs() > f64::EPSILON)
}

fn assistant_cad_vectors_are_perpendicular(left: [f64; 3], right: [f64; 3]) -> bool {
    let left_length_squared = left.iter().map(|value| value * value).sum::<f64>();
    let right_length_squared = right.iter().map(|value| value * value).sum::<f64>();
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f64>();
    dot.is_finite() && dot.abs() <= ROUNDING * (left_length_squared * right_length_squared).sqrt()
}

impl AssistantInstancePath {
    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        if self.root_occurrence_id == 0
            || self.steps.len() > MAX_ASSISTANT_INSTANCE_PATH_STEPS
            || self.steps.iter().any(|step| match step {
                AssistantInstancePathStep::Group {
                    owner_definition_id,
                    local_id,
                }
                | AssistantInstancePathStep::Occurrence {
                    owner_definition_id,
                    local_id,
                } => *owner_definition_id == 0 || *local_id == 0,
            })
        {
            return Err(AssistantRequestInvalid::invalid("instance path"));
        }
        Ok(())
    }
}

impl AssistantAssemblyJointAxis {
    fn validate(self) -> Result<(), AssistantRequestInvalid> {
        if !assistant_cad_vector_is_bounded(self.direction_in_parent)
            || !assistant_cad_vector_is_nonzero(self.direction_in_parent)
            || !assistant_cad_vector_is_bounded(self.pivot_in_parent_mm)
        {
            return Err(AssistantRequestInvalid::invalid("assembly joint axis"));
        }
        Ok(())
    }
}

impl AssistantAssemblyJointLimits {
    fn validate(self, position: f64) -> Result<(), AssistantRequestInvalid> {
        if !self.min.is_finite()
            || !self.max.is_finite()
            || self.min > self.max
            || self.min.abs() > MAX_COORDINATE_MM
            || self.max.abs() > MAX_COORDINATE_MM
            || position < self.min
            || position > self.max
        {
            return Err(AssistantRequestInvalid::invalid(
                "assembly joint limit range",
            ));
        }
        Ok(())
    }
}

impl AssistantAssemblyJointKind {
    fn validate(self) -> Result<(), AssistantRequestInvalid> {
        let validate_motion = |axis: AssistantAssemblyJointAxis,
                               limits: Option<AssistantAssemblyJointLimits>,
                               position: f64| {
            axis.validate()?;
            if !position.is_finite() || position.abs() > MAX_COORDINATE_MM {
                return Err(AssistantRequestInvalid::invalid("assembly joint position"));
            }
            if let Some(limits) = limits {
                limits.validate(position)?;
            }
            Ok(())
        };
        match self {
            Self::Fixed => Ok(()),
            Self::Revolute {
                axis,
                limits,
                position_degrees,
            } => validate_motion(axis, limits, position_degrees),
            Self::Prismatic {
                axis,
                limits,
                position_mm,
            } => validate_motion(axis, limits, position_mm),
            Self::Helical {
                axis,
                limits,
                lead_mm_per_revolution,
                position_degrees,
            } => {
                validate_motion(axis, limits, position_degrees)?;
                if !lead_mm_per_revolution.is_finite()
                    || lead_mm_per_revolution <= 0.0
                    || lead_mm_per_revolution > MAX_COORDINATE_MM
                {
                    return Err(AssistantRequestInvalid::invalid("assembly helical joint"));
                }
                Ok(())
            }
        }
    }
}

impl AssistantCadEntitySelector {
    fn bounded_target_count(&self) -> Result<usize, AssistantRequestInvalid> {
        match self {
            Self::CurrentSelection {} => Ok(MAX_ASSISTANT_CAD_SELECTOR_TARGETS),
            Self::Occurrences { occurrence_ids } => {
                let unique = occurrence_ids.iter().copied().collect::<BTreeSet<_>>();
                if occurrence_ids.is_empty()
                    || occurrence_ids.len() > MAX_ASSISTANT_CAD_SELECTOR_TARGETS
                    || unique.len() != occurrence_ids.len()
                    || occurrence_ids.contains(&0)
                {
                    return Err(AssistantRequestInvalid::invalid("CAD selector"));
                }
                Ok(occurrence_ids.len())
            }
        }
    }

    pub fn validate_resolved_target_count(
        &self,
        target_count: usize,
    ) -> Result<(), AssistantRequestInvalid> {
        self.bounded_target_count()?;
        if target_count == 0 || target_count > MAX_ASSISTANT_CAD_SELECTOR_TARGETS {
            return Err(AssistantRequestInvalid::invalid(
                "CAD resolved selector target count",
            ));
        }
        Ok(())
    }
}

impl AssistantCadRotation {
    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        let axis_length_squared = self.axis.iter().map(|value| value * value).sum::<f64>();
        let normalized_angle = self.angle_degrees.rem_euclid(360.0);
        let shortest_angle = normalized_angle.min(360.0 - normalized_angle);
        if !assistant_cad_vector_is_bounded(self.pivot_mm)
            || !assistant_cad_vector_is_bounded(self.axis)
            || !axis_length_squared.is_finite()
            || axis_length_squared <= f64::EPSILON
            || !self.angle_degrees.is_finite()
            || self.angle_degrees.abs() > MAX_COORDINATE_MM
            || shortest_angle < 0.01
        {
            return Err(AssistantRequestInvalid::invalid("CAD rotation"));
        }
        Ok(())
    }
}

impl AssistantWorkplaneSpec {
    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        match self {
            Self::Frame {
                origin_mm,
                x_axis,
                y_axis,
            } => ketchup_geometry::sketch::WorkplaneFrame::from_axes(*origin_mm, *x_axis, *y_axis)
                .map(|_| ())
                .map_err(|error| {
                    AssistantRequestInvalid::invalid("workplane frame").caused_by(error)
                }),
            Self::Principal { .. } => Ok(()),
            Self::Offset {
                base_feature_id,
                distance_mm,
            } if *base_feature_id != 0
                && distance_mm.is_finite()
                && distance_mm.abs() <= MAX_COORDINATE_MM =>
            {
                Ok(())
            }
            Self::Offset { .. } => Err(AssistantRequestInvalid::invalid("workplane")),
            Self::ConstructionPlane { plane } => plane.validate().map_err(|error| {
                AssistantRequestInvalid::invalid("construction-plane workplane").caused_by(error)
            }),
        }
    }
}

impl AssistantSketchEntity {
    fn id(&self) -> u64 {
        match self {
            Self::Line { id, .. }
            | Self::Arc { id, .. }
            | Self::Circle { id, .. }
            | Self::CubicBezier { id, .. } => *id,
            Self::Ellipse { segment_ids, .. } => segment_ids[0],
            Self::RoundedRectangle { segment_ids, .. } => segment_ids[0],
            Self::ProfileCopies { copies, .. } => copies
                .first()
                .and_then(|copy| copy.entity_ids.first())
                .copied()
                .unwrap_or(0),
        }
    }

    fn ids(&self) -> Vec<u64> {
        match self {
            Self::Ellipse { segment_ids, .. } => segment_ids.to_vec(),
            Self::RoundedRectangle { segment_ids, .. } => segment_ids.to_vec(),
            Self::ProfileCopies { copies, .. } => copies
                .iter()
                .flat_map(|copy| copy.entity_ids.iter().copied())
                .collect(),
            _ => vec![self.id()],
        }
    }

    fn supports_point(&self, point: AssistantSketchPointKind) -> bool {
        matches!(
            (self, point),
            (
                Self::Line { .. } | Self::Arc { .. } | Self::CubicBezier { .. },
                AssistantSketchPointKind::Start | AssistantSketchPointKind::End
            ) | (
                Self::Arc { .. } | Self::Circle { .. },
                AssistantSketchPointKind::Center
            ) | (
                Self::CubicBezier { .. },
                AssistantSketchPointKind::Control1 | AssistantSketchPointKind::Control2
            )
        )
    }

    fn is_line(&self) -> bool {
        matches!(self, Self::Line { .. })
    }

    fn is_circular(&self) -> bool {
        matches!(self, Self::Arc { .. } | Self::Circle { .. })
    }

    fn ellipse_approximation_is_within_tolerance(&self, uniform_scale: f64) -> bool {
        match self {
            Self::Ellipse {
                radius_x_mm,
                radius_y_mm,
                maximum_deviation_mm,
                ..
            } => {
                uniform_scale
                    * radius_x_mm.max(*radius_y_mm)
                    * ELLIPSE_CUBIC_MAX_NORMALIZED_RADIAL_DEVIATION
                    <= *maximum_deviation_mm
            }
            _ => true,
        }
    }

    fn radial_bound(&self) -> f64 {
        let radius = |point: [f64; 2]| point[0].hypot(point[1]);
        match self {
            Self::Line {
                start_mm, end_mm, ..
            } => radius(*start_mm).max(radius(*end_mm)),
            Self::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                let arc_radius = (start_mm[0] - center_mm[0])
                    .hypot(start_mm[1] - center_mm[1])
                    .max((end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]));
                radius(*center_mm) + arc_radius
            }
            Self::Circle {
                center_mm,
                radius_mm,
                ..
            } => radius(*center_mm) + radius_mm,
            Self::Ellipse {
                center_mm,
                radius_x_mm,
                radius_y_mm,
                ..
            } => radius(*center_mm) + radius_x_mm.max(*radius_y_mm),
            Self::RoundedRectangle {
                center_mm,
                width_mm,
                height_mm,
                ..
            } => radius(*center_mm) + (0.5 * width_mm).hypot(0.5 * height_mm),
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => [*start_mm, *control_1_mm, *control_2_mm, *end_mm]
                .into_iter()
                .map(radius)
                .fold(0.0, f64::max),
            Self::ProfileCopies { .. } => f64::INFINITY,
        }
    }

    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        let point = |point: &[f64; 2]| {
            point
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
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
                    && *radius_mm <= MAX_COORDINATE_MM
            }
            Self::Ellipse {
                segment_ids,
                center_mm,
                radius_x_mm,
                radius_y_mm,
                rotation_degrees,
                maximum_deviation_mm,
            } => {
                segment_ids.iter().all(|id| *id != 0)
                    && segment_ids.iter().collect::<BTreeSet<_>>().len() == segment_ids.len()
                    && point(center_mm)
                    && radius_x_mm.is_finite()
                    && *radius_x_mm > 0.0
                    && *radius_x_mm <= MAX_COORDINATE_MM
                    && radius_y_mm.is_finite()
                    && *radius_y_mm > 0.0
                    && *radius_y_mm <= MAX_COORDINATE_MM
                    && rotation_degrees.is_finite()
                    && maximum_deviation_mm.is_finite()
                    && *maximum_deviation_mm > 0.0
                    && *maximum_deviation_mm <= MAX_COORDINATE_MM
                    && self.ellipse_approximation_is_within_tolerance(1.0)
            }
            Self::RoundedRectangle {
                segment_ids,
                center_mm,
                width_mm,
                height_mm,
                corner_radius_mm,
                rotation_degrees,
            } => {
                segment_ids.iter().all(|id| *id != 0)
                    && segment_ids.iter().collect::<BTreeSet<_>>().len() == segment_ids.len()
                    && point(center_mm)
                    && width_mm.is_finite()
                    && *width_mm > 0.0
                    && *width_mm <= MAX_COORDINATE_MM
                    && height_mm.is_finite()
                    && *height_mm > 0.0
                    && *height_mm <= MAX_COORDINATE_MM
                    && corner_radius_mm.is_finite()
                    && *corner_radius_mm > 0.0
                    && *corner_radius_mm < 0.5 * width_mm.min(*height_mm)
                    && rotation_degrees.is_finite()
                    && center_mm[0].abs() + 0.5 * width_mm + 0.5 * height_mm <= MAX_COORDINATE_MM
                    && center_mm[1].abs() + 0.5 * width_mm + 0.5 * height_mm <= MAX_COORDINATE_MM
            }
            Self::ProfileCopies {
                source_entities,
                copies,
            } => {
                let source_ids = source_entities
                    .iter()
                    .flat_map(Self::ids)
                    .collect::<Vec<_>>();
                let source_bound = source_entities
                    .iter()
                    .map(Self::radial_bound)
                    .fold(0.0, f64::max);
                !source_entities.is_empty()
                    && !source_entities
                        .iter()
                        .any(|entity| matches!(entity, Self::ProfileCopies { .. }))
                    && source_entities
                        .iter()
                        .all(|entity| entity.validate().is_ok())
                    && source_ids.len() <= ketchup_geometry::sketch::MAX_SKETCH_ENTITIES
                    && source_ids.iter().collect::<BTreeSet<_>>().len() == source_ids.len()
                    && (1..=MAX_ASSISTANT_PROFILE_COPIES).contains(&copies.len())
                    && copies.iter().all(|copy| {
                        copy.entity_ids.len() == source_ids.len()
                            && copy.entity_ids.iter().all(|id| *id != 0)
                            && copy.entity_ids.iter().collect::<BTreeSet<_>>().len()
                                == copy.entity_ids.len()
                            && point(&copy.translation_mm)
                            && copy.rotation_degrees.is_finite()
                            && copy.uniform_scale.is_finite()
                            && copy.uniform_scale > 0.0
                            && copy.uniform_scale <= 1_000.0
                            && source_entities.iter().all(|entity| {
                                entity.ellipse_approximation_is_within_tolerance(copy.uniform_scale)
                            })
                            && copy.translation_mm[0].hypot(copy.translation_mm[1])
                                + copy.uniform_scale * source_bound
                                <= MAX_COORDINATE_MM
                    })
            }
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => point(start_mm) && point(control_1_mm) && point(control_2_mm) && point(end_mm),
        };
        if self.id() == 0 || !valid {
            return Err(AssistantRequestInvalid::invalid("sketch entity"));
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

    fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        let valid_point_ref = |point: &AssistantSketchPointRef| point.entity_id != 0;
        let valid_point = |point: &[f64; 2]| {
            point
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
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
                    && *value_mm <= MAX_COORDINATE_MM
            }
            Self::Radius {
                entity_id,
                value_mm,
                ..
            } => {
                *entity_id != 0
                    && value_mm.is_finite()
                    && *value_mm > 0.0
                    && *value_mm <= MAX_COORDINATE_MM
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
            return Err(AssistantRequestInvalid::invalid("sketch constraint"));
        }
        Ok(())
    }

    fn validate_references(
        &self,
        entities: &BTreeMap<u64, &AssistantSketchEntity>,
    ) -> Result<(), AssistantRequestInvalid> {
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
            Err(AssistantRequestInvalid::invalid(
                "sketch constraint reference",
            ))
        }
    }
}

/// Checks each entity and that no two share an ID, within the sketch entity limit.
fn assistant_sketch_entities_by_id(
    entities: &[AssistantSketchEntity],
) -> Result<BTreeMap<u64, &AssistantSketchEntity>, AssistantRequestInvalid> {
    let expanded_entity_count = entities
        .iter()
        .map(|entity| entity.ids().len())
        .sum::<usize>();
    if expanded_entity_count > ketchup_geometry::sketch::MAX_SKETCH_ENTITIES {
        return Err(AssistantRequestInvalid::invalid("sketch entity count"));
    }
    let mut entities_by_id = BTreeMap::new();
    for entity in entities {
        entity.validate()?;
        for id in entity.ids() {
            if entities_by_id.insert(id, entity).is_some() {
                return Err(AssistantRequestInvalid::new(
                    "sketch entity ID",
                    AssistantRequestProblem::Duplicate,
                )
                .item(id));
            }
        }
    }
    Ok(entities_by_id)
}

/// A swept profile is sketch entities joined end to start into one closed loop; the
/// document checks the loop itself when the profile feature is created.
fn validate_assistant_profile_entities(
    entities: &[AssistantSketchEntity],
) -> Result<(), AssistantRequestInvalid> {
    if entities
        .iter()
        .any(|entity| matches!(entity, AssistantSketchEntity::ProfileCopies { .. }))
    {
        return Err(AssistantRequestInvalid::invalid("swept profile"));
    }
    assistant_sketch_entities_by_id(entities).map(|_| ())
}

fn validate_assistant_sketch_payload(
    name: &str,
    workplane: &AssistantWorkplaneSpec,
    entities: &[AssistantSketchEntity],
    constraints: &[AssistantSketchConstraint],
) -> Result<(), AssistantRequestInvalid> {
    if name.trim().is_empty()
        || name.len() > MAX_ASSISTANT_NAME_BYTES
        || name.chars().any(char::is_control)
        || entities.is_empty()
        || constraints.len() > ketchup_geometry::sketch::MAX_SKETCH_CONSTRAINTS
    {
        return Err(AssistantRequestInvalid::invalid("sketch creation"));
    }
    workplane.validate()?;
    let entities_by_id = assistant_sketch_entities_by_id(entities)?;
    let mut constraint_ids = BTreeSet::new();
    for constraint in constraints {
        constraint.validate()?;
        constraint.validate_references(&entities_by_id)?;
        if !constraint_ids.insert(constraint.id()) {
            return Err(AssistantRequestInvalid::new(
                "sketch constraint ID",
                AssistantRequestProblem::Duplicate,
            )
            .item(constraint.id()));
        }
    }
    Ok(())
}

impl AssistantCadEditProgram {
    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        if self.operations.is_empty() || self.operations.len() > MAX_ASSISTANT_CAD_EDIT_OPERATIONS {
            return Err(AssistantRequestInvalid::invalid(
                "CAD edit program operation count",
            ));
        }
        let mut generated_occurrences = 0usize;
        let mut named_outputs = BTreeMap::<String, AssistantCadProgramFeatureOutput>::new();
        for (operation_index, operation) in self.operations.iter().enumerate() {
            let bounded_targets = match operation {
                AssistantCadEditOperation::CreateSketch { .. }
                | AssistantCadEditOperation::CreateProgramSketch { .. }
                | AssistantCadEditOperation::AppendFeature { .. }
                | AssistantCadEditOperation::FilletEdges { .. }
                | AssistantCadEditOperation::ChamferEdges { .. }
                | AssistantCadEditOperation::AppendProgramPocket { .. }
                | AssistantCadEditOperation::BindProgramOutput { .. }
                | AssistantCadEditOperation::SetDimension { .. }
                | AssistantCadEditOperation::SetFeatureParameter { .. }
                | AssistantCadEditOperation::MakeOccurrenceUnique { .. }
                | AssistantCadEditOperation::CreateAssemblyJoint { .. }
                | AssistantCadEditOperation::CreatePinJoint { .. }
                | AssistantCadEditOperation::CreateProgramPinJoint { .. }
                | AssistantCadEditOperation::CreatePhysicalPinJoint { .. }
                | AssistantCadEditOperation::DeletePhysicalPinJoint { .. }
                | AssistantCadEditOperation::MovePhysicalPinPair { .. }
                | AssistantCadEditOperation::SetAssemblyJointPosition { .. }
                | AssistantCadEditOperation::CreateDrawing { .. }
                | AssistantCadEditOperation::UpsertCamPlan { .. }
                | AssistantCadEditOperation::CreateTag { .. }
                | AssistantCadEditOperation::SetTagVisibility { .. }
                | AssistantCadEditOperation::UpsertClassificationDimension { .. }
                | AssistantCadEditOperation::CreateEvaluatorInput { .. } => 0,
                AssistantCadEditOperation::CreatePart { .. }
                | AssistantCadEditOperation::CreatePanel { .. }
                | AssistantCadEditOperation::CreateSpatialPath { .. }
                | AssistantCadEditOperation::CreateConstructionPoint { .. }
                | AssistantCadEditOperation::CreateConstructionAxis { .. }
                | AssistantCadEditOperation::CreateConstructionPlane { .. }
                | AssistantCadEditOperation::CreateHelix { .. } => 1,
                AssistantCadEditOperation::Delete { selector, .. }
                | AssistantCadEditOperation::SetColor { selector, .. }
                | AssistantCadEditOperation::SetGrounded { selector, .. }
                | AssistantCadEditOperation::SetOccurrenceTag { selector, .. }
                | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
                | AssistantCadEditOperation::Transform { selector, .. }
                | AssistantCadEditOperation::Copy { selector, .. }
                | AssistantCadEditOperation::LinearPattern { selector, .. }
                | AssistantCadEditOperation::CircularPattern { selector, .. }
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
                        return Err(AssistantRequestInvalid::invalid("sketch creation"));
                    }
                    validate_assistant_sketch_payload(name, workplane, entities, constraints)?;
                    if matches!(
                        workplane,
                        AssistantWorkplaneSpec::ConstructionPlane {
                            plane: AssistantCadFeatureReference::ProgramOutput(_)
                        }
                    ) {
                        return Err(AssistantRequestInvalid::invalid(
                            "sketch workplane reference",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::CreateProgramSketch {
                    definition,
                    name,
                    workplane,
                    entities,
                    constraints,
                } => {
                    definition.validate_for(
                        operation_index,
                        &self.operations,
                        AssistantCadProgramFeatureOutput::Definition,
                    )?;
                    if matches!(workplane, AssistantWorkplaneSpec::Offset { .. }) {
                        return Err(AssistantRequestInvalid::invalid(
                            "CAD program Sketch workplane reference",
                        ));
                    }
                    if let AssistantWorkplaneSpec::ConstructionPlane {
                        plane: AssistantCadFeatureReference::ProgramOutput(plane),
                    } = workplane
                    {
                        plane.validate_for(
                            operation_index,
                            &self.operations,
                            AssistantCadProgramFeatureOutput::ConstructionFeature,
                        )?;
                        if !matches!(
                            self.operations.get(plane.operation_index as usize),
                            Some(AssistantCadEditOperation::CreateConstructionPlane { .. })
                        ) {
                            return Err(AssistantRequestInvalid::invalid(
                                "construction-plane workplane reference",
                            ));
                        }
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
                    if matches!(workplane, AssistantWorkplaneSpec::ConstructionPlane { .. }) {
                        return Err(AssistantRequestInvalid::invalid("part workplane reference"));
                    }
                    feature.validate()?;
                    if let AssistantCadPartFeature::Revolve { axis, .. } = feature {
                        axis.validate_reference_for_operation(operation_index, &self.operations)?;
                    }
                    if !assistant_cad_vector_is_bounded(*translation_mm) {
                        return Err(AssistantRequestInvalid::invalid("CAD part placement"));
                    }
                    if let Some(rotation) = rotation {
                        rotation.validate()?;
                    }
                    1
                }
                AssistantCadEditOperation::CreatePanel {
                    name,
                    dimensions_mm,
                    holes,
                    pockets,
                    translation_mm,
                    rotation,
                } => {
                    if pockets.len() > MAX_ASSISTANT_PANEL_POCKETS {
                        return Err(AssistantRequestInvalid::new(
                            "panel pocket count",
                            AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_PANEL_POCKETS),
                        ));
                    }
                    let mut pocket_ids = BTreeSet::new();
                    for pocket in pockets {
                        if !pocket_ids.insert(pocket.id.as_str()) {
                            return Err(AssistantRequestInvalid::new(
                                "panel pocket id",
                                AssistantRequestProblem::Duplicate,
                            )
                            .item(&pocket.id));
                        }
                        pocket.validate(*dimensions_mm)?;
                    }
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || dimensions_mm.iter().any(|value| {
                            !value.is_finite() || *value <= 0.0 || *value > MAX_COORDINATE_MM
                        })
                        || holes.len() > 128
                        || !assistant_cad_vector_is_bounded(*translation_mm)
                    {
                        return Err(AssistantRequestInvalid::invalid("panel creation"));
                    }
                    if let Some(rotation) = rotation {
                        rotation.validate()?;
                    }
                    let mut hole_ids = BTreeSet::new();
                    for hole in holes {
                        let radius = hole.diameter_mm * 0.5;
                        let Some(axis) = hole
                            .inward_unit_local
                            .iter()
                            .position(|value| (value.abs() - 1.0).abs() <= ROUNDING)
                        else {
                            return Err(AssistantRequestInvalid::invalid("panel hole direction"));
                        };
                        if hole.id.trim().is_empty()
                            || hole.id.len() > MAX_ASSISTANT_NAME_BYTES
                            || hole.id.chars().any(char::is_control)
                            || !hole_ids.insert(hole.id.as_str())
                            || !assistant_cad_vector_is_bounded(hole.entry_local_mm)
                            || !assistant_cad_vector_is_bounded(hole.inward_unit_local)
                            || hole
                                .inward_unit_local
                                .iter()
                                .enumerate()
                                .any(|(index, value)| index != axis && value.abs() > ROUNDING)
                            || !hole.diameter_mm.is_finite()
                            || hole.diameter_mm <= 0.0
                            || !hole.depth_mm.is_finite()
                            || hole.depth_mm <= 0.0
                            || hole.depth_mm > dimensions_mm[axis]
                            || (if hole.inward_unit_local[axis] > 0.0 {
                                hole.entry_local_mm[axis].abs()
                            } else {
                                (hole.entry_local_mm[axis] - dimensions_mm[axis]).abs()
                            }) > ROUNDING
                            || (0..3).any(|index| {
                                index != axis
                                    && (hole.entry_local_mm[index] < radius
                                        || hole.entry_local_mm[index]
                                            > dimensions_mm[index] - radius)
                            })
                        {
                            return Err(AssistantRequestInvalid::invalid("panel hole"));
                        }
                    }
                    1
                }
                AssistantCadEditOperation::CreateSpatialPath { name, segments } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err(AssistantRequestInvalid::invalid("spatial path creation"));
                    }
                    // Structural pre-check; the document validates the path again with its own
                    // tolerance when the lowered commands apply.
                    validated_spatial_path_segments(segments, DEFAULT_LINEAR_TOLERANCE_MM)?;
                    1
                }
                AssistantCadEditOperation::CreateConstructionPoint { name, position_mm } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !assistant_cad_vector_is_bounded(*position_mm)
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "construction point creation",
                        ));
                    }
                    1
                }
                AssistantCadEditOperation::CreateConstructionAxis {
                    name,
                    origin_mm,
                    direction,
                } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !assistant_cad_vector_is_bounded(*origin_mm)
                        || !assistant_cad_vector_is_bounded(*direction)
                        || !assistant_cad_vector_is_nonzero(*direction)
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "construction axis creation",
                        ));
                    }
                    1
                }
                AssistantCadEditOperation::CreateConstructionPlane {
                    name,
                    origin_mm,
                    normal,
                    x_direction,
                } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !assistant_cad_vector_is_bounded(*origin_mm)
                        || !assistant_cad_vector_is_bounded(*normal)
                        || !assistant_cad_vector_is_bounded(*x_direction)
                        || !assistant_cad_vector_is_nonzero(*normal)
                        || !assistant_cad_vector_is_nonzero(*x_direction)
                        || !assistant_cad_vectors_are_perpendicular(*normal, *x_direction)
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "construction plane creation",
                        ));
                    }
                    1
                }
                AssistantCadEditOperation::CreateHelix {
                    name,
                    parameters,
                    profile,
                } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err(AssistantRequestInvalid::invalid("Helix creation"));
                    }
                    parameters.validate()?;
                    parameters
                        .axis
                        .validate_reference_for_operation(operation_index, &self.operations)?;
                    validate_assistant_profile_entities(profile)?;
                    1
                }
                AssistantCadEditOperation::FilletEdges {
                    definition_id,
                    name,
                    target_feature_id,
                    edge_reference_ids,
                    radius_mm,
                } => {
                    if *definition_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD Fillet"));
                    }
                    AssistantCadBodyFeature::TopologyFillet {
                        target_feature_id: *target_feature_id,
                        edge_reference_ids: edge_reference_ids.clone(),
                        radius_mm: *radius_mm,
                        radius_stations: Vec::new(),
                    }
                    .validate()?;
                    0
                }
                AssistantCadEditOperation::ChamferEdges {
                    definition_id,
                    name,
                    target_feature_id,
                    edge_reference_ids,
                    distance_mm,
                } => {
                    if *definition_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD Chamfer"));
                    }
                    AssistantCadBodyFeature::TopologyChamfer {
                        target_feature_id: *target_feature_id,
                        edge_reference_ids: edge_reference_ids.clone(),
                        distance_mm: *distance_mm,
                        mode: AssistantCadChamferMode::Symmetric,
                        side_face_reference_ids: Vec::new(),
                    }
                    .validate()?;
                    0
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
                        return Err(AssistantRequestInvalid::invalid("CAD feature append"));
                    }
                    feature.validate()?;
                    feature.validate_program_references(operation_index, &self.operations)?;
                    0
                }
                AssistantCadEditOperation::AppendProgramPocket {
                    definition,
                    name,
                    target_feature,
                    profile_feature,
                    depth_mm,
                } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !depth_mm.is_finite()
                        || *depth_mm <= 0.0
                        || *depth_mm > MAX_COORDINATE_MM
                        || target_feature == profile_feature
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD program Pocket"));
                    }
                    definition.validate_for(
                        operation_index,
                        &self.operations,
                        AssistantCadProgramFeatureOutput::Definition,
                    )?;
                    target_feature.validate_for(
                        operation_index,
                        &self.operations,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                    )?;
                    profile_feature.validate_for(
                        operation_index,
                        &self.operations,
                        AssistantCadProgramFeatureOutput::SketchFeature,
                    )?;
                    0
                }
                AssistantCadEditOperation::BindProgramOutput { name, source } => {
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || named_outputs.contains_key(name)
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "CAD program output binding",
                        ));
                    }
                    source.validate_for(operation_index, &self.operations, source.output)?;
                    named_outputs.insert(name.clone(), source.output);
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
                        || *value_mm > MAX_COORDINATE_MM
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD dimension edit"));
                    }
                    0
                }
                AssistantCadEditOperation::SetFeatureParameter {
                    feature_id,
                    parameter_path,
                    value_type,
                    value,
                } => {
                    if *feature_id == 0
                        || parameter_path.trim().is_empty()
                        || parameter_path.len() > MAX_ASSISTANT_NAME_BYTES
                        || parameter_path.chars().any(char::is_control)
                        || !value.is_finite()
                        || value.abs() > MAX_COORDINATE_MM
                        || (*value_type == AssistantCadParameterValueType::Length && *value <= 0.0)
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "CAD feature parameter edit",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id } => {
                    if *occurrence_id == 0 {
                        return Err(AssistantRequestInvalid::invalid(
                            "occurrence make-unique target",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::CreateAssemblyJoint {
                    parent_instance_path,
                    child_instance_path,
                    kind,
                } => {
                    parent_instance_path.validate()?;
                    child_instance_path.validate()?;
                    kind.validate()?;
                    if parent_instance_path == child_instance_path {
                        return Err(AssistantRequestInvalid::invalid(
                            "assembly joint endpoint pair",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::CreatePinJoint {
                    name,
                    first,
                    second,
                    first_center_local_mm,
                    row_unit_first_local,
                    count,
                    spacing_mm,
                    physical_hole_pairs,
                    ..
                } => {
                    first.instance_path.validate()?;
                    second.instance_path.validate()?;
                    let first_holes = physical_hole_pairs
                        .iter()
                        .map(|pair| pair.first_pocket_feature_id)
                        .collect::<BTreeSet<_>>();
                    let second_holes = physical_hole_pairs
                        .iter()
                        .map(|pair| pair.second_pocket_feature_id)
                        .collect::<BTreeSet<_>>();
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || first.instance_path == second.instance_path
                        || !assistant_cad_vector_is_bounded(first.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(first.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(first.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(first.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(second.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(second.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(second.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(second.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(*first_center_local_mm)
                        || !assistant_cad_vector_is_bounded(*row_unit_first_local)
                        || !assistant_cad_vector_is_nonzero(*row_unit_first_local)
                        || !(1..=128).contains(count)
                        || !spacing_mm.is_finite()
                        || *spacing_mm < 0.0
                        || (!physical_hole_pairs.is_empty()
                            && physical_hole_pairs.len() != *count as usize)
                        || physical_hole_pairs.iter().any(|pair| {
                            pair.first_pocket_feature_id == 0 || pair.second_pocket_feature_id == 0
                        })
                        || first_holes.len() != physical_hole_pairs.len()
                        || second_holes.len() != physical_hole_pairs.len()
                    {
                        return Err(AssistantRequestInvalid::invalid("pin joint creation"));
                    }
                    0
                }
                AssistantCadEditOperation::CreatePhysicalPinJoint {
                    joint_id,
                    name,
                    first,
                    second,
                    first_center_local_mm,
                    row_unit_first_local,
                    count,
                    spacing_mm,
                    first_insertion_mm,
                    ..
                } => {
                    first.instance_path.validate()?;
                    second.instance_path.validate()?;
                    if joint_id == &Some(0)
                        || first_insertion_mm
                            .is_some_and(|value| !value.is_finite() || value <= 0.0)
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || first.instance_path == second.instance_path
                        || !assistant_cad_vector_is_bounded(first.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(first.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(first.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(first.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(second.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(second.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(second.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(second.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(*first_center_local_mm)
                        || !assistant_cad_vector_is_bounded(*row_unit_first_local)
                        || !assistant_cad_vector_is_nonzero(*row_unit_first_local)
                        || !(1..=128).contains(count)
                        || !spacing_mm.is_finite()
                        || *spacing_mm < 0.0
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "physical pin joint creation",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::DeletePhysicalPinJoint { joint_id } => {
                    if *joint_id == 0 {
                        return Err(AssistantRequestInvalid::invalid(
                            "physical pin joint deletion",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::MovePhysicalPinPair {
                    joint_id,
                    offset_first_local_mm,
                    ..
                } => {
                    if *joint_id == 0 || !assistant_cad_vector_is_bounded(*offset_first_local_mm) {
                        return Err(AssistantRequestInvalid::invalid("physical pin pair move"));
                    }
                    0
                }
                AssistantCadEditOperation::CreateProgramPinJoint {
                    name,
                    first,
                    second,
                    first_center_local_mm,
                    row_unit_first_local,
                    count,
                    spacing_mm,
                    physical_hole_pairs,
                    ..
                } => {
                    let named_output_is = |reference: &AssistantCadNamedProgramOutputReference,
                                           expected| {
                        reference.validate_name().is_ok()
                            && reference.output == expected
                            && named_outputs.get(&reference.name) == Some(&expected)
                    };
                    let first_holes = physical_hole_pairs
                        .iter()
                        .map(|pair| pair.first_pocket_feature.name.as_str())
                        .collect::<BTreeSet<_>>();
                    let second_holes = physical_hole_pairs
                        .iter()
                        .map(|pair| pair.second_pocket_feature.name.as_str())
                        .collect::<BTreeSet<_>>();
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !named_output_is(
                            &first.occurrence,
                            AssistantCadProgramFeatureOutput::Occurrence,
                        )
                        || !named_output_is(
                            &second.occurrence,
                            AssistantCadProgramFeatureOutput::Occurrence,
                        )
                        || first.occurrence.name == second.occurrence.name
                        || !assistant_cad_vector_is_bounded(first.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(first.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(first.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(first.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(second.face_origin_local_mm)
                        || !assistant_cad_vector_is_bounded(second.inward_unit_local)
                        || !assistant_cad_vector_is_bounded(second.bounds_min_local_mm)
                        || !assistant_cad_vector_is_bounded(second.bounds_max_local_mm)
                        || !assistant_cad_vector_is_bounded(*first_center_local_mm)
                        || !assistant_cad_vector_is_bounded(*row_unit_first_local)
                        || !assistant_cad_vector_is_nonzero(*row_unit_first_local)
                        || !(1..=128).contains(count)
                        || !spacing_mm.is_finite()
                        || *spacing_mm < 0.0
                        || physical_hole_pairs.len() != *count as usize
                        || first_holes.len() != physical_hole_pairs.len()
                        || second_holes.len() != physical_hole_pairs.len()
                        || physical_hole_pairs.iter().any(|pair| {
                            !named_output_is(
                                &pair.first_pocket_feature,
                                AssistantCadProgramFeatureOutput::BodyFeature,
                            ) || !named_output_is(
                                &pair.second_pocket_feature,
                                AssistantCadProgramFeatureOutput::BodyFeature,
                            )
                        })
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "named-output pin joint creation",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::SetAssemblyJointPosition { joint_id, position } => {
                    if *joint_id == 0
                        || !position.is_finite()
                        || position.abs() > MAX_COORDINATE_MM
                        || self.operations.len() != 1
                    {
                        return Err(AssistantRequestInvalid::invalid("assembly joint edit"));
                    }
                    0
                }
                AssistantCadEditOperation::CreateDrawing {
                    name,
                    instance_paths,
                } => {
                    let unique = instance_paths.iter().collect::<BTreeSet<_>>();
                    if name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || instance_paths.is_empty()
                        || instance_paths.len() > MAX_ASSISTANT_CAD_SELECTOR_TARGETS
                        || unique.len() != instance_paths.len()
                    {
                        return Err(AssistantRequestInvalid::invalid("drawing creation"));
                    }
                    for path in instance_paths {
                        path.validate()?;
                    }
                    0
                }
                AssistantCadEditOperation::UpsertCamPlan {
                    plan_id,
                    name,
                    target_definition_id,
                    target_feature_id,
                    stock_minimum_mm,
                    stock_maximum_mm,
                    tool_number,
                    tool_diameter_mm,
                    flute_length_mm,
                    overall_length_mm,
                    holder_diameter_mm,
                    holder_length_mm,
                    spindle_rpm,
                    feed_mm_per_min,
                    plunge_mm_per_min,
                    origin_mm,
                    x_axis,
                    y_axis,
                    safe_height_mm,
                    maximum_stepdown_mm,
                    stepover_ratio,
                    radial_allowance_mm,
                    axial_allowance_mm,
                    ..
                } => {
                    let positive = [
                        *tool_diameter_mm,
                        *flute_length_mm,
                        *overall_length_mm,
                        *holder_diameter_mm,
                        *holder_length_mm,
                        *feed_mm_per_min,
                        *plunge_mm_per_min,
                        *maximum_stepdown_mm,
                    ];
                    let signed = [
                        *safe_height_mm,
                        *stepover_ratio,
                        *radial_allowance_mm,
                        *axial_allowance_mm,
                    ];
                    if *plan_id == 0
                        || *target_definition_id == 0
                        || *target_feature_id == 0
                        || *tool_number == 0
                        || *spindle_rpm == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !assistant_cad_vector_is_bounded(*stock_minimum_mm)
                        || !assistant_cad_vector_is_bounded(*stock_maximum_mm)
                        || !assistant_cad_vector_is_bounded(*origin_mm)
                        || !assistant_cad_vector_is_bounded(*x_axis)
                        || !assistant_cad_vector_is_bounded(*y_axis)
                        || !assistant_cad_vector_is_nonzero(*x_axis)
                        || !assistant_cad_vector_is_nonzero(*y_axis)
                        || positive
                            .iter()
                            .any(|value| !value.is_finite() || *value <= 0.0)
                        || signed.iter().any(|value| !value.is_finite())
                    {
                        return Err(AssistantRequestInvalid::invalid("CAM plan"));
                    }
                    0
                }
                AssistantCadEditOperation::UpsertClassificationDimension {
                    dimension_id,
                    name,
                    categories,
                } => {
                    let mut category_ids = BTreeSet::new();
                    if *dimension_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || categories.is_empty()
                        || categories.len() > MAX_ASSISTANT_CAD_EDIT_OPERATIONS
                        || categories.iter().any(|category| {
                            category.id == 0
                                || category.name.trim().is_empty()
                                || category.name.len() > MAX_ASSISTANT_NAME_BYTES
                                || category.name.chars().any(char::is_control)
                                || !category_ids.insert(category.id)
                        })
                    {
                        return Err(AssistantRequestInvalid::invalid(
                            "CAD classification dimension",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::SetOccurrenceClassification {
                    dimension_id,
                    category_id,
                    ..
                } => {
                    if *dimension_id == 0 || category_id == &Some(0) {
                        return Err(AssistantRequestInvalid::invalid(
                            "CAD classification assignment",
                        ));
                    }
                    0
                }
                AssistantCadEditOperation::CreateEvaluatorInput {
                    node_id,
                    name,
                    value,
                } => {
                    if *node_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                        || !value.is_finite()
                        || value.abs() > MAX_COORDINATE_MM
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD evaluator input"));
                    }
                    0
                }
                AssistantCadEditOperation::CreateTag { tag_id, name, .. } => {
                    if *tag_id == 0
                        || name.trim().is_empty()
                        || name.len() > MAX_ASSISTANT_NAME_BYTES
                        || name.chars().any(char::is_control)
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD tag creation"));
                    }
                    0
                }
                AssistantCadEditOperation::SetOccurrenceTag { tag_id, .. } => {
                    if tag_id == &Some(0) {
                        return Err(AssistantRequestInvalid::invalid("CAD tag assignment"));
                    }
                    0
                }
                AssistantCadEditOperation::SetTagVisibility { tag_id, .. } => {
                    if *tag_id == 0 {
                        return Err(AssistantRequestInvalid::invalid("CAD tag visibility"));
                    }
                    0
                }
                AssistantCadEditOperation::SetColor { .. }
                | AssistantCadEditOperation::SetGrounded { .. }
                | AssistantCadEditOperation::Delete { .. } => 0,
                AssistantCadEditOperation::Transform {
                    translation_mm,
                    rotation,
                    ..
                } => {
                    if !assistant_cad_vector_is_bounded(*translation_mm)
                        || (!assistant_cad_vector_is_nonzero(*translation_mm) && rotation.is_none())
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD transform"));
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
                        return Err(AssistantRequestInvalid::invalid("CAD copy"));
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
                                > MAX_COORDINATE_MM
                        })
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD linear pattern"));
                    }
                    instances.saturating_sub(1) as usize
                }
                AssistantCadEditOperation::CircularPattern {
                    instances,
                    axis,
                    angle_step_degrees,
                    ..
                } => {
                    let valid_count = (2..=MAX_ASSISTANT_ARRAY_INSTANCES).contains(instances);
                    let duplicate_angle = valid_count
                        && (1..*instances).any(|instance| {
                            let normalized =
                                (angle_step_degrees * f64::from(instance)).rem_euclid(360.0);
                            normalized.min(360.0 - normalized) < 0.01
                        });
                    if !valid_count
                        || axis.validate().is_err()
                        || !angle_step_degrees.is_finite()
                        || angle_step_degrees.abs() > MAX_COORDINATE_MM
                        || duplicate_angle
                    {
                        return Err(AssistantRequestInvalid::invalid("CAD circular pattern"));
                    }
                    axis.validate_reference_for_operation(operation_index, &self.operations)?;
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
                        return Err(AssistantRequestInvalid::invalid("CAD mirror"));
                    }
                    1
                }
            };
            generated_occurrences = generated_occurrences
                .checked_add(bounded_targets.checked_mul(generated_per_target).ok_or(
                    AssistantRequestInvalid::invalid("CAD generated occurrence count"),
                )?)
                .ok_or(AssistantRequestInvalid::invalid(
                    "CAD generated occurrence count",
                ))?;
            if generated_occurrences > MAX_ASSISTANT_CAD_GENERATED_OCCURRENCES {
                return Err(AssistantRequestInvalid::new(
                    "CAD edit program generated occurrence count",
                    AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_CAD_GENERATED_OCCURRENCES),
                ));
            }
        }
        Ok(())
    }
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
}

fn boxes_overlap(left: &AssistantSubtractionIntent, right: &AssistantSubtractionIntent) -> bool {
    (0..3).all(|axis| {
        left.origin_mm[axis] < right.origin_mm[axis] + right.size_mm[axis]
            && right.origin_mm[axis] < left.origin_mm[axis] + left.size_mm[axis]
    })
}

impl AssistantModelIntent {
    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        if self.boxes.is_empty()
            && self.translations.is_empty()
            && self.rotations.is_empty()
            && self.profile_translations.is_empty()
            && self.parameter_edits.is_empty()
            && self.linear_arrays.is_empty()
        {
            return Err(AssistantRequestInvalid::new(
                "proposal",
                AssistantRequestProblem::Empty,
            ));
        }
        if self.boxes.len() > MAX_ASSISTANT_BOXES {
            return Err(AssistantRequestInvalid::new(
                "proposal box count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_BOXES),
            ));
        }
        if self.translations.len() > MAX_ASSISTANT_TRANSLATIONS {
            return Err(AssistantRequestInvalid::new(
                "proposal translation count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_TRANSLATIONS),
            ));
        }
        if self.rotations.len() > MAX_ASSISTANT_ROTATIONS {
            return Err(AssistantRequestInvalid::new(
                "proposal rotation count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_ROTATIONS),
            ));
        }
        if self.profile_translations.len() > MAX_ASSISTANT_PROFILE_TRANSLATIONS {
            return Err(AssistantRequestInvalid::new(
                "proposal profile translation count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_PROFILE_TRANSLATIONS),
            ));
        }
        if self.parameter_edits.len() > MAX_ASSISTANT_PARAMETER_EDITS {
            return Err(AssistantRequestInvalid::new(
                "proposal parameter edit count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_PARAMETER_EDITS),
            ));
        }
        if self.linear_arrays.len() > MAX_ASSISTANT_ARRAYS {
            return Err(AssistantRequestInvalid::new(
                "proposal linear array count",
                AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_ARRAYS),
            ));
        }
        if self.replace_scene
            && (!self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty())
        {
            return Err(AssistantRequestInvalid::new(
                "edit of existing geometry",
                AssistantRequestProblem::ConflictsWith("scene replacement"),
            ));
        }
        if !self.profile_translations.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty())
        {
            return Err(AssistantRequestInvalid::new(
                "profile translation",
                AssistantRequestProblem::ConflictsWith("geometry mutations"),
            ));
        }
        if !self.parameter_edits.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.rotations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.linear_arrays.is_empty())
        {
            return Err(AssistantRequestInvalid::new(
                "parameter edit",
                AssistantRequestProblem::ConflictsWith("geometry mutations"),
            ));
        }
        let mut translated_occurrences = BTreeSet::new();
        for translation in &self.translations {
            if translation.occurrence_id == 0
                || !translated_occurrences.insert(translation.occurrence_id)
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            {
                return Err(AssistantRequestInvalid::invalid("translation"));
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
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
                || !axis_length_squared.is_finite()
                || axis_length_squared <= f64::EPSILON
                || !rotation.angle_degrees.is_finite()
                || rotation.angle_degrees.abs() > MAX_COORDINATE_MM
                || shortest_angle < 0.01
            {
                return Err(AssistantRequestInvalid::invalid("rotation"));
            }
        }
        if !rotated_occurrences.is_empty() && !rotated_groups.is_empty() {
            return Err(AssistantRequestInvalid::new(
                "rotation occurrence target",
                AssistantRequestProblem::ConflictsWith("group targets"),
            ));
        }
        for translation in &self.profile_translations {
            if translation.definition_id == 0
                || translation.body_id == 0
                || translation.profile_id == 0
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
                || translation.delta_mm.iter().all(|value| *value == 0.0)
            {
                return Err(AssistantRequestInvalid::invalid("profile translation"));
            }
        }
        for edit in &self.parameter_edits {
            if edit.definition_id == 0
                || edit.body_id == 0
                || edit.feature_id == 0
                || edit.constraint_id == Some(0)
                || !edit.value_mm.is_finite()
                || edit.value_mm <= 0.0
                || edit.value_mm > MAX_COORDINATE_MM
            {
                return Err(AssistantRequestInvalid::invalid("parameter edit"));
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
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
                || array.step_mm.iter().all(|value| *value == 0.0)
                || array.step_mm.iter().any(|value| {
                    (*value * f64::from(array.instances.saturating_sub(1))).abs()
                        > MAX_COORDINATE_MM
                })
            {
                return Err(AssistantRequestInvalid::invalid("linear array"));
            }
            let Some(outputs) = array
                .occurrence_ids
                .len()
                .checked_mul(array.instances.saturating_sub(1) as usize)
            else {
                return Err(AssistantRequestInvalid::invalid(
                    "linear array output count",
                ));
            };
            let Some(total_outputs) = array_outputs.checked_add(outputs) else {
                return Err(AssistantRequestInvalid::invalid(
                    "linear array output count",
                ));
            };
            if total_outputs > MAX_ASSISTANT_ARRAY_OUTPUTS {
                return Err(AssistantRequestInvalid::new(
                    "proposal array occurrence count",
                    AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_ARRAY_OUTPUTS),
                ));
            }
            array_outputs = total_outputs;
        }
        for item in &self.boxes {
            if item.name.trim().is_empty()
                || item.name.len() > MAX_ASSISTANT_NAME_BYTES
                || item.name.chars().any(char::is_control)
            {
                return Err(AssistantRequestInvalid::invalid("box name"));
            }
            if item
                .size_mm
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_COORDINATE_MM)
                || item
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            {
                return Err(AssistantRequestInvalid::new(
                    "box",
                    AssistantRequestProblem::Outside("the coordinate envelope"),
                ));
            }
            if item.subtract_boxes.len() > MAX_ASSISTANT_SUBTRACTIONS {
                return Err(AssistantRequestInvalid::new(
                    "body subtraction count",
                    AssistantRequestProblem::ExceedsLimit(MAX_ASSISTANT_SUBTRACTIONS),
                ));
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
                    return Err(AssistantRequestInvalid::new(
                        "subtraction",
                        AssistantRequestProblem::Outside("its body"),
                    ));
                }
            }
            if item.subtract_boxes.iter().enumerate().any(|(index, left)| {
                item.subtract_boxes[index + 1..]
                    .iter()
                    .any(|right| boxes_overlap(left, right))
            }) {
                return Err(AssistantRequestInvalid::new(
                    "subtraction",
                    AssistantRequestProblem::ConflictsWith("an overlapping subtraction"),
                ));
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
    /// Messages of the errors that caused the rejection, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causes: Vec<String>,
}

impl AssistantRejectionDiagnostic {
    /// Records `error` and its sources, outermost first. Control characters become
    /// spaces and the whole chain shares the text budget of one field, so the
    /// diagnostic stays valid.
    #[must_use]
    pub fn caused_by(mut self: Box<Self>, error: &(dyn std::error::Error + 'static)) -> Box<Self> {
        let mut budget = MAX_ASSISTANT_REJECTION_TEXT_BYTES
            .saturating_sub(self.causes.iter().map(String::len).sum());
        let mut next = Some(error);
        while let Some(error) = next {
            next = error.source();
            let nested = error
                .downcast_ref::<Self>()
                .map(|nested| nested.causes.clone());
            let message: String = error
                .to_string()
                .chars()
                .map(|character| {
                    if character.is_control() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect();
            let message = message.trim();
            let mut end = message.len().min(budget);
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                continue;
            }
            self.causes.push(message[..end].trim_end().to_owned());
            budget -= end;
            // A nested diagnostic carries its own chain as text.
            for cause in nested.into_iter().flatten() {
                if cause.len() > budget {
                    break;
                }
                budget -= cause.len();
                self.causes.push(cause);
            }
        }
        self
    }

    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
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
            || !self
                .causes
                .iter()
                .all(|cause| bounded_text_is_valid(cause, MAX_ASSISTANT_REJECTION_TEXT_BYTES))
            || self.causes.iter().map(String::len).sum::<usize>()
                > MAX_ASSISTANT_REJECTION_TEXT_BYTES
            || serde_json::to_vec(self)
                .map_or(true, |bytes| bytes.len() > MAX_ASSISTANT_REJECTION_BYTES)
        {
            return Err(AssistantRequestInvalid::invalid("rejection diagnostic"));
        }
        Ok(())
    }
}

impl fmt::Display for AssistantRejectionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code, self.target, self.failed_invariant
        )
    }
}

impl std::error::Error for AssistantRejectionDiagnostic {}

impl From<AssistantRejectionPhase> for ketchup_rejection::RejectionPhase {
    fn from(phase: AssistantRejectionPhase) -> Self {
        match phase {
            AssistantRejectionPhase::IntentValidation
            | AssistantRejectionPhase::CanonicalValidation
            | AssistantRejectionPhase::DomainValidation => Self::Validation,
            AssistantRejectionPhase::ProposalPlanning => Self::Planning,
            AssistantRejectionPhase::ExactValidation => Self::Verification,
            AssistantRejectionPhase::CommitValidation => Self::Commit,
        }
    }
}

/// The sidecar wire diagnostic read as the shared rejection; the operation that
/// refused leads the reason so nothing of the diagnostic is lost.
impl From<AssistantRejectionDiagnostic> for ketchup_rejection::Rejection {
    fn from(diagnostic: AssistantRejectionDiagnostic) -> Self {
        Self::new(diagnostic.code, diagnostic.phase.into())
            .target(diagnostic.target)
            .reason(format!(
                "{}: {}",
                diagnostic.operation, diagnostic.failed_invariant
            ))
            .fix_hint(diagnostic.repair_hint)
            .caused_by_messages(diagnostic.causes)
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
    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        if self.provider.is_empty()
            || self.provider.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.model.is_empty()
            || self.model.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.system_prompt.len() > 64 * 1024
            || self.response_text.len() > 64 * 1024
            || serde_json::to_vec(&self.request_payload)
                .map_or(true, |bytes| bytes.len() > 128 * 1024)
        {
            return Err(AssistantRequestInvalid::new(
                "API diagnostics record",
                AssistantRequestProblem::Outside("its bounded envelope"),
            ));
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
    pub fn validate(&self) -> Result<(), AssistantRequestInvalid> {
        if self.message.trim().is_empty() {
            return Err(AssistantRequestInvalid::new(
                "reply message",
                AssistantRequestProblem::Empty,
            ));
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
