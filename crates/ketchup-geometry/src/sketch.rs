use crate::dimension::Dimension;
use crate::id::FeatureId;
use crate::reference::BodySubshapeRef;
use crate::tolerance::{
    ACCUMULATED_ROUNDING, APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, FINITE_DIFFERENCE_STEP,
    INITIAL_DAMPING, MAX_COORDINATE_MM, NEGLIGIBLE, ROUNDING,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

mod rank;

mod region;
mod solver;
use region::*;
use solver::*;

pub const MAX_SKETCH_ENTITIES: usize = 4_096;
pub const MAX_SKETCH_CONSTRAINTS: usize = 8_192;
const MAX_SKETCH_SOLVER_DOF: usize = 512;
const EPSILON_MM: f64 = DEFAULT_LINEAR_TOLERANCE_MM;
const FRAME_EPSILON: f64 = ROUNDING;
const MAX_SKETCH_SOLVER_ITERATIONS: u16 = 256;
const MAX_SKETCH_NUMERICAL_EVALUATIONS: usize = 67_108_864;
const MAX_CUBIC_FLATTEN_DEPTH: u8 = 16;
const MAX_CUBIC_FLATTEN_SEGMENTS: usize = 16_384;
const CUBIC_FLATTEN_TOLERANCE_MM: f64 = APPROXIMATION;

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct SketchEntityId(pub u64);

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct SketchConstraintId(pub u64);

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct SketchRegionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PrincipalPlane {
    Xy,
    Yz,
    Xz,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WorkplaneSupportHealth {
    Resolved,
    Ambiguous,
    Lost,
    Stale,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WorkplaneSupport {
    /// An authoritative frame with no upstream geometric support.
    Free,
    Principal(PrincipalPlane),
    Offset {
        base: FeatureId,
        distance: Dimension,
    },
    PlanarFace {
        reference: Box<BodySubshapeRef>,
        #[serde(serialize_with = "crate::derived::derived")]
        health: WorkplaneSupportHealth,
    },
    ConstructionPlane {
        feature: FeatureId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkplaneFrame {
    pub origin_mm: [f64; 3],
    pub x_axis: [f64; 3],
    pub y_axis: [f64; 3],
    pub normal: [f64; 3],
}

impl WorkplaneFrame {
    /// Construct a right-handed frame without normalizing or repairing input axes.
    pub fn from_axes(
        origin_mm: [f64; 3],
        x_axis: [f64; 3],
        y_axis: [f64; 3],
    ) -> Result<Self, SketchError> {
        let frame = Self {
            origin_mm,
            x_axis,
            y_axis,
            normal: cross(x_axis, y_axis),
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn from_construction_plane(
        origin_mm: [f64; 3],
        normal: [f64; 3],
        x_direction: [f64; 3],
    ) -> Result<Self, SketchError> {
        let normalized = |vector: [f64; 3]| {
            let length = dot(vector, vector).sqrt();
            vector.map(|value| value / length)
        };
        let normal = normalized(normal);
        let x_axis = normalized(x_direction);
        Self::from_axes(origin_mm, x_axis, cross(normal, x_axis))
    }

    #[must_use]
    pub const fn principal(plane: PrincipalPlane) -> Self {
        match plane {
            PrincipalPlane::Xy => Self {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
            },
            PrincipalPlane::Yz => Self {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [0.0, 1.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [1.0, 0.0, 0.0],
            },
            PrincipalPlane::Xz => Self {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [0.0, -1.0, 0.0],
            },
        }
    }

    #[must_use]
    pub fn offset(self, distance_mm: f64) -> Self {
        let mut frame = self;
        for (coordinate, normal) in frame.origin_mm.iter_mut().zip(frame.normal) {
            *coordinate += normal * distance_mm;
        }
        frame
    }

    pub fn validate(&self) -> Result<(), SketchError> {
        if self
            .origin_mm
            .iter()
            .chain(self.x_axis.iter())
            .chain(self.y_axis.iter())
            .chain(self.normal.iter())
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        {
            return Err(SketchError::InvalidWorkplaneFrame);
        }
        let unit = |axis: [f64; 3]| (dot(axis, axis) - 1.0).abs() <= FRAME_EPSILON;
        let cross_xy = cross(self.x_axis, self.y_axis);
        if !unit(self.x_axis)
            || !unit(self.y_axis)
            || !unit(self.normal)
            || dot(self.x_axis, self.y_axis).abs() > FRAME_EPSILON
            || dot(self.x_axis, self.normal).abs() > FRAME_EPSILON
            || dot(self.y_axis, self.normal).abs() > FRAME_EPSILON
            || distance3(cross_xy, self.normal) > FRAME_EPSILON
        {
            return Err(SketchError::InvalidWorkplaneFrame);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct WorkplaneSpec {
    pub support: WorkplaneSupport,
    pub frame: WorkplaneFrame,
}

/// A workplane on a planar face takes its frame from the face at every evaluation, so
/// only a workplane without face support is identified by its frame.
impl serde::Serialize for WorkplaneSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        struct Frame<'a>(&'a WorkplaneSpec);
        impl serde::Serialize for Frame<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                if matches!(self.0.support, WorkplaneSupport::PlanarFace { .. }) {
                    crate::derived::derived(&self.0.frame, serializer)
                } else {
                    self.0.frame.serialize(serializer)
                }
            }
        }
        let mut spec = serializer.serialize_struct("WorkplaneSpec", 2)?;
        spec.serialize_field("support", &self.support)?;
        spec.serialize_field("frame", &Frame(self))?;
        spec.end()
    }
}

impl WorkplaneSpec {
    #[must_use]
    pub const fn principal(plane: PrincipalPlane) -> Self {
        Self {
            support: WorkplaneSupport::Principal(plane),
            frame: WorkplaneFrame::principal(plane),
        }
    }

    pub fn validate_local(&self) -> Result<(), SketchError> {
        self.frame.validate()?;
        match &self.support {
            WorkplaneSupport::Free => Ok(()),
            WorkplaneSupport::Principal(plane)
                if self.frame == WorkplaneFrame::principal(*plane) =>
            {
                Ok(())
            }
            WorkplaneSupport::Principal(_) => Err(SketchError::InvalidWorkplaneFrame),
            WorkplaneSupport::Offset { base, distance } => {
                if base.0 == 0 {
                    return Err(SketchError::MissingWorkplaneSupport(*base));
                }
                Dimension::new(distance.source_token(), distance.millimetres())
                    .map_err(|_| SketchError::InvalidDimension)?;
                Ok(())
            }
            WorkplaneSupport::PlanarFace { reference, health } => {
                if *health != WorkplaneSupportHealth::Resolved {
                    return Err(SketchError::UnresolvedWorkplaneSupport(*health));
                }
                if reference.expected_type != "planar_face"
                    || reference.expected_cardinality != 1
                    || !reference.has_valid_lineage()
                {
                    return Err(SketchError::InvalidPlanarFaceSupport);
                }
                Ok(())
            }
            WorkplaneSupport::ConstructionPlane { feature } if feature.0 != 0 => Ok(()),
            WorkplaneSupport::ConstructionPlane { feature } => {
                Err(SketchError::MissingWorkplaneSupport(*feature))
            }
        }
    }
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum SketchPointKind {
    Start,
    End,
    Center,
    Control1,
    Control2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum SketchOffsetSide {
    Left,
    Right,
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct SketchPointRef {
    pub entity: SketchEntityId,
    pub point: SketchPointKind,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SketchEntity {
    Line {
        id: SketchEntityId,
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    Arc {
        id: SketchEntityId,
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    Circle {
        id: SketchEntityId,
        center_mm: [f64; 2],
        radius_mm: f64,
    },
    CubicBezier {
        id: SketchEntityId,
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
    },
}

impl SketchEntity {
    #[must_use]
    pub const fn id(&self) -> SketchEntityId {
        match self {
            Self::Line { id, .. }
            | Self::Arc { id, .. }
            | Self::Circle { id, .. }
            | Self::CubicBezier { id, .. } => *id,
        }
    }

    fn degrees_of_freedom(&self) -> usize {
        match self {
            Self::Line { .. } => 4,
            Self::Arc { .. } => 5,
            Self::Circle { .. } => 3,
            Self::CubicBezier { .. } => 8,
        }
    }

    fn point(&self, point: SketchPointKind) -> Option<[f64; 2]> {
        match (self, point) {
            (
                Self::Line { start_mm, .. }
                | Self::Arc { start_mm, .. }
                | Self::CubicBezier { start_mm, .. },
                SketchPointKind::Start,
            ) => Some(*start_mm),
            (
                Self::Line { end_mm, .. }
                | Self::Arc { end_mm, .. }
                | Self::CubicBezier { end_mm, .. },
                SketchPointKind::End,
            ) => Some(*end_mm),
            (
                Self::Arc { center_mm, .. } | Self::Circle { center_mm, .. },
                SketchPointKind::Center,
            ) => Some(*center_mm),
            (Self::CubicBezier { control_1_mm, .. }, SketchPointKind::Control1) => {
                Some(*control_1_mm)
            }
            (Self::CubicBezier { control_2_mm, .. }, SketchPointKind::Control2) => {
                Some(*control_2_mm)
            }
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), SketchError> {
        if self.id().0 == 0 {
            return Err(SketchError::ReservedEntityId);
        }
        let valid_point = |point: &[f64; 2]| {
            point
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        };
        match self {
            Self::Line {
                start_mm, end_mm, ..
            } => {
                if !valid_point(start_mm)
                    || !valid_point(end_mm)
                    || distance2(*start_mm, *end_mm) <= EPSILON_MM
                {
                    return Err(SketchError::InvalidEntity(self.id()));
                }
            }
            Self::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                let start_radius = distance2(*start_mm, *center_mm);
                let end_radius = distance2(*end_mm, *center_mm);
                if !valid_point(start_mm)
                    || !valid_point(end_mm)
                    || !valid_point(center_mm)
                    || start_radius <= EPSILON_MM
                    || (start_radius - end_radius).abs() > EPSILON_MM
                    || distance2(*start_mm, *end_mm) <= EPSILON_MM
                {
                    return Err(SketchError::InvalidEntity(self.id()));
                }
            }
            Self::Circle {
                center_mm,
                radius_mm,
                ..
            } => {
                if !valid_point(center_mm)
                    || !radius_mm.is_finite()
                    || *radius_mm <= EPSILON_MM
                    || *radius_mm > MAX_COORDINATE_MM
                {
                    return Err(SketchError::InvalidEntity(self.id()));
                }
            }
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => {
                if !valid_point(start_mm)
                    || !valid_point(control_1_mm)
                    || !valid_point(control_2_mm)
                    || !valid_point(end_mm)
                    || distance2(*start_mm, *end_mm) <= EPSILON_MM
                    || cubic_control_polygon_length([
                        *start_mm,
                        *control_1_mm,
                        *control_2_mm,
                        *end_mm,
                    ]) <= EPSILON_MM
                {
                    return Err(SketchError::InvalidEntity(self.id()));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SketchConstraintKind {
    Horizontal {
        entity: SketchEntityId,
    },
    Vertical {
        entity: SketchEntityId,
    },
    Coincident {
        a: SketchPointRef,
        b: SketchPointRef,
    },
    Distance {
        a: SketchPointRef,
        b: SketchPointRef,
        value: Dimension,
    },
    Radius {
        entity: SketchEntityId,
        value: Dimension,
    },
    FixedPoint {
        point: SketchPointRef,
        position_mm: [f64; 2],
    },
    Parallel {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Perpendicular {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Tangent {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Angle {
        a: SketchEntityId,
        b: SketchEntityId,
        angle_degrees: f64,
    },
    Equal {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Symmetric {
        a: SketchPointRef,
        b: SketchPointRef,
        axis: SketchEntityId,
    },
    Concentric {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Collinear {
        a: SketchEntityId,
        b: SketchEntityId,
    },
    Midpoint {
        point: SketchPointRef,
        line: SketchEntityId,
    },
    PointOnCurve {
        point: SketchPointRef,
        curve: SketchEntityId,
    },
    Projection {
        entity: SketchEntityId,
        source_feature: FeatureId,
        source_entity: SketchEntityId,
        target: Box<SketchEntity>,
    },
    Construction {
        entity: SketchEntityId,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SketchConstraint {
    pub id: SketchConstraintId,
    pub kind: SketchConstraintKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchSolverPolicy {
    pub max_iterations: u16,
    pub tolerance_mm: f64,
    pub finite_difference_step: f64,
    pub initial_damping: f64,
}

impl Default for SketchSolverPolicy {
    fn default() -> Self {
        Self {
            max_iterations: 64,
            tolerance_mm: EPSILON_MM,
            finite_difference_step: FINITE_DIFFERENCE_STEP,
            initial_damping: INITIAL_DAMPING,
        }
    }
}

impl SketchSolverPolicy {
    fn validate(self) -> Result<Self, SketchError> {
        if self.max_iterations == 0
            || self.max_iterations > MAX_SKETCH_SOLVER_ITERATIONS
            || !self.tolerance_mm.is_finite()
            || self.tolerance_mm <= 0.0
            || self.tolerance_mm > EPSILON_MM
            || !self.finite_difference_step.is_finite()
            || self.finite_difference_step <= 0.0
            || !self.initial_damping.is_finite()
            || self.initial_damping <= 0.0
        {
            return Err(SketchError::InvalidSolverPolicy);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchSolveStatus {
    UnderConstrained { remaining_dof: usize },
    FullyConstrained,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SketchSolveReport {
    pub status: SketchSolveStatus,
    pub entity_count: usize,
    pub constraint_count: usize,
    pub equation_count: usize,
    pub unconstrained_entity_ids: Vec<SketchEntityId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchDiagnosticStatus {
    UnderConstrained { remaining_dof: usize },
    FullyConstrained,
    Conflicting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SketchDiagnosticReport {
    pub status: SketchDiagnosticStatus,
    pub entity_ids: Vec<SketchEntityId>,
    pub constraint_ids: Vec<SketchConstraintId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchSolution {
    pub report: SketchSolveReport,
    pub entities: Vec<SketchEntity>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolvedSketchRegionEdge {
    Line {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    Arc {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
    },
}

impl SolvedSketchRegionEdge {
    #[must_use]
    pub const fn start_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { start_mm, .. }
            | Self::Arc { start_mm, .. }
            | Self::CubicBezier { start_mm, .. } => *start_mm,
        }
    }

    #[must_use]
    pub const fn end_mm(&self) -> [f64; 2] {
        match self {
            Self::Line { end_mm, .. }
            | Self::Arc { end_mm, .. }
            | Self::CubicBezier { end_mm, .. } => *end_mm,
        }
    }

    #[must_use]
    fn reversed(&self) -> Self {
        match self {
            Self::Line { start_mm, end_mm } => Self::Line {
                start_mm: *end_mm,
                end_mm: *start_mm,
            },
            Self::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => Self::Arc {
                start_mm: *end_mm,
                end_mm: *start_mm,
                center_mm: *center_mm,
                clockwise: !clockwise,
            },
            Self::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => Self::CubicBezier {
                start_mm: *end_mm,
                control_1_mm: *control_2_mm,
                control_2_mm: *control_1_mm,
                end_mm: *start_mm,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolvedSketchRegionProfile {
    Polyline(Vec<[f64; 2]>),
    Boundary(Vec<SolvedSketchRegionEdge>),
    Circle { center_mm: [f64; 2], radius_mm: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SolvedSketchRegion {
    pub id: SketchRegionId,
    pub entity_ids: Vec<SketchEntityId>,
    /// The outer loop's entities in boundary order, one per outer edge.
    pub outer_entity_ids: Vec<SketchEntityId>,
    pub outer: SolvedSketchRegionProfile,
    pub holes: Vec<SolvedSketchRegionProfile>,
}

struct SolvedSketchLoop {
    entity_ids: Vec<SketchEntityId>,
    ordered_entity_ids: Vec<SketchEntityId>,
    profile: SolvedSketchRegionProfile,
    area: f64,
}

#[derive(Clone, Copy)]
enum RegionCurve {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    Arc {
        start: [f64; 2],
        end: [f64; 2],
        center: [f64; 2],
        clockwise: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FeatureDirection {
    AlongNormal,
    OppositeNormal,
    Vector([f64; 3]),
}

impl FeatureDirection {
    pub fn validate(self) -> Result<(), SketchError> {
        if let Self::Vector(vector) = self {
            let length = vector[0].hypot(vector[1]).hypot(vector[2]);
            if vector.iter().any(|component| !component.is_finite())
                || length <= EPSILON_MM
                || length > MAX_COORDINATE_MM
            {
                return Err(SketchError::InvalidFeatureDirection);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn vector(self, normal: [f64; 3]) -> Option<[f64; 3]> {
        let vector = match self {
            Self::AlongNormal => normal,
            Self::OppositeNormal => normal.map(|component| -component),
            Self::Vector(vector) => vector,
        };
        let length = vector[0].hypot(vector[1]).hypot(vector[2]);
        (vector.iter().all(|component| component.is_finite()) && length > EPSILON_MM).then(|| {
            vector.map(|component| {
                let normalized = component / length;
                if normalized == 0.0 { 0.0 } else { normalized }
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FeatureExtentEnd {
    Blind(Dimension),
    ThroughAll,
    UpToFace(Box<BodySubshapeRef>),
}

impl FeatureExtentEnd {
    fn validate(&self) -> Result<(), SketchError> {
        match self {
            Self::Blind(distance) => validate_extent_distance(distance),
            Self::ThroughAll => Ok(()),
            Self::UpToFace(reference) => validate_extent_reference(reference),
        }
    }

    fn references(&self) -> Option<&BodySubshapeRef> {
        match self {
            Self::UpToFace(reference) => Some(reference),
            Self::Blind(_) | Self::ThroughAll => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FeatureExtent {
    Blind(Dimension),
    ThroughAll,
    UpToFace(Box<BodySubshapeRef>),
    Symmetric(Dimension),
    Bidirectional {
        along: FeatureExtentEnd,
        opposite: FeatureExtentEnd,
    },
}

impl FeatureExtent {
    pub fn validate(&self) -> Result<(), SketchError> {
        match self {
            // A negative blind distance sweeps against the direction.
            Self::Blind(distance) => validate_extent_length(distance, distance.millimetres().abs()),
            Self::Symmetric(distance) => validate_extent_distance(distance),
            Self::ThroughAll => Ok(()),
            Self::UpToFace(reference) => validate_extent_reference(reference),
            Self::Bidirectional { along, opposite } => {
                along.validate()?;
                opposite.validate()
            }
        }
    }

    #[must_use]
    pub const fn blind_distance(&self) -> Option<&Dimension> {
        match self {
            Self::Blind(distance) => Some(distance),
            _ => None,
        }
    }

    #[must_use]
    pub fn references(&self) -> Vec<&BodySubshapeRef> {
        match self {
            Self::UpToFace(reference) => vec![reference],
            Self::Bidirectional { along, opposite } => [along.references(), opposite.references()]
                .into_iter()
                .flatten()
                .collect(),
            Self::Blind(_) | Self::ThroughAll | Self::Symmetric(_) => Vec::new(),
        }
    }
}

fn validate_extent_distance(distance: &Dimension) -> Result<(), SketchError> {
    validate_extent_length(distance, distance.millimetres())
}

fn validate_extent_length(distance: &Dimension, length_mm: f64) -> Result<(), SketchError> {
    Dimension::new(distance.source_token(), distance.millimetres())
        .map_err(|_| SketchError::InvalidDimension)?;
    if length_mm <= EPSILON_MM || length_mm > MAX_COORDINATE_MM {
        return Err(SketchError::InvalidDimension);
    }
    Ok(())
}

fn validate_extent_reference(reference: &BodySubshapeRef) -> Result<(), SketchError> {
    if reference.expected_type != "planar_face"
        || reference.expected_cardinality != 1
        || !reference.has_valid_lineage()
    {
        return Err(SketchError::InvalidFeatureExtentReference);
    }
    Ok(())
}

/// The closed planar shape a pad sweeps.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PadProfile {
    /// A profile feature, or a sketch with exactly one solved region.
    Feature(FeatureId),
    /// One region of a sketch.
    SketchRegion {
        sketch: FeatureId,
        region: SketchRegionId,
    },
}

impl PadProfile {
    #[must_use]
    pub const fn feature_id(self) -> FeatureId {
        match self {
            Self::Feature(id) | Self::SketchRegion { sketch: id, .. } => id,
        }
    }
}

/// Where a cut measures its extent from.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CutStart {
    /// The plane of the profile.
    ProfilePlane,
    /// The plane of the profile, which lies on this face of the target.
    Support(Box<BodySubshapeRef>),
    /// The target face the direction points out of; a blind cut reaches back
    /// into the target from there.
    TargetFace,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PadOperation {
    NewBody,
    Cut { target: FeatureId, start: CutStart },
}

impl PadOperation {
    #[must_use]
    pub const fn target(&self) -> Option<FeatureId> {
        match self {
            Self::NewBody => None,
            Self::Cut { target, .. } => Some(*target),
        }
    }

    #[must_use]
    pub fn support(&self) -> Option<&BodySubshapeRef> {
        match self {
            Self::Cut {
                start: CutStart::Support(support),
                ..
            } => Some(support),
            _ => None,
        }
    }
}

/// Every linear sweep of a closed profile: a new body or a cut into a target.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PadSpec {
    pub profile: PadProfile,
    pub direction: FeatureDirection,
    pub extent: FeatureExtent,
    pub operation: PadOperation,
}

impl PadSpec {
    /// Face references the extent and the cut start resolve against.
    #[must_use]
    pub fn references(&self) -> Vec<&BodySubshapeRef> {
        let mut references = self.extent.references();
        references.extend(self.operation.support());
        references
    }

    /// The profile and blind distance of a pad swept along the profile normal.
    #[must_use]
    pub fn blind_along_normal(&self) -> Option<(FeatureId, &Dimension)> {
        match (&self.direction, &self.extent) {
            (FeatureDirection::AlongNormal, FeatureExtent::Blind(distance)) => {
                Some((self.profile.feature_id(), distance))
            }
            _ => None,
        }
    }

    /// The same pad with its profile and target features renamed by `map`;
    /// face references are left for the caller.
    pub fn with_features<E>(
        &self,
        mut map: impl FnMut(FeatureId) -> Result<FeatureId, E>,
    ) -> Result<Self, E> {
        let mut mapped = self.clone();
        mapped.profile = match self.profile {
            PadProfile::Feature(id) => PadProfile::Feature(map(id)?),
            PadProfile::SketchRegion { sketch, region } => PadProfile::SketchRegion {
                sketch: map(sketch)?,
                region,
            },
        };
        if let PadOperation::Cut { target, .. } = &mut mapped.operation {
            *target = map(*target)?;
        }
        Ok(mapped)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SketchSpec {
    pub workplane: FeatureId,
    pub entities: Vec<SketchEntity>,
    pub constraints: Vec<SketchConstraint>,
}

impl SketchSpec {
    /// Corners of the axis-aligned rectangle this sketch draws, if it is one.
    #[must_use]
    pub fn rectangle_bounds(&self) -> Option<[[f64; 2]; 2]> {
        if !self.constraints.is_empty() || self.entities.len() != 4 {
            return None;
        }
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for entity in &self.entities {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity
            else {
                return None;
            };
            for point in [start_mm, end_mm] {
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(point[axis]);
                    maximum[axis] = maximum[axis].max(point[axis]);
                }
            }
        }
        if (0..2).any(|axis| maximum[axis] - minimum[axis] <= EPSILON_MM) {
            return None;
        }
        let mut sides = std::collections::BTreeSet::new();
        for entity in &self.entities {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity
            else {
                return None;
            };
            let axis = (0..2).find(|axis| start_mm[*axis] == end_mm[*axis])?;
            let tangent = 1 - axis;
            let maximum_side = if start_mm[axis] == minimum[axis] {
                false
            } else if start_mm[axis] == maximum[axis] {
                true
            } else {
                return None;
            };
            if start_mm[tangent].min(end_mm[tangent]) != minimum[tangent]
                || start_mm[tangent].max(end_mm[tangent]) != maximum[tangent]
                || !sides.insert((axis, maximum_side))
            {
                return None;
            }
        }
        Some([minimum, maximum])
    }

    #[must_use]
    pub fn is_projected_entity(&self, entity_id: SketchEntityId) -> bool {
        self.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                SketchConstraintKind::Projection { entity, .. } if entity == entity_id
            )
        })
    }

    #[must_use]
    pub fn is_construction_entity(&self, entity_id: SketchEntityId) -> bool {
        self.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                SketchConstraintKind::Projection { entity, .. }
                    | SketchConstraintKind::Construction { entity }
                    if entity == entity_id
            )
        })
    }

    pub fn create_constraint(&mut self, constraint: SketchConstraint) -> Result<(), SketchError> {
        if constraint.id.0 == 0 {
            return Err(SketchError::ReservedConstraintId);
        }
        if matches!(
            constraint.kind,
            SketchConstraintKind::Projection { .. } | SketchConstraintKind::Construction { .. }
        ) {
            return Err(SketchError::ManagedConstraintReadOnly(constraint.id));
        }
        if self.constraints.len() >= MAX_SKETCH_CONSTRAINTS {
            return Err(SketchError::ResourceLimit);
        }
        if self
            .constraints
            .iter()
            .any(|candidate| candidate.id == constraint.id)
        {
            return Err(SketchError::ConstraintIdCollision(constraint.id));
        }
        let mut updated = self.clone();
        updated.constraints.push(constraint);
        updated.constraints.sort_by_key(|candidate| candidate.id);
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn replace_constraint(&mut self, constraint: SketchConstraint) -> Result<(), SketchError> {
        if constraint.id.0 == 0 {
            return Err(SketchError::ReservedConstraintId);
        }
        let Some(index) = self
            .constraints
            .iter()
            .position(|candidate| candidate.id == constraint.id)
        else {
            return Err(SketchError::InvalidConstraintReference(constraint.id));
        };
        if matches!(
            self.constraints[index].kind,
            SketchConstraintKind::Projection { .. } | SketchConstraintKind::Construction { .. }
        ) || matches!(
            constraint.kind,
            SketchConstraintKind::Projection { .. } | SketchConstraintKind::Construction { .. }
        ) {
            return Err(SketchError::ManagedConstraintReadOnly(constraint.id));
        }
        let mut updated = self.clone();
        updated.constraints[index] = constraint;
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn delete_constraint(
        &mut self,
        constraint_id: SketchConstraintId,
    ) -> Result<(), SketchError> {
        let Some(index) = self
            .constraints
            .iter()
            .position(|constraint| constraint.id == constraint_id)
        else {
            return Err(SketchError::InvalidConstraintReference(constraint_id));
        };
        if matches!(
            self.constraints[index].kind,
            SketchConstraintKind::Projection { .. } | SketchConstraintKind::Construction { .. }
        ) {
            return Err(SketchError::ManagedConstraintReadOnly(constraint_id));
        }
        let mut updated = self.clone();
        updated.constraints.remove(index);
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn set_entity_construction(
        &mut self,
        entity_id: SketchEntityId,
        construction: bool,
        constraint_id: SketchConstraintId,
    ) -> Result<(), SketchError> {
        if entity_id.0 == 0 || !self.entities.iter().any(|entity| entity.id() == entity_id) {
            return Err(SketchError::EntityNotFound(entity_id));
        }
        if self.is_projected_entity(entity_id) {
            return Err(SketchError::ProjectedEntityReadOnly(entity_id));
        }
        let existing = self.constraints.iter().position(|constraint| {
            matches!(
                constraint.kind,
                SketchConstraintKind::Construction { entity } if entity == entity_id
            )
        });
        let mut updated = self.clone();
        if construction {
            if constraint_id.0 == 0 {
                return Err(SketchError::ReservedConstraintId);
            }
            if existing.is_some() {
                return Err(SketchError::InvalidConstructionState(entity_id));
            }
            if updated.constraints.len() >= MAX_SKETCH_CONSTRAINTS {
                return Err(SketchError::ResourceLimit);
            }
            if updated
                .constraints
                .iter()
                .any(|constraint| constraint.id == constraint_id)
            {
                return Err(SketchError::ConstraintIdCollision(constraint_id));
            }
            updated.constraints.push(SketchConstraint {
                id: constraint_id,
                kind: SketchConstraintKind::Construction { entity: entity_id },
            });
            updated.constraints.sort_by_key(|constraint| constraint.id);
        } else {
            let Some(index) = existing else {
                return Err(SketchError::InvalidConstructionState(entity_id));
            };
            if updated.constraints[index].id != constraint_id {
                return Err(SketchError::InvalidConstructionState(entity_id));
            }
            updated.constraints.remove(index);
        }
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn add_projection(
        &mut self,
        entity: SketchEntity,
        source_feature: FeatureId,
        source_entity: SketchEntityId,
        constraint_id: SketchConstraintId,
    ) -> Result<(), SketchError> {
        if source_feature.0 == 0 || source_entity.0 == 0 {
            return Err(SketchError::InvalidProjectionSource);
        }
        if entity.id().0 == 0 {
            return Err(SketchError::ReservedEntityId);
        }
        if constraint_id.0 == 0 {
            return Err(SketchError::ReservedConstraintId);
        }
        if self.entities.len() >= MAX_SKETCH_ENTITIES
            || self.constraints.len() >= MAX_SKETCH_CONSTRAINTS
        {
            return Err(SketchError::ResourceLimit);
        }
        if self
            .entities
            .iter()
            .any(|candidate| candidate.id() == entity.id())
        {
            return Err(SketchError::EntityIdCollision(entity.id()));
        }
        if self
            .constraints
            .iter()
            .any(|constraint| constraint.id == constraint_id)
        {
            return Err(SketchError::ConstraintIdCollision(constraint_id));
        }
        entity.validate()?;
        let mut updated = self.clone();
        updated.entities.push(entity.clone());
        updated.entities.sort_by_key(SketchEntity::id);
        updated.constraints.push(SketchConstraint {
            id: constraint_id,
            kind: SketchConstraintKind::Projection {
                entity: entity.id(),
                source_feature,
                source_entity,
                target: Box::new(entity),
            },
        });
        updated.constraints.sort_by_key(|constraint| constraint.id);
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn refresh_projection(
        &mut self,
        entity_id: SketchEntityId,
        target: SketchEntity,
    ) -> Result<(), SketchError> {
        if target.id() != entity_id {
            return Err(SketchError::InvalidProjectionSource);
        }
        target.validate()?;
        let entity = self
            .entities
            .iter_mut()
            .find(|entity| entity.id() == entity_id)
            .ok_or(SketchError::EntityNotFound(entity_id))?;
        let constraint = self
            .constraints
            .iter_mut()
            .find(|constraint| {
                matches!(
                    constraint.kind,
                    SketchConstraintKind::Projection { entity, .. } if entity == entity_id
                )
            })
            .ok_or(SketchError::InvalidProjectionSource)?;
        let SketchConstraintKind::Projection {
            target: stored_target,
            ..
        } = &mut constraint.kind
        else {
            unreachable!();
        };
        *entity = target.clone();
        **stored_target = target;
        self.solve()?;
        Ok(())
    }

    pub fn split_entity(
        &mut self,
        entity_id: SketchEntityId,
        new_entity_id: SketchEntityId,
        parameter: f64,
        joint_constraint_ids: &[SketchConstraintId],
    ) -> Result<(), SketchError> {
        if self.is_projected_entity(entity_id) {
            return Err(SketchError::ProjectedEntityReadOnly(entity_id));
        }
        if !parameter.is_finite() || parameter <= EPSILON_MM || parameter >= 1.0 - EPSILON_MM {
            return Err(SketchError::InvalidSplitParameter);
        }
        if new_entity_id.0 == 0 {
            return Err(SketchError::ReservedEntityId);
        }
        if self.entities.len() >= MAX_SKETCH_ENTITIES {
            return Err(SketchError::ResourceLimit);
        }
        if self
            .entities
            .iter()
            .any(|entity| entity.id() == new_entity_id)
        {
            return Err(SketchError::EntityIdCollision(new_entity_id));
        }
        let source_index = self
            .entities
            .iter()
            .position(|entity| entity.id() == entity_id)
            .ok_or(SketchError::EntityNotFound(entity_id))?;
        let source = self.entities[source_index].clone();
        let required_joint_count = usize::from(matches!(source, SketchEntity::Circle { .. })) + 1;
        if joint_constraint_ids.len() != required_joint_count
            || joint_constraint_ids.iter().any(|id| id.0 == 0)
            || joint_constraint_ids.windows(2).any(|ids| ids[0] >= ids[1])
            || joint_constraint_ids.iter().any(|id| {
                self.constraints
                    .iter()
                    .any(|constraint| constraint.id == *id)
            })
            || self
                .constraints
                .len()
                .checked_add(required_joint_count)
                .is_none_or(|count| count > MAX_SKETCH_CONSTRAINTS)
        {
            return Err(SketchError::InvalidSplitConstraintIds);
        }

        let remap_point = |reference: &mut SketchPointRef| -> Result<(), SketchError> {
            if reference.entity != entity_id {
                return Ok(());
            }
            match reference.point {
                SketchPointKind::Start => Ok(()),
                SketchPointKind::End => {
                    reference.entity = new_entity_id;
                    Ok(())
                }
                SketchPointKind::Center | SketchPointKind::Control1 | SketchPointKind::Control2 => {
                    Err(SketchError::AmbiguousSplitConstraint)
                }
            }
        };
        let mut constraints = self.constraints.clone();
        for constraint in &mut constraints {
            match &mut constraint.kind {
                SketchConstraintKind::Horizontal { entity }
                | SketchConstraintKind::Vertical { entity }
                | SketchConstraintKind::Radius { entity, .. }
                    if *entity == entity_id =>
                {
                    return Err(SketchError::AmbiguousSplitConstraint);
                }
                SketchConstraintKind::Coincident { a, b }
                | SketchConstraintKind::Distance { a, b, .. } => {
                    remap_point(a)?;
                    remap_point(b)?;
                }
                SketchConstraintKind::FixedPoint { point, .. } => remap_point(point)?,
                SketchConstraintKind::Parallel { a, b }
                | SketchConstraintKind::Perpendicular { a, b }
                | SketchConstraintKind::Tangent { a, b }
                | SketchConstraintKind::Angle { a, b, .. }
                | SketchConstraintKind::Equal { a, b }
                | SketchConstraintKind::Concentric { a, b }
                | SketchConstraintKind::Collinear { a, b }
                    if *a == entity_id || *b == entity_id =>
                {
                    return Err(SketchError::AmbiguousSplitConstraint);
                }
                SketchConstraintKind::Symmetric { a, b, axis } => {
                    if *axis == entity_id {
                        return Err(SketchError::AmbiguousSplitConstraint);
                    }
                    remap_point(a)?;
                    remap_point(b)?;
                }
                SketchConstraintKind::Midpoint { point, line } => {
                    if *line == entity_id {
                        return Err(SketchError::AmbiguousSplitConstraint);
                    }
                    remap_point(point)?;
                }
                SketchConstraintKind::PointOnCurve { point, curve } => {
                    if *curve == entity_id {
                        return Err(SketchError::AmbiguousSplitConstraint);
                    }
                    remap_point(point)?;
                }
                SketchConstraintKind::Projection { .. } => {}
                SketchConstraintKind::Construction { entity } if *entity == entity_id => {
                    return Err(SketchError::AmbiguousSplitConstraint);
                }
                SketchConstraintKind::Horizontal { .. }
                | SketchConstraintKind::Vertical { .. }
                | SketchConstraintKind::Radius { .. }
                | SketchConstraintKind::Parallel { .. }
                | SketchConstraintKind::Perpendicular { .. }
                | SketchConstraintKind::Tangent { .. }
                | SketchConstraintKind::Angle { .. }
                | SketchConstraintKind::Equal { .. }
                | SketchConstraintKind::Concentric { .. }
                | SketchConstraintKind::Collinear { .. }
                | SketchConstraintKind::Construction { .. } => {}
            }
        }

        let lerp =
            |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let (first, second, joints) = match source {
            SketchEntity::Line {
                id,
                start_mm,
                end_mm,
            } => {
                let split = lerp(start_mm, end_mm, parameter);
                (
                    SketchEntity::Line {
                        id,
                        start_mm,
                        end_mm: split,
                    },
                    SketchEntity::Line {
                        id: new_entity_id,
                        start_mm: split,
                        end_mm,
                    },
                    vec![(
                        SketchPointRef {
                            entity: id,
                            point: SketchPointKind::End,
                        },
                        SketchPointRef {
                            entity: new_entity_id,
                            point: SketchPointKind::Start,
                        },
                    )],
                )
            }
            SketchEntity::Arc {
                id,
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let sweep = if clockwise {
                    -((start_angle - end_angle).rem_euclid(std::f64::consts::TAU))
                } else {
                    (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
                };
                let angle = start_angle + sweep * parameter;
                let radius = distance2(start_mm, center_mm);
                let split = [
                    center_mm[0] + radius * angle.cos(),
                    center_mm[1] + radius * angle.sin(),
                ];
                (
                    SketchEntity::Arc {
                        id,
                        start_mm,
                        end_mm: split,
                        center_mm,
                        clockwise,
                    },
                    SketchEntity::Arc {
                        id: new_entity_id,
                        start_mm: split,
                        end_mm,
                        center_mm,
                        clockwise,
                    },
                    vec![(
                        SketchPointRef {
                            entity: id,
                            point: SketchPointKind::End,
                        },
                        SketchPointRef {
                            entity: new_entity_id,
                            point: SketchPointKind::Start,
                        },
                    )],
                )
            }
            SketchEntity::Circle {
                id,
                center_mm,
                radius_mm,
            } => {
                if constraints.iter().any(|constraint| match &constraint.kind {
                    SketchConstraintKind::Radius { entity, .. } => *entity == entity_id,
                    SketchConstraintKind::Concentric { a, b } => *a == entity_id || *b == entity_id,
                    SketchConstraintKind::FixedPoint { point, .. }
                    | SketchConstraintKind::Midpoint { point, .. }
                    | SketchConstraintKind::PointOnCurve { point, .. } => point.entity == entity_id,
                    SketchConstraintKind::Coincident { a, b }
                    | SketchConstraintKind::Distance { a, b, .. } => {
                        a.entity == entity_id || b.entity == entity_id
                    }
                    SketchConstraintKind::Symmetric { a, b, axis } => {
                        a.entity == entity_id || b.entity == entity_id || *axis == entity_id
                    }
                    SketchConstraintKind::Horizontal { entity }
                    | SketchConstraintKind::Vertical { entity } => *entity == entity_id,
                    SketchConstraintKind::Parallel { a, b }
                    | SketchConstraintKind::Perpendicular { a, b }
                    | SketchConstraintKind::Tangent { a, b }
                    | SketchConstraintKind::Angle { a, b, .. }
                    | SketchConstraintKind::Equal { a, b }
                    | SketchConstraintKind::Collinear { a, b } => {
                        *a == entity_id || *b == entity_id
                    }
                    SketchConstraintKind::Projection { entity, .. }
                    | SketchConstraintKind::Construction { entity } => *entity == entity_id,
                }) {
                    return Err(SketchError::AmbiguousSplitConstraint);
                }
                let angle = parameter * std::f64::consts::TAU;
                let opposite_angle = angle + std::f64::consts::PI;
                let split = [
                    center_mm[0] + radius_mm * angle.cos(),
                    center_mm[1] + radius_mm * angle.sin(),
                ];
                let opposite = [
                    center_mm[0] + radius_mm * opposite_angle.cos(),
                    center_mm[1] + radius_mm * opposite_angle.sin(),
                ];
                (
                    SketchEntity::Arc {
                        id,
                        start_mm: split,
                        end_mm: opposite,
                        center_mm,
                        clockwise: false,
                    },
                    SketchEntity::Arc {
                        id: new_entity_id,
                        start_mm: opposite,
                        end_mm: split,
                        center_mm,
                        clockwise: false,
                    },
                    vec![
                        (
                            SketchPointRef {
                                entity: id,
                                point: SketchPointKind::End,
                            },
                            SketchPointRef {
                                entity: new_entity_id,
                                point: SketchPointKind::Start,
                            },
                        ),
                        (
                            SketchPointRef {
                                entity: new_entity_id,
                                point: SketchPointKind::End,
                            },
                            SketchPointRef {
                                entity: id,
                                point: SketchPointKind::Start,
                            },
                        ),
                    ],
                )
            }
            SketchEntity::CubicBezier {
                id,
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                let p01 = lerp(start_mm, control_1_mm, parameter);
                let p12 = lerp(control_1_mm, control_2_mm, parameter);
                let p23 = lerp(control_2_mm, end_mm, parameter);
                let p012 = lerp(p01, p12, parameter);
                let p123 = lerp(p12, p23, parameter);
                let split = lerp(p012, p123, parameter);
                (
                    SketchEntity::CubicBezier {
                        id,
                        start_mm,
                        control_1_mm: p01,
                        control_2_mm: p012,
                        end_mm: split,
                    },
                    SketchEntity::CubicBezier {
                        id: new_entity_id,
                        start_mm: split,
                        control_1_mm: p123,
                        control_2_mm: p23,
                        end_mm,
                    },
                    vec![(
                        SketchPointRef {
                            entity: id,
                            point: SketchPointKind::End,
                        },
                        SketchPointRef {
                            entity: new_entity_id,
                            point: SketchPointKind::Start,
                        },
                    )],
                )
            }
        };

        let mut entities = self.entities.clone();
        entities[source_index] = first;
        entities.push(second);
        entities.sort_by_key(SketchEntity::id);
        for (id, (a, b)) in joint_constraint_ids.iter().copied().zip(joints) {
            constraints.push(SketchConstraint {
                id,
                kind: SketchConstraintKind::Coincident { a, b },
            });
        }
        constraints.sort_by_key(|constraint| constraint.id);
        let updated = Self {
            workplane: self.workplane,
            entities,
            constraints,
        };
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn join_entities(
        &mut self,
        source_id: SketchEntityId,
        source_endpoint: SketchPointKind,
        consumed_id: SketchEntityId,
        consumed_endpoint: SketchPointKind,
    ) -> Result<(), SketchError> {
        if source_id == consumed_id
            || !matches!(
                source_endpoint,
                SketchPointKind::Start | SketchPointKind::End
            )
            || !matches!(
                consumed_endpoint,
                SketchPointKind::Start | SketchPointKind::End
            )
        {
            return Err(SketchError::IncompatibleJoinGeometry);
        }
        let source_index = self
            .entities
            .iter()
            .position(|entity| entity.id() == source_id)
            .ok_or(SketchError::EntityNotFound(source_id))?;
        let consumed_index = self
            .entities
            .iter()
            .position(|entity| entity.id() == consumed_id)
            .ok_or(SketchError::EntityNotFound(consumed_id))?;
        if self.is_projected_entity(source_id) {
            return Err(SketchError::ProjectedEntityReadOnly(source_id));
        }
        if self.is_projected_entity(consumed_id) {
            return Err(SketchError::ProjectedEntityReadOnly(consumed_id));
        }
        let source = self.entities[source_index].clone();
        let consumed = self.entities[consumed_index].clone();
        if matches!(source, SketchEntity::Circle { .. }) {
            return Err(SketchError::UnsupportedJoinEntity(source_id));
        }
        if matches!(consumed, SketchEntity::Circle { .. }) {
            return Err(SketchError::UnsupportedJoinEntity(consumed_id));
        }
        let source_joint = source
            .point(source_endpoint)
            .ok_or(SketchError::IncompatibleJoinGeometry)?;
        let consumed_joint = consumed
            .point(consumed_endpoint)
            .ok_or(SketchError::IncompatibleJoinGeometry)?;
        if distance2(source_joint, consumed_joint) > EPSILON_MM {
            return Err(SketchError::IncompatibleJoinGeometry);
        }

        let source_joins_at_end = source_endpoint == SketchPointKind::End;
        let consumed_joins_at_start = consumed_endpoint == SketchPointKind::Start;
        let joined = match (source, consumed) {
            (
                SketchEntity::Line {
                    start_mm: source_start,
                    end_mm: source_end,
                    ..
                },
                SketchEntity::Line {
                    start_mm: consumed_start,
                    end_mm: consumed_end,
                    ..
                },
            ) => {
                let source_outer = if source_joins_at_end {
                    source_start
                } else {
                    source_end
                };
                let consumed_outer = if consumed_joins_at_start {
                    consumed_end
                } else {
                    consumed_start
                };
                let (start_mm, end_mm) = if source_joins_at_end {
                    (source_outer, consumed_outer)
                } else {
                    (consumed_outer, source_outer)
                };
                let left = subtract2(source_joint, start_mm);
                let right = subtract2(end_mm, source_joint);
                if point_line_distance(source_joint, start_mm, end_mm) > EPSILON_MM
                    || left[0] * right[0] + left[1] * right[1] <= 0.0
                {
                    return Err(SketchError::IncompatibleJoinGeometry);
                }
                SketchEntity::Line {
                    id: source_id,
                    start_mm,
                    end_mm,
                }
            }
            (
                SketchEntity::Arc {
                    start_mm: source_start,
                    end_mm: source_end,
                    center_mm: source_center,
                    clockwise: source_clockwise,
                    ..
                },
                SketchEntity::Arc {
                    start_mm: consumed_start,
                    end_mm: consumed_end,
                    center_mm: consumed_center,
                    clockwise: consumed_clockwise,
                    ..
                },
            ) => {
                let (consumed_start, consumed_end, consumed_clockwise) =
                    if consumed_joins_at_start == source_joins_at_end {
                        (consumed_start, consumed_end, consumed_clockwise)
                    } else {
                        (consumed_end, consumed_start, !consumed_clockwise)
                    };
                if distance2(source_center, consumed_center) > EPSILON_MM
                    || source_clockwise != consumed_clockwise
                    || (distance2(source_start, source_center)
                        - distance2(consumed_start, consumed_center))
                    .abs()
                        > EPSILON_MM
                {
                    return Err(SketchError::IncompatibleJoinGeometry);
                }
                let arc_sweep = |start: [f64; 2], end: [f64; 2], clockwise: bool| {
                    let start_angle =
                        (start[1] - source_center[1]).atan2(start[0] - source_center[0]);
                    let end_angle = (end[1] - source_center[1]).atan2(end[0] - source_center[0]);
                    if clockwise {
                        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
                    } else {
                        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
                    }
                };
                let sweep = arc_sweep(source_start, source_end, source_clockwise)
                    + arc_sweep(consumed_start, consumed_end, consumed_clockwise);
                if sweep >= std::f64::consts::TAU - EPSILON_MM {
                    return Err(SketchError::IncompatibleJoinGeometry);
                }
                SketchEntity::Arc {
                    id: source_id,
                    start_mm: if source_joins_at_end {
                        source_start
                    } else {
                        consumed_start
                    },
                    end_mm: if source_joins_at_end {
                        consumed_end
                    } else {
                        source_end
                    },
                    center_mm: source_center,
                    clockwise: source_clockwise,
                }
            }
            (
                SketchEntity::CubicBezier {
                    start_mm: source_start,
                    control_1_mm: source_control_1,
                    control_2_mm: source_control_2,
                    end_mm: source_end,
                    ..
                },
                SketchEntity::CubicBezier {
                    start_mm: consumed_start,
                    control_1_mm: consumed_control_1,
                    control_2_mm: consumed_control_2,
                    end_mm: consumed_end,
                    ..
                },
            ) => {
                let source_points = [source_start, source_control_1, source_control_2, source_end];
                let consumed_points = if consumed_joins_at_start == source_joins_at_end {
                    [
                        consumed_start,
                        consumed_control_1,
                        consumed_control_2,
                        consumed_end,
                    ]
                } else {
                    [
                        consumed_end,
                        consumed_control_2,
                        consumed_control_1,
                        consumed_start,
                    ]
                };
                let (left, right) = if source_joins_at_end {
                    (source_points, consumed_points)
                } else {
                    (consumed_points, source_points)
                };
                let left_tangent = subtract2(left[3], left[2]);
                let right_tangent = subtract2(right[1], right[0]);
                let left_length = distance2(left[3], left[2]);
                let right_length = distance2(right[1], right[0]);
                if left_length <= EPSILON_MM
                    || right_length <= EPSILON_MM
                    || cross2(left_tangent, right_tangent).abs()
                        > EPSILON_MM * left_length.max(right_length)
                    || left_tangent[0] * right_tangent[0] + left_tangent[1] * right_tangent[1]
                        <= 0.0
                {
                    return Err(SketchError::IncompatibleJoinGeometry);
                }
                let parameter = left_length / (left_length + right_length);
                let inverse = |origin: [f64; 2], point: [f64; 2], scale: f64| {
                    [
                        origin[0] + (point[0] - origin[0]) / scale,
                        origin[1] + (point[1] - origin[1]) / scale,
                    ]
                };
                let control_1 = inverse(left[0], left[1], parameter);
                let control_2 = inverse(right[3], right[2], 1.0 - parameter);
                let lerp = |a: [f64; 2], b: [f64; 2], t: f64| {
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                };
                let p01 = lerp(left[0], control_1, parameter);
                let p12 = lerp(control_1, control_2, parameter);
                let p23 = lerp(control_2, right[3], parameter);
                let p012 = lerp(p01, p12, parameter);
                let p123 = lerp(p12, p23, parameter);
                let joint = lerp(p012, p123, parameter);
                let reconstructed_left = [left[0], p01, p012, joint];
                let reconstructed_right = [joint, p123, p23, right[3]];
                if reconstructed_left
                    .iter()
                    .zip(left)
                    .chain(reconstructed_right.iter().zip(right))
                    .any(|(actual, expected)| distance2(*actual, expected) > EPSILON_MM)
                {
                    return Err(SketchError::IncompatibleJoinGeometry);
                }
                SketchEntity::CubicBezier {
                    id: source_id,
                    start_mm: left[0],
                    control_1_mm: control_1,
                    control_2_mm: control_2,
                    end_mm: right[3],
                }
            }
            (SketchEntity::Circle { .. }, _) => {
                return Err(SketchError::UnsupportedJoinEntity(source_id));
            }
            (_, SketchEntity::Circle { .. }) => {
                return Err(SketchError::UnsupportedJoinEntity(consumed_id));
            }
            _ => return Err(SketchError::IncompatibleJoinGeometry),
        };

        let source_joint_ref = SketchPointRef {
            entity: source_id,
            point: source_endpoint,
        };
        let consumed_joint_ref = SketchPointRef {
            entity: consumed_id,
            point: consumed_endpoint,
        };
        let remap_point = |reference: &mut SketchPointRef| -> Result<(), SketchError> {
            if reference.entity == source_id {
                if reference.point == source_endpoint
                    || !matches!(
                        reference.point,
                        SketchPointKind::Start | SketchPointKind::End
                    )
                {
                    return Err(SketchError::AmbiguousJoinConstraint);
                }
            } else if reference.entity == consumed_id {
                if reference.point == consumed_endpoint
                    || !matches!(
                        reference.point,
                        SketchPointKind::Start | SketchPointKind::End
                    )
                {
                    return Err(SketchError::AmbiguousJoinConstraint);
                }
                reference.entity = source_id;
                reference.point = source_endpoint;
            }
            Ok(())
        };
        let touches_joined = |entity: SketchEntityId| entity == source_id || entity == consumed_id;
        let mut constraints = Vec::with_capacity(self.constraints.len());
        for constraint in &self.constraints {
            if matches!(
                constraint.kind,
                SketchConstraintKind::Coincident { a, b }
                    if (a == source_joint_ref && b == consumed_joint_ref)
                        || (a == consumed_joint_ref && b == source_joint_ref)
            ) {
                continue;
            }
            let mut constraint = constraint.clone();
            match &mut constraint.kind {
                SketchConstraintKind::Horizontal { entity }
                | SketchConstraintKind::Vertical { entity }
                | SketchConstraintKind::Radius { entity, .. }
                | SketchConstraintKind::Projection { entity, .. }
                | SketchConstraintKind::Construction { entity }
                    if touches_joined(*entity) =>
                {
                    return Err(SketchError::AmbiguousJoinConstraint);
                }
                SketchConstraintKind::Coincident { a, b }
                | SketchConstraintKind::Distance { a, b, .. } => {
                    remap_point(a)?;
                    remap_point(b)?;
                }
                SketchConstraintKind::FixedPoint { point, .. } => remap_point(point)?,
                SketchConstraintKind::Parallel { a, b }
                | SketchConstraintKind::Perpendicular { a, b }
                | SketchConstraintKind::Tangent { a, b }
                | SketchConstraintKind::Angle { a, b, .. }
                | SketchConstraintKind::Equal { a, b }
                | SketchConstraintKind::Concentric { a, b }
                | SketchConstraintKind::Collinear { a, b }
                    if touches_joined(*a) || touches_joined(*b) =>
                {
                    return Err(SketchError::AmbiguousJoinConstraint);
                }
                SketchConstraintKind::Symmetric { a, b, axis } => {
                    if touches_joined(*axis) {
                        return Err(SketchError::AmbiguousJoinConstraint);
                    }
                    remap_point(a)?;
                    remap_point(b)?;
                }
                SketchConstraintKind::Midpoint { point, line } => {
                    if touches_joined(*line) {
                        return Err(SketchError::AmbiguousJoinConstraint);
                    }
                    remap_point(point)?;
                }
                SketchConstraintKind::PointOnCurve { point, curve } => {
                    if touches_joined(*curve) {
                        return Err(SketchError::AmbiguousJoinConstraint);
                    }
                    remap_point(point)?;
                }
                SketchConstraintKind::Horizontal { .. }
                | SketchConstraintKind::Vertical { .. }
                | SketchConstraintKind::Radius { .. }
                | SketchConstraintKind::Parallel { .. }
                | SketchConstraintKind::Perpendicular { .. }
                | SketchConstraintKind::Tangent { .. }
                | SketchConstraintKind::Angle { .. }
                | SketchConstraintKind::Equal { .. }
                | SketchConstraintKind::Concentric { .. }
                | SketchConstraintKind::Collinear { .. }
                | SketchConstraintKind::Projection { .. }
                | SketchConstraintKind::Construction { .. } => {}
            }
            constraints.push(constraint);
        }

        let mut entities = self.entities.clone();
        entities[source_index] = joined;
        entities.remove(consumed_index);
        entities.sort_by_key(SketchEntity::id);
        let updated = Self {
            workplane: self.workplane,
            entities,
            constraints,
        };
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn trim_entity(
        &mut self,
        entity_id: SketchEntityId,
        start_parameter: f64,
        end_parameter: f64,
    ) -> Result<(), SketchError> {
        if self.is_projected_entity(entity_id) {
            return Err(SketchError::ProjectedEntityReadOnly(entity_id));
        }
        if !start_parameter.is_finite()
            || !end_parameter.is_finite()
            || start_parameter < 0.0
            || end_parameter > 1.0
            || end_parameter - start_parameter <= EPSILON_MM
            || (start_parameter == 0.0 && end_parameter == 1.0)
        {
            return Err(SketchError::InvalidTrimInterval);
        }
        let source_index = self
            .entities
            .iter()
            .position(|entity| entity.id() == entity_id)
            .ok_or(SketchError::EntityNotFound(entity_id))?;
        let point_is_removed = |point: SketchPointRef| {
            point.entity == entity_id
                && match point.point {
                    SketchPointKind::Start => start_parameter > 0.0,
                    SketchPointKind::End => end_parameter < 1.0,
                    SketchPointKind::Center => false,
                    SketchPointKind::Control1 | SketchPointKind::Control2 => true,
                }
        };
        for constraint in &self.constraints {
            let ambiguous = match &constraint.kind {
                SketchConstraintKind::Horizontal { entity }
                | SketchConstraintKind::Vertical { entity }
                | SketchConstraintKind::Radius { entity, .. } => *entity == entity_id,
                SketchConstraintKind::Coincident { a, b }
                | SketchConstraintKind::Distance { a, b, .. } => {
                    point_is_removed(*a) || point_is_removed(*b)
                }
                SketchConstraintKind::FixedPoint { point, .. } => point_is_removed(*point),
                SketchConstraintKind::Parallel { a, b }
                | SketchConstraintKind::Perpendicular { a, b }
                | SketchConstraintKind::Tangent { a, b }
                | SketchConstraintKind::Angle { a, b, .. }
                | SketchConstraintKind::Equal { a, b }
                | SketchConstraintKind::Concentric { a, b }
                | SketchConstraintKind::Collinear { a, b } => *a == entity_id || *b == entity_id,
                SketchConstraintKind::Symmetric { a, b, axis } => {
                    point_is_removed(*a) || point_is_removed(*b) || *axis == entity_id
                }
                SketchConstraintKind::Midpoint { point, line } => {
                    point_is_removed(*point) || *line == entity_id
                }
                SketchConstraintKind::PointOnCurve { point, curve } => {
                    point_is_removed(*point) || *curve == entity_id
                }
                SketchConstraintKind::Projection { entity, .. } => *entity == entity_id,
                SketchConstraintKind::Construction { .. } => false,
            };
            if ambiguous {
                return Err(SketchError::AmbiguousTrimConstraint);
            }
        }

        let lerp =
            |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let arc_point = |start_mm: [f64; 2],
                         end_mm: [f64; 2],
                         center_mm: [f64; 2],
                         clockwise: bool,
                         parameter: f64| {
            let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
            let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
            let sweep = if clockwise {
                -((start_angle - end_angle).rem_euclid(std::f64::consts::TAU))
            } else {
                (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
            };
            let angle = start_angle + sweep * parameter;
            let radius = distance2(start_mm, center_mm);
            [
                center_mm[0] + radius * angle.cos(),
                center_mm[1] + radius * angle.sin(),
            ]
        };
        let trimmed = match self.entities[source_index].clone() {
            SketchEntity::Line {
                id,
                start_mm,
                end_mm,
            } => SketchEntity::Line {
                id,
                start_mm: lerp(start_mm, end_mm, start_parameter),
                end_mm: lerp(start_mm, end_mm, end_parameter),
            },
            SketchEntity::Arc {
                id,
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => SketchEntity::Arc {
                id,
                start_mm: arc_point(start_mm, end_mm, center_mm, clockwise, start_parameter),
                end_mm: arc_point(start_mm, end_mm, center_mm, clockwise, end_parameter),
                center_mm,
                clockwise,
            },
            SketchEntity::Circle {
                id,
                center_mm,
                radius_mm,
            } => {
                let point = |parameter: f64| {
                    let angle = parameter * std::f64::consts::TAU;
                    [
                        center_mm[0] + radius_mm * angle.cos(),
                        center_mm[1] + radius_mm * angle.sin(),
                    ]
                };
                SketchEntity::Arc {
                    id,
                    start_mm: point(start_parameter),
                    end_mm: point(end_parameter),
                    center_mm,
                    clockwise: false,
                }
            }
            SketchEntity::CubicBezier {
                id,
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                let split = |points: [[f64; 2]; 4], parameter: f64| {
                    let p01 = lerp(points[0], points[1], parameter);
                    let p12 = lerp(points[1], points[2], parameter);
                    let p23 = lerp(points[2], points[3], parameter);
                    let p012 = lerp(p01, p12, parameter);
                    let p123 = lerp(p12, p23, parameter);
                    let joint = lerp(p012, p123, parameter);
                    ([points[0], p01, p012, joint], [joint, p123, p23, points[3]])
                };
                let points = [start_mm, control_1_mm, control_2_mm, end_mm];
                let through_end = if end_parameter < 1.0 {
                    split(points, end_parameter).0
                } else {
                    points
                };
                let trimmed = if start_parameter > 0.0 {
                    split(through_end, start_parameter / end_parameter).1
                } else {
                    through_end
                };
                SketchEntity::CubicBezier {
                    id,
                    start_mm: trimmed[0],
                    control_1_mm: trimmed[1],
                    control_2_mm: trimmed[2],
                    end_mm: trimmed[3],
                }
            }
        };
        let mut updated = self.clone();
        updated.entities[source_index] = trimmed;
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn extend_entity(
        &mut self,
        entity_id: SketchEntityId,
        endpoint: SketchPointKind,
        parameter: f64,
    ) -> Result<(), SketchError> {
        if self.is_projected_entity(entity_id) {
            return Err(SketchError::ProjectedEntityReadOnly(entity_id));
        }
        let extends_start = endpoint == SketchPointKind::Start && parameter < 0.0;
        let extends_end = endpoint == SketchPointKind::End && parameter > 1.0;
        if !parameter.is_finite() || (!extends_start && !extends_end) {
            return Err(SketchError::InvalidExtendParameter);
        }
        let source_index = self
            .entities
            .iter()
            .position(|entity| entity.id() == entity_id)
            .ok_or(SketchError::EntityNotFound(entity_id))?;
        if matches!(self.entities[source_index], SketchEntity::Circle { .. }) {
            return Err(SketchError::UnsupportedExtendEntity(entity_id));
        }
        let point_is_changed = |point: SketchPointRef| {
            point.entity == entity_id
                && match point.point {
                    SketchPointKind::Start => extends_start,
                    SketchPointKind::End => extends_end,
                    SketchPointKind::Center => false,
                    SketchPointKind::Control1 | SketchPointKind::Control2 => true,
                }
        };
        for constraint in &self.constraints {
            let ambiguous = match &constraint.kind {
                SketchConstraintKind::Horizontal { entity }
                | SketchConstraintKind::Vertical { entity }
                | SketchConstraintKind::Radius { entity, .. } => *entity == entity_id,
                SketchConstraintKind::Coincident { a, b }
                | SketchConstraintKind::Distance { a, b, .. } => {
                    point_is_changed(*a) || point_is_changed(*b)
                }
                SketchConstraintKind::FixedPoint { point, .. } => point_is_changed(*point),
                SketchConstraintKind::Parallel { a, b }
                | SketchConstraintKind::Perpendicular { a, b }
                | SketchConstraintKind::Tangent { a, b }
                | SketchConstraintKind::Angle { a, b, .. }
                | SketchConstraintKind::Equal { a, b }
                | SketchConstraintKind::Concentric { a, b }
                | SketchConstraintKind::Collinear { a, b } => *a == entity_id || *b == entity_id,
                SketchConstraintKind::Symmetric { a, b, axis } => {
                    point_is_changed(*a) || point_is_changed(*b) || *axis == entity_id
                }
                SketchConstraintKind::Midpoint { point, line } => {
                    point_is_changed(*point) || *line == entity_id
                }
                SketchConstraintKind::PointOnCurve { point, curve } => {
                    point_is_changed(*point) || *curve == entity_id
                }
                SketchConstraintKind::Projection { entity, .. } => *entity == entity_id,
                SketchConstraintKind::Construction { .. } => false,
            };
            if ambiguous {
                return Err(SketchError::AmbiguousExtendConstraint);
            }
        }

        let lerp =
            |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let extended = match self.entities[source_index].clone() {
            SketchEntity::Line {
                id,
                start_mm,
                end_mm,
            } => SketchEntity::Line {
                id,
                start_mm: if extends_start {
                    lerp(start_mm, end_mm, parameter)
                } else {
                    start_mm
                },
                end_mm: if extends_end {
                    lerp(start_mm, end_mm, parameter)
                } else {
                    end_mm
                },
            },
            SketchEntity::Arc {
                id,
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let sweep = if clockwise {
                    -((start_angle - end_angle).rem_euclid(std::f64::consts::TAU))
                } else {
                    (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
                };
                let angle = start_angle + sweep * parameter;
                let radius = distance2(start_mm, center_mm);
                let point = [
                    center_mm[0] + radius * angle.cos(),
                    center_mm[1] + radius * angle.sin(),
                ];
                SketchEntity::Arc {
                    id,
                    start_mm: if extends_start { point } else { start_mm },
                    end_mm: if extends_end { point } else { end_mm },
                    center_mm,
                    clockwise,
                }
            }
            SketchEntity::CubicBezier {
                id,
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                let points = [start_mm, control_1_mm, control_2_mm, end_mm];
                let p01 = lerp(points[0], points[1], parameter);
                let p12 = lerp(points[1], points[2], parameter);
                let p23 = lerp(points[2], points[3], parameter);
                let p012 = lerp(p01, p12, parameter);
                let p123 = lerp(p12, p23, parameter);
                let joint = lerp(p012, p123, parameter);
                let extended = if extends_start {
                    [joint, p123, p23, points[3]]
                } else {
                    [points[0], p01, p012, joint]
                };
                SketchEntity::CubicBezier {
                    id,
                    start_mm: extended[0],
                    control_1_mm: extended[1],
                    control_2_mm: extended[2],
                    end_mm: extended[3],
                }
            }
            SketchEntity::Circle { .. } => unreachable!(),
        };
        let mut updated = self.clone();
        updated.entities[source_index] = extended;
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn offset_entity(
        &mut self,
        entity_id: SketchEntityId,
        new_entity_id: SketchEntityId,
        distance_mm: f64,
        side: SketchOffsetSide,
    ) -> Result<(), SketchError> {
        if self.is_projected_entity(entity_id) {
            return Err(SketchError::ProjectedEntityReadOnly(entity_id));
        }
        if !distance_mm.is_finite() || distance_mm <= EPSILON_MM || distance_mm > MAX_COORDINATE_MM
        {
            return Err(SketchError::InvalidOffsetDistance);
        }
        if new_entity_id.0 == 0 {
            return Err(SketchError::ReservedEntityId);
        }
        if self.entities.len() >= MAX_SKETCH_ENTITIES {
            return Err(SketchError::ResourceLimit);
        }
        if self
            .entities
            .iter()
            .any(|entity| entity.id() == new_entity_id)
        {
            return Err(SketchError::EntityIdCollision(new_entity_id));
        }
        let source = self
            .entities
            .iter()
            .find(|entity| entity.id() == entity_id)
            .cloned()
            .ok_or(SketchError::EntityNotFound(entity_id))?;
        let side_sign = match side {
            SketchOffsetSide::Left => 1.0,
            SketchOffsetSide::Right => -1.0,
        };
        let left_offset = |tangent: [f64; 2]| -> Result<[f64; 2], SketchError> {
            let length = tangent[0].hypot(tangent[1]);
            if !length.is_finite() || length <= EPSILON_MM {
                return Err(SketchError::AmbiguousOffset);
            }
            Ok([
                -tangent[1] / length * distance_mm * side_sign,
                tangent[0] / length * distance_mm * side_sign,
            ])
        };
        let translate =
            |point: [f64; 2], offset: [f64; 2]| [point[0] + offset[0], point[1] + offset[1]];
        let radial_point = |point: [f64; 2], center: [f64; 2], radius: f64| {
            let source_radius = distance2(point, center);
            [
                center[0] + (point[0] - center[0]) * radius / source_radius,
                center[1] + (point[1] - center[1]) * radius / source_radius,
            ]
        };

        let offset = match source {
            SketchEntity::Line {
                start_mm, end_mm, ..
            } => {
                let normal = left_offset([end_mm[0] - start_mm[0], end_mm[1] - start_mm[1]])?;
                SketchEntity::Line {
                    id: new_entity_id,
                    start_mm: translate(start_mm, normal),
                    end_mm: translate(end_mm, normal),
                }
            }
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
                ..
            } => {
                let source_radius = distance2(start_mm, center_mm);
                let radial_direction = if clockwise { side_sign } else { -side_sign };
                let radius = source_radius + distance_mm * radial_direction;
                if !radius.is_finite() || radius <= EPSILON_MM || radius > MAX_COORDINATE_MM {
                    return Err(SketchError::AmbiguousOffset);
                }
                SketchEntity::Arc {
                    id: new_entity_id,
                    start_mm: radial_point(start_mm, center_mm, radius),
                    end_mm: radial_point(end_mm, center_mm, radius),
                    center_mm,
                    clockwise,
                }
            }
            SketchEntity::Circle {
                center_mm,
                radius_mm,
                ..
            } => {
                let radius = radius_mm - distance_mm * side_sign;
                if !radius.is_finite() || radius <= EPSILON_MM || radius > MAX_COORDINATE_MM {
                    return Err(SketchError::AmbiguousOffset);
                }
                SketchEntity::Circle {
                    id: new_entity_id,
                    center_mm,
                    radius_mm: radius,
                }
            }
            SketchEntity::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => {
                let points = [start_mm, control_1_mm, control_2_mm, end_mm];
                let sample = |parameter: f64| -> Result<[f64; 2], SketchError> {
                    let inverse = 1.0 - parameter;
                    let point = [
                        inverse.powi(3) * points[0][0]
                            + 3.0 * inverse.powi(2) * parameter * points[1][0]
                            + 3.0 * inverse * parameter.powi(2) * points[2][0]
                            + parameter.powi(3) * points[3][0],
                        inverse.powi(3) * points[0][1]
                            + 3.0 * inverse.powi(2) * parameter * points[1][1]
                            + 3.0 * inverse * parameter.powi(2) * points[2][1]
                            + parameter.powi(3) * points[3][1],
                    ];
                    let tangent = [
                        3.0 * inverse.powi(2) * (points[1][0] - points[0][0])
                            + 6.0 * inverse * parameter * (points[2][0] - points[1][0])
                            + 3.0 * parameter.powi(2) * (points[3][0] - points[2][0]),
                        3.0 * inverse.powi(2) * (points[1][1] - points[0][1])
                            + 6.0 * inverse * parameter * (points[2][1] - points[1][1])
                            + 3.0 * parameter.powi(2) * (points[3][1] - points[2][1]),
                    ];
                    Ok(translate(point, left_offset(tangent)?))
                };
                let q0 = sample(0.0)?;
                let q1 = sample(1.0 / 3.0)?;
                let q2 = sample(2.0 / 3.0)?;
                let q3 = sample(1.0)?;
                let a = [
                    27.0 * q1[0] - 8.0 * q0[0] - q3[0],
                    27.0 * q1[1] - 8.0 * q0[1] - q3[1],
                ];
                let b = [
                    27.0 * q2[0] - q0[0] - 8.0 * q3[0],
                    27.0 * q2[1] - q0[1] - 8.0 * q3[1],
                ];
                SketchEntity::CubicBezier {
                    id: new_entity_id,
                    start_mm: q0,
                    control_1_mm: [(2.0 * a[0] - b[0]) / 18.0, (2.0 * a[1] - b[1]) / 18.0],
                    control_2_mm: [(2.0 * b[0] - a[0]) / 18.0, (2.0 * b[1] - a[1]) / 18.0],
                    end_mm: q3,
                }
            }
        };

        let mut updated = self.clone();
        updated.entities.push(offset);
        updated.entities.sort_by_key(SketchEntity::id);
        updated.solve()?;
        *self = updated;
        Ok(())
    }

    pub fn diagnose(&self) -> Result<SketchDiagnosticReport, SketchError> {
        match self.solve() {
            Ok(report) => Ok(SketchDiagnosticReport {
                status: match report.status {
                    SketchSolveStatus::UnderConstrained { remaining_dof } => {
                        SketchDiagnosticStatus::UnderConstrained { remaining_dof }
                    }
                    SketchSolveStatus::FullyConstrained => SketchDiagnosticStatus::FullyConstrained,
                },
                entity_ids: report.unconstrained_entity_ids,
                constraint_ids: Vec::new(),
            }),
            Err(SketchError::OverConstrained(constraint_id)) => {
                let constraint = self
                    .constraints
                    .iter()
                    .find(|constraint| constraint.id == constraint_id)
                    .ok_or(SketchError::InvalidConstraintReference(constraint_id))?;
                Ok(SketchDiagnosticReport {
                    status: SketchDiagnosticStatus::Conflicting,
                    entity_ids: constraint_entity_ids(&constraint.kind),
                    constraint_ids: vec![constraint_id],
                })
            }
            Err(error) => Err(error),
        }
    }

    pub fn solve(&self) -> Result<SketchSolveReport, SketchError> {
        self.solve_with_policy(SketchSolverPolicy::default())
    }

    pub fn solve_with_policy(
        &self,
        policy: SketchSolverPolicy,
    ) -> Result<SketchSolveReport, SketchError> {
        Ok(self.solve_geometry_with_policy(policy)?.report)
    }

    pub fn solve_geometry(&self) -> Result<SketchSolution, SketchError> {
        self.solve_geometry_with_policy(SketchSolverPolicy::default())
    }

    pub fn solve_geometry_with_policy(
        &self,
        policy: SketchSolverPolicy,
    ) -> Result<SketchSolution, SketchError> {
        let policy = policy.validate()?;
        if self.workplane.0 == 0 {
            return Err(SketchError::MissingWorkplaneSupport(self.workplane));
        }
        if self.entities.is_empty()
            || self.entities.len() > MAX_SKETCH_ENTITIES
            || self.constraints.len() > MAX_SKETCH_CONSTRAINTS
        {
            return Err(SketchError::ResourceLimit);
        }
        let mut entities = BTreeMap::new();
        let mut previous_entity = None;
        let mut degrees_of_freedom = 0usize;
        for entity in &self.entities {
            entity.validate()?;
            if previous_entity.is_some_and(|id| id >= entity.id()) {
                return Err(SketchError::EntitiesNotCanonical);
            }
            previous_entity = Some(entity.id());
            degrees_of_freedom = degrees_of_freedom
                .checked_add(entity.degrees_of_freedom())
                .ok_or(SketchError::ResourceLimit)?;
            entities.insert(entity.id(), entity);
        }

        let (variable_layouts, variable_count) = variable_layouts(&self.entities)?;
        debug_assert_eq!(variable_count, degrees_of_freedom);
        let mut rank_equations = Vec::<Vec<usize>>::new();
        let mut constraint_equation_ranges = Vec::new();
        let mut coincidence_parents = BTreeMap::new();
        let mut previous_constraint = None;
        let mut signatures = BTreeSet::new();
        let mut relation_equation_signatures = BTreeSet::new();
        let mut dimensional_constraints = Vec::new();
        let mut equation_count = 0usize;
        for constraint in &self.constraints {
            if constraint.id.0 == 0 {
                return Err(SketchError::ReservedConstraintId);
            }
            if previous_constraint.is_some_and(|id| id >= constraint.id) {
                return Err(SketchError::ConstraintsNotCanonical);
            }
            previous_constraint = Some(constraint.id);
            if let SketchConstraintKind::Coincident { a, b } = &constraint.kind {
                let a_root = coincidence_root(&coincidence_parents, *a);
                let b_root = coincidence_root(&coincidence_parents, *b);
                if a_root == b_root {
                    return Err(SketchError::OverConstrained(constraint.id));
                }
                let (first, second) = canonical_point_pair(a_root, b_root);
                coincidence_parents.insert(second, first);
            }
            let (signature, equations) = evaluate_constraint(constraint, &entities)?;
            if !signatures.insert(signature)
                || overlapping_relation_signature(&constraint.kind)
                    .is_some_and(|signature| !relation_equation_signatures.insert(signature))
            {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            if matches!(
                constraint.kind,
                SketchConstraintKind::Distance { .. }
                    | SketchConstraintKind::Radius { .. }
                    | SketchConstraintKind::FixedPoint { .. }
                    | SketchConstraintKind::Angle { .. }
            ) {
                dimensional_constraints.push(constraint);
            }
            equation_count = equation_count
                .checked_add(equations)
                .ok_or(SketchError::ResourceLimit)?;
            let start = rank_equations.len();
            rank_equations.extend(constraint_variable_equations(
                constraint,
                &variable_layouts,
            )?);
            constraint_equation_ranges.push((constraint.id, start..rank_equations.len()));
        }
        let mut dimensional_targets = BTreeMap::new();
        for constraint in dimensional_constraints {
            let (target, value) =
                dimensional_constraint_target(constraint, &coincidence_parents, &entities)?;
            if let Some((first_id, first_value)) = dimensional_targets.get(&target) {
                if first_value != &value {
                    return Err(SketchError::OverConstrained(*first_id));
                }
            } else {
                dimensional_targets.insert(target, (constraint.id, value));
            }
        }
        let variable_owners = structural_constraint_matching(&rank_equations, variable_count, None);
        let constraint_rank = variable_owners.iter().flatten().count();
        for (constraint_id, range) in &constraint_equation_ranges {
            let rank_without =
                structural_constraint_rank(&rank_equations, variable_count, Some(range.clone()));
            if constraint_rank.saturating_sub(rank_without) < range.len() {
                return Err(SketchError::OverConstrained(*constraint_id));
            }
        }
        // Structural matching is only an early upper-bound check, not a DOF proof.
        let mut solved_entities = self.entities.clone();
        solve_constraints(&mut solved_entities, &self.constraints, policy)?;
        let (status, unconstrained_entity_ids) = rank::analyze(
            &solved_entities,
            &self.constraints,
            &variable_layouts,
            &rank_equations,
            &constraint_equation_ranges,
            variable_count,
        )?;
        for entity in &solved_entities {
            entity.validate()?;
        }
        Ok(SketchSolution {
            report: SketchSolveReport {
                status,
                entity_count: self.entities.len(),
                constraint_count: self.constraints.len(),
                equation_count,
                unconstrained_entity_ids,
            },
            entities: solved_entities,
        })
    }

    pub fn solved_regions(&self) -> Result<Vec<SolvedSketchRegion>, SketchError> {
        let solution = self.solve_geometry()?;
        let construction_entities = self
            .constraints
            .iter()
            .filter_map(|constraint| match constraint.kind {
                SketchConstraintKind::Projection { entity, .. }
                | SketchConstraintKind::Construction { entity } => Some(entity),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut loops = Vec::new();
        let mut boundaries = BTreeMap::new();
        for entity in solution.entities {
            if construction_entities.contains(&entity.id()) {
                continue;
            }
            match entity {
                SketchEntity::Circle {
                    id,
                    center_mm,
                    radius_mm,
                } => loops.push(SolvedSketchLoop {
                    entity_ids: vec![id],
                    ordered_entity_ids: vec![id],
                    profile: SolvedSketchRegionProfile::Circle {
                        center_mm,
                        radius_mm,
                    },
                    area: std::f64::consts::PI * radius_mm * radius_mm,
                }),
                SketchEntity::Line {
                    id,
                    start_mm,
                    end_mm,
                } => {
                    boundaries.insert(id, SolvedSketchRegionEdge::Line { start_mm, end_mm });
                }
                SketchEntity::Arc {
                    id,
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => {
                    boundaries.insert(
                        id,
                        SolvedSketchRegionEdge::Arc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        },
                    );
                }
                SketchEntity::CubicBezier {
                    id,
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => {
                    boundaries.insert(
                        id,
                        SolvedSketchRegionEdge::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        },
                    );
                }
            }
        }

        while let Some((&first_id, first_edge)) = boundaries.first_key_value() {
            let first_edge = first_edge.clone();
            boundaries.remove(&first_id);
            let first_start = first_edge.start_mm();
            let mut current = first_edge.end_mm();
            let mut entity_ids = vec![first_id];
            let mut edges = vec![first_edge];
            while distance2(current, first_start) > EPSILON_MM {
                let candidates = boundaries
                    .iter()
                    .filter_map(|(id, edge)| {
                        if distance2(edge.start_mm(), current) <= EPSILON_MM {
                            Some((*id, false))
                        } else if distance2(edge.end_mm(), current) <= EPSILON_MM {
                            Some((*id, true))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let [(next_id, reversed)] = candidates.as_slice() else {
                    return Err(if candidates.is_empty() {
                        SketchError::OpenRegion
                    } else {
                        SketchError::InvalidRegionIdentity
                    });
                };
                let edge = boundaries
                    .remove(next_id)
                    .ok_or(SketchError::InvalidRegionIdentity)?;
                let edge = if *reversed { edge.reversed() } else { edge };
                current = edge.end_mm();
                entity_ids.push(*next_id);
                edges.push(edge);
                if entity_ids.len() > MAX_SKETCH_ENTITIES {
                    return Err(SketchError::ResourceLimit);
                }
            }
            let area = region_signed_area(&edges)?.abs();
            if edges.len() < 2 || area <= EPSILON_MM * EPSILON_MM {
                return Err(SketchError::OpenRegion);
            }
            let ordered_entity_ids = entity_ids.clone();
            entity_ids.sort_unstable();
            let profile = if edges
                .iter()
                .all(|edge| matches!(edge, SolvedSketchRegionEdge::Line { .. }))
            {
                SolvedSketchRegionProfile::Polyline(
                    edges.iter().map(SolvedSketchRegionEdge::start_mm).collect(),
                )
            } else {
                SolvedSketchRegionProfile::Boundary(edges)
            };
            validate_profile_topology(&profile)?;
            loops.push(SolvedSketchLoop {
                entity_ids,
                ordered_entity_ids,
                profile,
                area,
            });
        }
        if loops.is_empty() {
            return Err(SketchError::InvalidRegionIdentity);
        }
        for left in 0..loops.len() {
            for right in left + 1..loops.len() {
                if profiles_intersect(&loops[left].profile, &loops[right].profile)? {
                    return Err(SketchError::InvalidRegionIdentity);
                }
            }
        }
        let mut parents = vec![None; loops.len()];
        for child in 0..loops.len() {
            let point = profile_point(&loops[child].profile);
            parents[child] = (0..loops.len())
                .filter(|parent| {
                    *parent != child
                        && loops[*parent].area > loops[child].area
                        && point_in_profile(point, &loops[*parent].profile).unwrap_or(true)
                })
                .min_by(|left, right| loops[*left].area.total_cmp(&loops[*right].area));
        }
        let mut depths = vec![0_usize; loops.len()];
        for index in 0..loops.len() {
            let mut cursor = parents[index];
            while let Some(parent) = cursor {
                depths[index] += 1;
                if depths[index] > loops.len() {
                    return Err(SketchError::InvalidRegionIdentity);
                }
                cursor = parents[parent];
            }
        }
        let mut regions = Vec::new();
        for outer_index in (0..loops.len()).filter(|index| depths[*index] % 2 == 0) {
            let outer = &loops[outer_index];
            let mut hole_indices = (0..loops.len())
                .filter(|index| parents[*index] == Some(outer_index) && depths[*index] % 2 == 1)
                .collect::<Vec<_>>();
            hole_indices.sort_by_key(|index| stable_region_id(&loops[*index].entity_ids));
            let mut entity_ids = outer.entity_ids.clone();
            for index in &hole_indices {
                entity_ids.extend_from_slice(&loops[*index].entity_ids);
            }
            entity_ids.sort_unstable();
            regions.push(SolvedSketchRegion {
                id: stable_region_id(&outer.entity_ids),
                entity_ids,
                outer_entity_ids: outer.ordered_entity_ids.clone(),
                outer: outer.profile.clone(),
                holes: hole_indices
                    .into_iter()
                    .map(|index| loops[index].profile.clone())
                    .collect(),
            });
        }
        regions.sort_by_key(|region| region.id);
        if regions.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(SketchError::InvalidRegionIdentity);
        }
        Ok(regions)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SketchError {
    InvalidWorkplaneFrame,
    MissingWorkplaneSupport(FeatureId),
    WorkplaneCycle(FeatureId),
    InvalidPlanarFaceSupport,
    UnresolvedWorkplaneSupport(WorkplaneSupportHealth),
    ReservedEntityId,
    ReservedConstraintId,
    EntitiesNotCanonical,
    ConstraintsNotCanonical,
    InvalidEntity(SketchEntityId),
    EntityNotFound(SketchEntityId),
    EntityIdCollision(SketchEntityId),
    ConstraintIdCollision(SketchConstraintId),
    ManagedConstraintReadOnly(SketchConstraintId),
    InvalidProjectionSource,
    ProjectedEntityReadOnly(SketchEntityId),
    InvalidConstructionState(SketchEntityId),
    InvalidSplitParameter,
    InvalidSplitConstraintIds,
    AmbiguousSplitConstraint,
    UnsupportedJoinEntity(SketchEntityId),
    IncompatibleJoinGeometry,
    AmbiguousJoinConstraint,
    InvalidTrimInterval,
    AmbiguousTrimConstraint,
    InvalidExtendParameter,
    UnsupportedExtendEntity(SketchEntityId),
    AmbiguousExtendConstraint,
    InvalidOffsetDistance,
    AmbiguousOffset,
    InvalidConstraintReference(SketchConstraintId),
    ConstraintEditInvalidatesProfile(SketchConstraintId),
    InvalidDimension,
    InvalidSolverPolicy,
    NonConvergent,
    InvalidFeatureDirection,
    InvalidFeatureExtentReference,
    OverConstrained(SketchConstraintId),
    SketchNotFullyConstrained,
    OpenRegion,
    UnsupportedRegionGeometry,
    InvalidRegionIdentity,
    ResourceLimit,
}

impl fmt::Display for SketchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWorkplaneFrame => {
                formatter.write_str("workplane frame is not finite, orthonormal, and right-handed")
            }
            Self::MissingWorkplaneSupport(id) => {
                write!(formatter, "workplane support feature {} is missing", id.0)
            }
            Self::WorkplaneCycle(id) => write!(
                formatter,
                "workplane support cycle reaches feature {}",
                id.0
            ),
            Self::InvalidPlanarFaceSupport => formatter
                .write_str("planar-face support does not carry one valid stable face lineage"),
            Self::UnresolvedWorkplaneSupport(health) => {
                write!(formatter, "workplane support is not resolved: {health:?}")
            }
            Self::ReservedEntityId => formatter.write_str("sketch entity ID zero is reserved"),
            Self::ReservedConstraintId => {
                formatter.write_str("sketch constraint ID zero is reserved")
            }
            Self::EntitiesNotCanonical => {
                formatter.write_str("sketch entities must be unique and strictly sorted by ID")
            }
            Self::ConstraintsNotCanonical => {
                formatter.write_str("sketch constraints must be unique and strictly sorted by ID")
            }
            Self::InvalidEntity(id) => {
                write!(formatter, "sketch entity {} is invalid or degenerate", id.0)
            }
            Self::EntityNotFound(id) => {
                write!(formatter, "sketch entity {} does not exist", id.0)
            }
            Self::EntityIdCollision(id) => {
                write!(formatter, "sketch entity ID {} already exists", id.0)
            }
            Self::ConstraintIdCollision(id) => {
                write!(formatter, "sketch constraint ID {} already exists", id.0)
            }
            Self::ManagedConstraintReadOnly(id) => write!(
                formatter,
                "managed sketch constraint {} is read-only through direct editing",
                id.0
            ),
            Self::InvalidProjectionSource => formatter.write_str(
                "sketch projection source and canonical projected geometry must be valid",
            ),
            Self::ProjectedEntityReadOnly(id) => {
                write!(formatter, "projected sketch entity {} is read-only", id.0)
            }
            Self::InvalidConstructionState(id) => write!(
                formatter,
                "sketch entity {} is already in the requested construction state",
                id.0
            ),
            Self::InvalidSplitParameter => formatter
                .write_str("sketch split parameter must be finite and strictly inside the curve"),
            Self::InvalidSplitConstraintIds => formatter.write_str(
                "sketch split requires unique non-zero joint constraint IDs of the expected count",
            ),
            Self::AmbiguousSplitConstraint => formatter.write_str(
                "sketch split cannot preserve a constraint that refers to the whole source curve",
            ),
            Self::UnsupportedJoinEntity(id) => {
                write!(formatter, "closed sketch entity {} cannot be joined", id.0)
            }
            Self::IncompatibleJoinGeometry => formatter.write_str(
                "sketch join requires compatible open curves with one shared selected endpoint",
            ),
            Self::AmbiguousJoinConstraint => formatter.write_str(
                "sketch join cannot preserve a constraint on removed or whole-curve geometry",
            ),
            Self::InvalidTrimInterval => formatter.write_str(
                "sketch trim keep interval must be finite, non-degenerate, bounded, and not the full curve",
            ),
            Self::AmbiguousTrimConstraint => formatter.write_str(
                "sketch trim cannot preserve a constraint on removed geometry or the whole source curve",
            ),
            Self::InvalidExtendParameter => formatter.write_str(
                "sketch extend requires Start below zero or End above one with a finite parameter",
            ),
            Self::UnsupportedExtendEntity(id) => {
                write!(formatter, "closed sketch entity {} cannot be extended", id.0)
            }
            Self::AmbiguousExtendConstraint => formatter.write_str(
                "sketch extend cannot preserve a constraint on changed geometry or the whole source curve",
            ),
            Self::InvalidOffsetDistance => formatter.write_str(
                "sketch offset distance must be finite, positive, and within the model envelope",
            ),
            Self::AmbiguousOffset => formatter.write_str(
                "sketch offset is degenerate because its radius collapses or its direction is undefined",
            ),
            Self::InvalidConstraintReference(id) => write!(
                formatter,
                "sketch constraint {} has an invalid entity or point reference",
                id.0
            ),
            Self::ConstraintEditInvalidatesProfile(id) => write!(
                formatter,
                "editing sketch constraint {} would invalidate a downstream Pad/Pocket region",
                id.0
            ),
            Self::InvalidDimension => formatter.write_str("sketch dimension is invalid"),
            Self::InvalidSolverPolicy => formatter.write_str("sketch solver policy is invalid"),
            Self::NonConvergent => formatter
                .write_str("sketch solver did not converge within the bounded numerical policy"),
            Self::InvalidFeatureDirection => {
                formatter.write_str("feature direction must be finite and non-zero")
            }
            Self::InvalidFeatureExtentReference => formatter
                .write_str("up-to-face extent requires one valid stable planar-face reference"),
            Self::OverConstrained(id) => write!(
                formatter,
                "sketch constraint {} is conflicting, redundant, or exceeds available degrees of freedom",
                id.0
            ),
            Self::SketchNotFullyConstrained => {
                formatter.write_str("sketch regions require fully constrained solved geometry")
            }
            Self::OpenRegion => formatter.write_str("sketch entities do not form closed regions"),
            Self::UnsupportedRegionGeometry => {
                formatter.write_str("sketch region geometry is not supported by exact Pad/Pocket")
            }
            Self::InvalidRegionIdentity => {
                formatter.write_str("sketch region identity is empty or ambiguous")
            }
            Self::ResourceLimit => {
                formatter.write_str("sketch exceeds bounded entity or constraint limits")
            }
        }
    }
}

impl std::error::Error for SketchError {}
