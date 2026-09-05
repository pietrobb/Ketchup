use crate::document::{Dimension, FeatureId};
use crate::exact_product::BodySubshapeRef;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_SKETCH_ENTITIES: usize = 4_096;
pub const MAX_SKETCH_CONSTRAINTS: usize = 8_192;
const MAX_ABS_MM: f64 = 1_000_000.0;
const MAX_SKETCH_SOLVER_DOF: usize = 512;
const EPSILON_MM: f64 = 1.0e-7;
const FRAME_EPSILON: f64 = 1.0e-9;
const MAX_SKETCH_SOLVER_ITERATIONS: u16 = 256;
const MAX_SKETCH_NUMERICAL_EVALUATIONS: usize = 67_108_864;
const MAX_CUBIC_FLATTEN_DEPTH: u8 = 16;
const MAX_CUBIC_FLATTEN_SEGMENTS: usize = 16_384;
const CUBIC_FLATTEN_TOLERANCE_MM: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SketchEntityId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SketchConstraintId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SketchRegionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalPlane {
    Xy,
    Yz,
    Xz,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkplaneSupportHealth {
    Resolved,
    Ambiguous,
    Lost,
    Stale,
}

#[derive(Clone, Debug, PartialEq)]
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
        health: WorkplaneSupportHealth,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
            .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
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

#[derive(Clone, Debug, PartialEq)]
pub struct WorkplaneSpec {
    pub support: WorkplaneSupport,
    pub frame: WorkplaneFrame,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SketchPointKind {
    Start,
    End,
    Center,
    Control1,
    Control2,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SketchPointRef {
    pub entity: SketchEntityId,
    pub point: SketchPointKind,
}

#[derive(Clone, Debug, PartialEq)]
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
                .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
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
                    || *radius_mm > MAX_ABS_MM
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

#[derive(Clone, Debug, PartialEq)]
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
}

#[derive(Clone, Debug, PartialEq)]
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
            finite_difference_step: 1.0e-6,
            initial_damping: 1.0e-6,
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
    pub outer: SolvedSketchRegionProfile,
    pub holes: Vec<SolvedSketchRegionProfile>,
}

struct SolvedSketchLoop {
    entity_ids: Vec<SketchEntityId>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
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
                || length > MAX_ABS_MM
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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
            Self::Blind(distance) | Self::Symmetric(distance) => validate_extent_distance(distance),
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
    Dimension::new(distance.source_token(), distance.millimetres())
        .map_err(|_| SketchError::InvalidDimension)?;
    if distance.millimetres() <= EPSILON_MM || distance.millimetres() > MAX_ABS_MM {
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

#[derive(Clone, Debug, PartialEq)]
pub struct PadSpec {
    pub sketch: FeatureId,
    pub region: SketchRegionId,
    pub direction: FeatureDirection,
    pub extent: FeatureExtent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PocketSpec {
    pub target: FeatureId,
    pub sketch: FeatureId,
    pub region: SketchRegionId,
    pub support: Box<BodySubshapeRef>,
    pub direction: FeatureDirection,
    pub extent: FeatureExtent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PadPocketOperation {
    Pad(PadSpec),
    Pocket(PocketSpec),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchSpec {
    pub workplane: FeatureId,
    pub entities: Vec<SketchEntity>,
    pub constraints: Vec<SketchConstraint>,
}

impl SketchSpec {
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
        let constraint_rank = structural_constraint_rank(&rank_equations, variable_count, None);
        for (constraint_id, range) in &constraint_equation_ranges {
            let rank_without =
                structural_constraint_rank(&rank_equations, variable_count, Some(range.clone()));
            if constraint_rank.saturating_sub(rank_without) < range.len() {
                return Err(SketchError::OverConstrained(*constraint_id));
            }
        }
        let status = if constraint_rank == degrees_of_freedom {
            SketchSolveStatus::FullyConstrained
        } else {
            SketchSolveStatus::UnderConstrained {
                remaining_dof: degrees_of_freedom - constraint_rank,
            }
        };
        let mut solved_entities = self.entities.clone();
        solve_constraints(&mut solved_entities, &self.constraints, policy)?;
        for entity in &solved_entities {
            entity.validate()?;
        }
        Ok(SketchSolution {
            report: SketchSolveReport {
                status,
                entity_count: self.entities.len(),
                constraint_count: self.constraints.len(),
                equation_count,
            },
            entities: solved_entities,
        })
    }

    pub fn solved_regions(&self) -> Result<Vec<SolvedSketchRegion>, SketchError> {
        let solution = self.solve_geometry()?;
        let mut loops = Vec::new();
        let mut boundaries = BTreeMap::new();
        for entity in solution.entities {
            match entity {
                SketchEntity::Circle {
                    id,
                    center_mm,
                    radius_mm,
                } => loops.push(SolvedSketchLoop {
                    entity_ids: vec![id],
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

fn region_signed_area(edges: &[SolvedSketchRegionEdge]) -> Result<f64, SketchError> {
    edges
        .iter()
        .map(|edge| match edge {
            SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                Ok(0.5 * (start_mm[0] * end_mm[1] - end_mm[0] * start_mm[1]))
            }
            SolvedSketchRegionEdge::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let radius = distance2(*start_mm, *center_mm);
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let mut sweep = end_angle - start_angle;
                if *clockwise {
                    if sweep >= 0.0 {
                        sweep -= std::f64::consts::TAU;
                    }
                } else if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                Ok(0.5
                    * (radius * center_mm[0] * (end_angle.sin() - start_angle.sin())
                        - radius * center_mm[1] * (end_angle.cos() - start_angle.cos())
                        + radius * radius * sweep))
            }
            SolvedSketchRegionEdge::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => Ok(
                flatten_cubic([*start_mm, *control_1_mm, *control_2_mm, *end_mm])?
                    .windows(2)
                    .map(|pair| 0.5 * (pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1]))
                    .sum(),
            ),
        })
        .sum()
}

fn profile_curves(profile: &SolvedSketchRegionProfile) -> Result<Vec<RegionCurve>, SketchError> {
    let mut curves = Vec::new();
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => curves.extend(
            points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .take(points.len())
                .map(|(start, end)| RegionCurve::Line {
                    start: *start,
                    end: *end,
                }),
        ),
        SolvedSketchRegionProfile::Boundary(edges) => {
            for edge in edges {
                match edge {
                    SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                        curves.push(RegionCurve::Line {
                            start: *start_mm,
                            end: *end_mm,
                        })
                    }
                    SolvedSketchRegionEdge::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => curves.push(RegionCurve::Arc {
                        start: *start_mm,
                        end: *end_mm,
                        center: *center_mm,
                        clockwise: *clockwise,
                    }),
                    SolvedSketchRegionEdge::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                    } => curves.extend(
                        flatten_cubic([*start_mm, *control_1_mm, *control_2_mm, *end_mm])?
                            .windows(2)
                            .map(|pair| RegionCurve::Line {
                                start: pair[0],
                                end: pair[1],
                            }),
                    ),
                }
            }
        }
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => {
            let right = [center_mm[0] + radius_mm, center_mm[1]];
            let left = [center_mm[0] - radius_mm, center_mm[1]];
            curves.extend([
                RegionCurve::Arc {
                    start: right,
                    end: left,
                    center: *center_mm,
                    clockwise: false,
                },
                RegionCurve::Arc {
                    start: left,
                    end: right,
                    center: *center_mm,
                    clockwise: false,
                },
            ]);
        }
    }
    Ok(curves)
}

fn profile_point(profile: &SolvedSketchRegionProfile) -> [f64; 2] {
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => points[0],
        SolvedSketchRegionProfile::Boundary(edges) => edges[0].start_mm(),
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => [center_mm[0] + radius_mm, center_mm[1]],
    }
}

fn profiles_intersect(
    left: &SolvedSketchRegionProfile,
    right: &SolvedSketchRegionProfile,
) -> Result<bool, SketchError> {
    let left = profile_curves(left)?;
    let right = profile_curves(right)?;
    Ok(left
        .iter()
        .any(|left| right.iter().any(|right| curves_intersect(*left, *right))))
}

fn curves_intersect(left: RegionCurve, right: RegionCurve) -> bool {
    match (left, right) {
        (
            RegionCurve::Line {
                start: left_start,
                end: left_end,
            },
            RegionCurve::Line {
                start: right_start,
                end: right_end,
            },
        ) => line_segments_intersect(left_start, left_end, right_start, right_end),
        (RegionCurve::Line { start, end }, arc @ RegionCurve::Arc { .. })
        | (arc @ RegionCurve::Arc { .. }, RegionCurve::Line { start, end }) => {
            line_arc_intersects(start, end, arc)
        }
        (left @ RegionCurve::Arc { .. }, right @ RegionCurve::Arc { .. }) => {
            arcs_intersect(left, right)
        }
    }
}

fn cross2(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn subtract2(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn line_segments_intersect(
    left_start: [f64; 2],
    left_end: [f64; 2],
    right_start: [f64; 2],
    right_end: [f64; 2],
) -> bool {
    let left = subtract2(left_end, left_start);
    let right = subtract2(right_end, right_start);
    let offset = subtract2(right_start, left_start);
    let denominator = cross2(left, right);
    let near = 3.0 * CUBIC_FLATTEN_TOLERANCE_MM;
    if denominator.abs() <= EPSILON_MM {
        if cross2(offset, left).abs() > EPSILON_MM {
            return point_segment_distance(left_start, right_start, right_end) <= near
                || point_segment_distance(left_end, right_start, right_end) <= near
                || point_segment_distance(right_start, left_start, left_end) <= near
                || point_segment_distance(right_end, left_start, left_end) <= near;
        }
        let axis = usize::from(left[1].abs() > left[0].abs());
        let (left_min, left_max) = if left_start[axis] <= left_end[axis] {
            (left_start[axis], left_end[axis])
        } else {
            (left_end[axis], left_start[axis])
        };
        let (right_min, right_max) = if right_start[axis] <= right_end[axis] {
            (right_start[axis], right_end[axis])
        } else {
            (right_end[axis], right_start[axis])
        };
        return left_min <= right_max + EPSILON_MM && right_min <= left_max + EPSILON_MM;
    }
    let along_left = cross2(offset, right) / denominator;
    let along_right = cross2(offset, left) / denominator;
    if (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&along_left)
        && (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&along_right)
    {
        return true;
    }
    point_segment_distance(left_start, right_start, right_end) <= near
        || point_segment_distance(left_end, right_start, right_end) <= near
        || point_segment_distance(right_start, left_start, left_end) <= near
        || point_segment_distance(right_end, left_start, left_end) <= near
}

fn point_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = subtract2(end, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared <= EPSILON_MM * EPSILON_MM {
        return distance2(point, start);
    }
    let parameter = ((point[0] - start[0]) * direction[0] + (point[1] - start[1]) * direction[1])
        / length_squared;
    let parameter = parameter.clamp(0.0, 1.0);
    distance2(
        point,
        [
            start[0] + parameter * direction[0],
            start[1] + parameter * direction[1],
        ],
    )
}

fn arc_parts(curve: RegionCurve) -> ([f64; 2], [f64; 2], [f64; 2], bool) {
    let RegionCurve::Arc {
        start,
        end,
        center,
        clockwise,
    } = curve
    else {
        unreachable!()
    };
    (start, end, center, clockwise)
}

fn point_on_arc(point: [f64; 2], curve: RegionCurve) -> bool {
    let (start, end, center, clockwise) = arc_parts(curve);
    let radius = distance2(start, center);
    if (distance2(point, center) - radius).abs() > radius.max(1.0) * 1.0e-8 {
        return false;
    }
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    let point_angle = (point[1] - center[1]).atan2(point[0] - center[0]);
    let total = if clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    };
    let offset = if clockwise {
        (start_angle - point_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (point_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    };
    offset <= total + 1.0e-10
}

fn line_arc_intersects(start: [f64; 2], end: [f64; 2], arc: RegionCurve) -> bool {
    let (_, _, center, _) = arc_parts(arc);
    let direction = subtract2(end, start);
    let offset = subtract2(start, center);
    let radius = distance2(arc_parts(arc).0, center);
    let a = direction[0] * direction[0] + direction[1] * direction[1];
    let b = 2.0 * (offset[0] * direction[0] + offset[1] * direction[1]);
    let c = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < -EPSILON_MM {
        return false;
    }
    let root = discriminant.max(0.0).sqrt();
    [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
        .into_iter()
        .any(|parameter| {
            (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&parameter)
                && point_on_arc(
                    [
                        start[0] + direction[0] * parameter,
                        start[1] + direction[1] * parameter,
                    ],
                    arc,
                )
        })
}

fn arcs_intersect(left: RegionCurve, right: RegionCurve) -> bool {
    let (left_start, _, left_center, _) = arc_parts(left);
    let (right_start, _, right_center, _) = arc_parts(right);
    let left_radius = distance2(left_start, left_center);
    let right_radius = distance2(right_start, right_center);
    let center_distance = distance2(left_center, right_center);
    let tolerance = left_radius.max(right_radius).max(1.0) * 1.0e-8;
    if center_distance <= tolerance && (left_radius - right_radius).abs() <= tolerance {
        return true;
    }
    if center_distance > left_radius + right_radius + tolerance
        || center_distance < (left_radius - right_radius).abs() - tolerance
        || center_distance <= tolerance
    {
        return false;
    }
    let along = (left_radius * left_radius - right_radius * right_radius
        + center_distance * center_distance)
        / (2.0 * center_distance);
    let height_squared = left_radius * left_radius - along * along;
    if height_squared < -tolerance {
        return false;
    }
    let direction = [
        (right_center[0] - left_center[0]) / center_distance,
        (right_center[1] - left_center[1]) / center_distance,
    ];
    let base = [
        left_center[0] + along * direction[0],
        left_center[1] + along * direction[1],
    ];
    let height = height_squared.max(0.0).sqrt();
    let perpendicular = [-direction[1], direction[0]];
    [
        [
            base[0] + height * perpendicular[0],
            base[1] + height * perpendicular[1],
        ],
        [
            base[0] - height * perpendicular[0],
            base[1] - height * perpendicular[1],
        ],
    ]
    .into_iter()
    .any(|point| point_on_arc(point, left) && point_on_arc(point, right))
}

fn point_in_profile(
    point: [f64; 2],
    profile: &SolvedSketchRegionProfile,
) -> Result<bool, SketchError> {
    let mut winding = 0_i32;
    for curve in profile_curves(profile)? {
        match curve {
            RegionCurve::Line { start, end } => {
                if start[1] <= point[1]
                    && point[1] < end[1]
                    && cross2(subtract2(end, start), subtract2(point, start)) > 0.0
                {
                    winding += 1;
                } else if end[1] <= point[1]
                    && point[1] < start[1]
                    && cross2(subtract2(end, start), subtract2(point, start)) < 0.0
                {
                    winding -= 1;
                }
            }
            arc @ RegionCurve::Arc {
                center, clockwise, ..
            } => {
                let radius = distance2(arc_parts(arc).0, center);
                let relative_y = (point[1] - center[1]) / radius;
                if relative_y.abs() > 1.0 {
                    continue;
                }
                let angle = relative_y.clamp(-1.0, 1.0).asin();
                for candidate in [angle, std::f64::consts::PI - angle] {
                    let crossing = [
                        center[0] + radius * candidate.cos(),
                        center[1] + radius * candidate.sin(),
                    ];
                    if crossing[0] <= point[0] + EPSILON_MM || !point_on_arc(crossing, arc) {
                        continue;
                    }
                    let derivative = candidate.cos() * if clockwise { -1.0 } else { 1.0 };
                    if derivative > EPSILON_MM {
                        winding += 1;
                    } else if derivative < -EPSILON_MM {
                        winding -= 1;
                    }
                }
            }
        }
    }
    Ok(winding != 0)
}

fn cubic_control_polygon_length(points: [[f64; 2]; 4]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance2(pair[0], pair[1]))
        .sum()
}

fn midpoint2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn point_line_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let chord = subtract2(end, start);
    let length = chord[0].hypot(chord[1]);
    if length <= EPSILON_MM {
        distance2(point, start)
    } else {
        cross2(chord, subtract2(point, start)).abs() / length
    }
}

fn flatten_cubic(points: [[f64; 2]; 4]) -> Result<Vec<[f64; 2]>, SketchError> {
    let tolerance = CUBIC_FLATTEN_TOLERANCE_MM;
    let mut output = vec![points[0]];
    let mut stack = vec![(points, 0_u8)];
    while let Some((curve, depth)) = stack.pop() {
        let flatness = point_line_distance(curve[1], curve[0], curve[3])
            .max(point_line_distance(curve[2], curve[0], curve[3]));
        let excess_length = cubic_control_polygon_length(curve) - distance2(curve[0], curve[3]);
        if flatness <= tolerance && excess_length <= tolerance {
            output.push(curve[3]);
            if output.len() > MAX_CUBIC_FLATTEN_SEGMENTS + 1 {
                return Err(SketchError::ResourceLimit);
            }
            continue;
        }
        if depth >= MAX_CUBIC_FLATTEN_DEPTH
            || stack.len() + output.len() >= MAX_CUBIC_FLATTEN_SEGMENTS
        {
            return Err(SketchError::ResourceLimit);
        }
        let p01 = midpoint2(curve[0], curve[1]);
        let p12 = midpoint2(curve[1], curve[2]);
        let p23 = midpoint2(curve[2], curve[3]);
        let p012 = midpoint2(p01, p12);
        let p123 = midpoint2(p12, p23);
        let middle = midpoint2(p012, p123);
        stack.push(([middle, p123, p23, curve[3]], depth + 1));
        stack.push(([curve[0], p01, p012, middle], depth + 1));
    }
    if output
        .windows(2)
        .any(|pair| distance2(pair[0], pair[1]) <= EPSILON_MM)
    {
        return Err(SketchError::InvalidRegionIdentity);
    }
    Ok(output)
}

fn adjacent_curves_overlap(previous: RegionCurve, next: RegionCurve) -> bool {
    let (
        RegionCurve::Line {
            start: previous_start,
            end: joint,
        },
        RegionCurve::Line {
            start: next_start,
            end: next_end,
        },
    ) = (previous, next)
    else {
        return false;
    };
    if distance2(joint, next_start) > EPSILON_MM {
        return true;
    }
    let incoming = subtract2(previous_start, joint);
    let outgoing = subtract2(next_end, joint);
    let scale = distance2(previous_start, joint)
        .max(distance2(next_end, joint))
        .max(1.0);
    cross2(incoming, outgoing).abs() <= EPSILON_MM * scale
        && incoming[0] * outgoing[0] + incoming[1] * outgoing[1] > 0.0
}

fn validate_profile_topology(profile: &SolvedSketchRegionProfile) -> Result<(), SketchError> {
    let curves = profile_curves(profile)?;
    if curves.len() < 2 {
        return Err(SketchError::OpenRegion);
    }
    for left in 0..curves.len() {
        for right in left + 1..curves.len() {
            let adjacent = right == left + 1 || (left == 0 && right + 1 == curves.len());
            if adjacent {
                let (previous, next) = if right == left + 1 {
                    (curves[left], curves[right])
                } else {
                    (curves[right], curves[left])
                };
                if adjacent_curves_overlap(previous, next) {
                    return Err(SketchError::InvalidRegionIdentity);
                }
            } else if curves_intersect(curves[left], curves[right]) {
                return Err(SketchError::InvalidRegionIdentity);
            }
        }
    }
    Ok(())
}

fn stable_region_id(entity_ids: &[SketchEntityId]) -> SketchRegionId {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for id in entity_ids {
        for byte in id.0.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    SketchRegionId(hash.max(1))
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
    InvalidConstraintReference(SketchConstraintId),
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
            Self::InvalidConstraintReference(id) => write!(
                formatter,
                "sketch constraint {} has an invalid entity or point reference",
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

fn evaluate_constraint(
    constraint: &SketchConstraint,
    entities: &BTreeMap<SketchEntityId, &SketchEntity>,
) -> Result<(Vec<u8>, usize), SketchError> {
    let point = |reference: SketchPointRef| {
        entities
            .get(&reference.entity)
            .and_then(|entity| entity.point(reference.point))
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))
    };
    let mut signature = vec![0];
    let equations = match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } | SketchConstraintKind::Vertical { entity } => {
            let tag = if matches!(constraint.kind, SketchConstraintKind::Horizontal { .. }) {
                1
            } else {
                2
            };
            signature[0] = tag;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            if !matches!(
                entities.get(entity).copied(),
                Some(SketchEntity::Line { .. })
            ) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            1
        }
        SketchConstraintKind::Coincident { a, b } => {
            point(*a)?;
            point(*b)?;
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let (first, second) = canonical_point_pair(*a, *b);
            signature[0] = 3;
            push_point_ref(&mut signature, first);
            push_point_ref(&mut signature, second);
            2
        }
        SketchConstraintKind::Distance { a, b, value } => {
            point(*a)?;
            point(*b)?;
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let expected = valid_positive_dimension(value)?;
            if let Some(entity) = arc_radius_entity(*a, *b).filter(|entity| {
                matches!(
                    entities.get(entity).copied(),
                    Some(SketchEntity::Arc { .. })
                )
            }) {
                signature[0] = 5;
                signature.extend_from_slice(&entity.0.to_le_bytes());
            } else {
                let (first, second) = canonical_point_pair(*a, *b);
                signature[0] = 4;
                push_point_ref(&mut signature, first);
                push_point_ref(&mut signature, second);
            }
            signature.extend_from_slice(&expected.to_bits().to_le_bytes());
            1
        }
        SketchConstraintKind::Radius { entity, value } => {
            let expected = valid_positive_dimension(value)?;
            signature[0] = 5;
            signature.extend_from_slice(&entity.0.to_le_bytes());
            signature.extend_from_slice(&expected.to_bits().to_le_bytes());
            if !matches!(
                entities.get(entity).copied(),
                Some(SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
            ) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            1
        }
        SketchConstraintKind::FixedPoint {
            point: reference,
            position_mm,
        } => {
            point(*reference)?;
            if position_mm
                .iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 6;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&position_mm[0].to_bits().to_le_bytes());
            signature.extend_from_slice(&position_mm[1].to_bits().to_le_bytes());
            2
        }
        SketchConstraintKind::Parallel { a, b }
        | SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Collinear { a, b } => {
            if a == b
                || !matches!(entities.get(a).copied(), Some(SketchEntity::Line { .. }))
                || !matches!(entities.get(b).copied(), Some(SketchEntity::Line { .. }))
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = match constraint.kind {
                SketchConstraintKind::Parallel { .. } => 7,
                SketchConstraintKind::Perpendicular { .. } => 8,
                SketchConstraintKind::Collinear { .. } => 14,
                _ => unreachable!(),
            };
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            if matches!(constraint.kind, SketchConstraintKind::Collinear { .. }) {
                2
            } else {
                1
            }
        }
        SketchConstraintKind::Tangent { a, b } => {
            if a == b || !supports_tangent(entities.get(a).copied(), entities.get(b).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 9;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            1
        }
        SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees,
        } => {
            if a == b
                || !matches!(entities.get(a).copied(), Some(SketchEntity::Line { .. }))
                || !matches!(entities.get(b).copied(), Some(SketchEntity::Line { .. }))
                || !valid_angle_degrees(*angle_degrees)
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 10;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            signature.extend_from_slice(&angle_degrees.to_bits().to_le_bytes());
            1
        }
        SketchConstraintKind::Equal { a, b } => {
            if a == b || !supports_equal(entities.get(a).copied(), entities.get(b).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 11;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            1
        }
        SketchConstraintKind::Symmetric { a, b, axis } => {
            point(*a)?;
            point(*b)?;
            if a == b || !matches!(entities.get(axis).copied(), Some(SketchEntity::Line { .. })) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 12;
            let (first, second) = canonical_point_pair(*a, *b);
            push_point_ref(&mut signature, first);
            push_point_ref(&mut signature, second);
            signature.extend_from_slice(&axis.0.to_le_bytes());
            2
        }
        SketchConstraintKind::Concentric { a, b } => {
            if a == b
                || !is_circular_entity(entities.get(a).copied())
                || !is_circular_entity(entities.get(b).copied())
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 13;
            let (first, second) = canonical_entity_pair(*a, *b);
            signature.extend_from_slice(&first.0.to_le_bytes());
            signature.extend_from_slice(&second.0.to_le_bytes());
            2
        }
        SketchConstraintKind::Midpoint {
            point: reference,
            line,
        } => {
            point(*reference)?;
            if reference.entity == *line
                || !matches!(entities.get(line).copied(), Some(SketchEntity::Line { .. }))
            {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 15;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&line.0.to_le_bytes());
            2
        }
        SketchConstraintKind::PointOnCurve {
            point: reference,
            curve,
        } => {
            point(*reference)?;
            if reference.entity == *curve || !is_curve_entity(entities.get(curve).copied()) {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            }
            signature[0] = 16;
            push_point_ref(&mut signature, *reference);
            signature.extend_from_slice(&curve.0.to_le_bytes());
            1
        }
    };
    Ok((signature, equations))
}

fn dimensional_constraint_target(
    constraint: &SketchConstraint,
    coincidence_parents: &BTreeMap<SketchPointRef, SketchPointRef>,
    entities: &BTreeMap<SketchEntityId, &SketchEntity>,
) -> Result<(Vec<u8>, Vec<u8>), SketchError> {
    let mut target = Vec::new();
    let mut value = Vec::new();
    match &constraint.kind {
        SketchConstraintKind::Distance {
            a,
            b,
            value: dimension,
        } => {
            let a = coincidence_root(coincidence_parents, *a);
            let b = coincidence_root(coincidence_parents, *b);
            if a == b {
                return Err(SketchError::OverConstrained(constraint.id));
            }
            let radial_entity = entities.iter().find_map(|(entity_id, entity)| {
                if !matches!(entity, SketchEntity::Arc { .. }) {
                    return None;
                }
                [SketchPointKind::Start, SketchPointKind::End]
                    .into_iter()
                    .any(|endpoint| {
                        let center = coincidence_root(
                            coincidence_parents,
                            SketchPointRef {
                                entity: *entity_id,
                                point: SketchPointKind::Center,
                            },
                        );
                        let endpoint = coincidence_root(
                            coincidence_parents,
                            SketchPointRef {
                                entity: *entity_id,
                                point: endpoint,
                            },
                        );
                        canonical_point_pair(center, endpoint) == canonical_point_pair(a, b)
                    })
                    .then_some(*entity_id)
            });
            if let Some(entity) = radial_entity {
                target.push(1);
                target.extend_from_slice(&entity.0.to_le_bytes());
            } else {
                target.push(2);
                let (first, second) = canonical_point_pair(a, b);
                push_point_ref(&mut target, first);
                push_point_ref(&mut target, second);
            }
            value.extend_from_slice(&dimension.millimetres().to_bits().to_le_bytes());
        }
        SketchConstraintKind::Radius {
            entity,
            value: dimension,
        } => {
            target.push(1);
            target.extend_from_slice(&entity.0.to_le_bytes());
            value.extend_from_slice(&dimension.millimetres().to_bits().to_le_bytes());
        }
        SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees,
        } => {
            target.push(4);
            let (first, second) = canonical_entity_pair(*a, *b);
            target.extend_from_slice(&first.0.to_le_bytes());
            target.extend_from_slice(&second.0.to_le_bytes());
            value.extend_from_slice(&angle_degrees.to_bits().to_le_bytes());
        }
        SketchConstraintKind::FixedPoint { point, position_mm } => {
            target.push(3);
            push_point_ref(&mut target, coincidence_root(coincidence_parents, *point));
            value.extend_from_slice(&position_mm[0].to_bits().to_le_bytes());
            value.extend_from_slice(&position_mm[1].to_bits().to_le_bytes());
        }
        SketchConstraintKind::Horizontal { .. }
        | SketchConstraintKind::Vertical { .. }
        | SketchConstraintKind::Coincident { .. }
        | SketchConstraintKind::Parallel { .. }
        | SketchConstraintKind::Perpendicular { .. }
        | SketchConstraintKind::Tangent { .. }
        | SketchConstraintKind::Equal { .. }
        | SketchConstraintKind::Symmetric { .. }
        | SketchConstraintKind::Concentric { .. }
        | SketchConstraintKind::Collinear { .. }
        | SketchConstraintKind::Midpoint { .. }
        | SketchConstraintKind::PointOnCurve { .. } => {
            unreachable!("only dimensional constraints are collected")
        }
    }
    Ok((target, value))
}

#[derive(Clone, Copy)]
enum VariableLayout {
    Line { base: usize },
    Arc { base: usize },
    Circle { base: usize },
    CubicBezier { base: usize },
}

fn variable_layouts(
    entities: &[SketchEntity],
) -> Result<(BTreeMap<SketchEntityId, VariableLayout>, usize), SketchError> {
    let mut layouts = BTreeMap::new();
    let mut next = 0usize;
    for entity in entities {
        let layout = match entity {
            SketchEntity::Line { .. } => VariableLayout::Line { base: next },
            SketchEntity::Arc { .. } => VariableLayout::Arc { base: next },
            SketchEntity::Circle { .. } => VariableLayout::Circle { base: next },
            SketchEntity::CubicBezier { .. } => VariableLayout::CubicBezier { base: next },
        };
        next = next
            .checked_add(entity.degrees_of_freedom())
            .ok_or(SketchError::ResourceLimit)?;
        if next > MAX_SKETCH_SOLVER_DOF {
            return Err(SketchError::ResourceLimit);
        }
        layouts.insert(entity.id(), layout);
    }
    Ok((layouts, next))
}

fn constraint_variable_equations(
    constraint: &SketchConstraint,
    layouts: &BTreeMap<SketchEntityId, VariableLayout>,
) -> Result<Vec<Vec<usize>>, SketchError> {
    let point_variables = |reference: SketchPointRef, axis: usize| {
        let layout = layouts
            .get(&reference.entity)
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))?;
        let variables = match (*layout, reference.point, axis) {
            (VariableLayout::Line { base }, SketchPointKind::Start, 0) => vec![base],
            (VariableLayout::Line { base }, SketchPointKind::Start, 1) => vec![base + 1],
            (VariableLayout::Line { base }, SketchPointKind::End, 0) => vec![base + 2],
            (VariableLayout::Line { base }, SketchPointKind::End, 1) => vec![base + 3],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Start, 0) => vec![base],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Start, 1) => vec![base + 1],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control1, 0) => vec![base + 2],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control1, 1) => vec![base + 3],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control2, 0) => vec![base + 4],
            (VariableLayout::CubicBezier { base }, SketchPointKind::Control2, 1) => vec![base + 5],
            (VariableLayout::CubicBezier { base }, SketchPointKind::End, 0) => vec![base + 6],
            (VariableLayout::CubicBezier { base }, SketchPointKind::End, 1) => vec![base + 7],
            (VariableLayout::Arc { base }, SketchPointKind::Center, 0)
            | (VariableLayout::Circle { base }, SketchPointKind::Center, 0) => vec![base],
            (VariableLayout::Arc { base }, SketchPointKind::Center, 1)
            | (VariableLayout::Circle { base }, SketchPointKind::Center, 1) => vec![base + 1],
            (VariableLayout::Arc { base }, SketchPointKind::Start, 0) => {
                vec![base, base + 2, base + 3]
            }
            (VariableLayout::Arc { base }, SketchPointKind::Start, 1) => {
                vec![base + 1, base + 2, base + 3]
            }
            (VariableLayout::Arc { base }, SketchPointKind::End, 0) => {
                vec![base, base + 2, base + 4]
            }
            (VariableLayout::Arc { base }, SketchPointKind::End, 1) => {
                vec![base + 1, base + 2, base + 4]
            }
            _ => return Err(SketchError::InvalidConstraintReference(constraint.id)),
        };
        Ok(variables)
    };
    let entity_variables = |entity: SketchEntityId| {
        let (base, width) = match layouts
            .get(&entity)
            .ok_or(SketchError::InvalidConstraintReference(constraint.id))?
        {
            VariableLayout::Line { base } => (*base, 4),
            VariableLayout::Arc { base } => (*base, 5),
            VariableLayout::Circle { base } => (*base, 3),
            VariableLayout::CubicBezier { base } => (*base, 8),
        };
        Ok((base..base + width).collect::<Vec<_>>())
    };
    let union = |mut a: Vec<usize>, b: Vec<usize>| {
        a.extend(b);
        a.sort_unstable();
        a.dedup();
        a
    };
    match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } => match layouts.get(entity) {
            Some(VariableLayout::Line { base }) => Ok(vec![vec![base + 1, base + 3]]),
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::Vertical { entity } => match layouts.get(entity) {
            Some(VariableLayout::Line { base }) => Ok(vec![vec![*base, base + 2]]),
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::Coincident { a, b } => Ok(vec![
            union(point_variables(*a, 0)?, point_variables(*b, 0)?),
            union(point_variables(*a, 1)?, point_variables(*b, 1)?),
        ]),
        SketchConstraintKind::Distance { a, b, .. } => {
            if let Some(entity) = arc_radius_entity(*a, *b)
                && let Some(VariableLayout::Arc { base }) = layouts.get(&entity)
            {
                return Ok(vec![vec![base + 2]]);
            }
            Ok(vec![union(
                union(point_variables(*a, 0)?, point_variables(*a, 1)?),
                union(point_variables(*b, 0)?, point_variables(*b, 1)?),
            )])
        }
        SketchConstraintKind::Radius { entity, .. } => match layouts.get(entity) {
            Some(VariableLayout::Arc { base } | VariableLayout::Circle { base }) => {
                Ok(vec![vec![base + 2]])
            }
            _ => Err(SketchError::InvalidConstraintReference(constraint.id)),
        },
        SketchConstraintKind::FixedPoint { point, .. } => Ok(vec![
            point_variables(*point, 0)?,
            point_variables(*point, 1)?,
        ]),
        SketchConstraintKind::Parallel { a, b }
        | SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Tangent { a, b }
        | SketchConstraintKind::Angle { a, b, .. }
        | SketchConstraintKind::Equal { a, b } => {
            Ok(vec![union(entity_variables(*a)?, entity_variables(*b)?)])
        }
        SketchConstraintKind::Symmetric { a, b, axis } => {
            let variables = union(
                union(
                    union(point_variables(*a, 0)?, point_variables(*a, 1)?),
                    union(point_variables(*b, 0)?, point_variables(*b, 1)?),
                ),
                entity_variables(*axis)?,
            );
            Ok(vec![variables.clone(), variables])
        }
        SketchConstraintKind::Concentric { a, b } => Ok(vec![
            union(
                point_variables(
                    SketchPointRef {
                        entity: *a,
                        point: SketchPointKind::Center,
                    },
                    0,
                )?,
                point_variables(
                    SketchPointRef {
                        entity: *b,
                        point: SketchPointKind::Center,
                    },
                    0,
                )?,
            ),
            union(
                point_variables(
                    SketchPointRef {
                        entity: *a,
                        point: SketchPointKind::Center,
                    },
                    1,
                )?,
                point_variables(
                    SketchPointRef {
                        entity: *b,
                        point: SketchPointKind::Center,
                    },
                    1,
                )?,
            ),
        ]),
        SketchConstraintKind::Collinear { a, b } => {
            let variables = union(entity_variables(*a)?, entity_variables(*b)?);
            Ok(vec![variables.clone(), variables])
        }
        SketchConstraintKind::Midpoint { point, line } => {
            let variables = entity_variables(*line)?;
            Ok(vec![
                union(point_variables(*point, 0)?, variables.clone()),
                union(point_variables(*point, 1)?, variables),
            ])
        }
        SketchConstraintKind::PointOnCurve { point, curve } => Ok(vec![union(
            union(point_variables(*point, 0)?, point_variables(*point, 1)?),
            entity_variables(*curve)?,
        )]),
    }
}

fn structural_constraint_rank(
    equations: &[Vec<usize>],
    variable_count: usize,
    skipped: Option<std::ops::Range<usize>>,
) -> usize {
    let mut variable_owner = vec![None; variable_count];
    let mut rank = 0usize;
    for equation in 0..equations.len() {
        if skipped
            .as_ref()
            .is_some_and(|range| range.contains(&equation))
        {
            continue;
        }
        let mut visited = vec![false; variable_count];
        if augment_constraint_rank(equation, equations, &mut variable_owner, &mut visited) {
            rank += 1;
        }
    }
    rank
}

fn augment_constraint_rank(
    equation: usize,
    equations: &[Vec<usize>],
    variable_owner: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for variable in &equations[equation] {
        if visited[*variable] {
            continue;
        }
        visited[*variable] = true;
        let can_assign = match variable_owner[*variable] {
            None => true,
            Some(owner) => augment_constraint_rank(owner, equations, variable_owner, visited),
        };
        if can_assign {
            variable_owner[*variable] = Some(equation);
            return true;
        }
    }
    false
}

fn solve_constraints(
    entities: &mut [SketchEntity],
    constraints: &[SketchConstraint],
    policy: SketchSolverPolicy,
) -> Result<(), SketchError> {
    let indices = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id(), index))
        .collect::<BTreeMap<_, _>>();

    // Preserve the established branch choice and exact results for simple constraints.
    for constraint in constraints {
        project_constraint(entities, &indices, constraint, constraints)?;
    }
    let mut residual = constraint_residuals(entities, &indices, constraints)?;
    if residuals_converged(&residual, policy.tolerance_mm) {
        return Ok(());
    }

    let (layouts, variable_count) = variable_layouts(entities)?;
    let equation_variables = constraints
        .iter()
        .map(|constraint| constraint_variable_equations(constraint, &layouts))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if equation_variables.len() != residual.len() {
        return Err(SketchError::NonConvergent);
    }
    let active_columns = equation_variables
        .iter()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let active_indices = active_columns
        .iter()
        .enumerate()
        .map(|(active, original)| (*original, active))
        .collect::<BTreeMap<_, _>>();
    let reduced_equation_variables = equation_variables
        .iter()
        .map(|variables| {
            variables
                .iter()
                .map(|variable| active_indices[variable])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let equation_nonzeros = reduced_equation_variables
        .iter()
        .map(Vec::len)
        .sum::<usize>();
    let finite_difference_work = active_columns
        .len()
        .checked_mul(residual.len())
        .and_then(|work| work.checked_mul(2))
        .ok_or(SketchError::ResourceLimit)?;
    let conjugate_gradient_work = active_columns
        .len()
        .checked_mul(equation_nonzeros)
        .and_then(|work| work.checked_mul(2))
        .ok_or(SketchError::ResourceLimit)?;
    let numerical_work = finite_difference_work
        .checked_add(conjugate_gradient_work)
        .and_then(|work| work.checked_mul(usize::from(policy.max_iterations)))
        .ok_or(SketchError::ResourceLimit)?;
    if numerical_work > MAX_SKETCH_NUMERICAL_EVALUATIONS {
        return Err(SketchError::ResourceLimit);
    }

    let mut parameters = pack_solver_parameters(entities);
    if parameters.len() != variable_count {
        return Err(SketchError::NonConvergent);
    }
    let mut objective = residual_objective(&residual);
    let mut damping = policy.initial_damping;

    for _ in 0..policy.max_iterations {
        let jacobian = numerical_sketch_jacobian(
            SketchJacobianContext {
                entities,
                indices: &indices,
                constraints,
                active_columns: &active_columns,
                equation_variables: &reduced_equation_variables,
                policy,
            },
            &parameters,
            &residual,
        )?;
        let Some(step) =
            sketch_least_squares_step(&jacobian, &reduced_equation_variables, &residual, damping)
        else {
            damping *= 10.0;
            if !damping.is_finite() {
                break;
            }
            continue;
        };
        let mut candidate_parameters = parameters.clone();
        for (column, delta) in active_columns.iter().zip(step) {
            candidate_parameters[*column] += delta;
        }
        let mut candidate_entities = entities.to_vec();
        if !unpack_solver_parameters(
            &mut candidate_entities,
            &candidate_parameters,
            &active_columns,
        ) {
            damping *= 10.0;
            continue;
        }
        let candidate_residual = constraint_residuals(&candidate_entities, &indices, constraints)?;
        let candidate_objective = residual_objective(&candidate_residual);
        if candidate_objective.is_finite() && candidate_objective < objective {
            entities.clone_from_slice(&candidate_entities);
            parameters = candidate_parameters;
            residual = candidate_residual;
            objective = candidate_objective;
            if residuals_converged(&residual, policy.tolerance_mm) {
                return Ok(());
            }
            damping = (damping * 0.25).max(f64::EPSILON);
        } else {
            damping *= 10.0;
            if !damping.is_finite() {
                break;
            }
        }
    }

    Err(SketchError::NonConvergent)
}

fn pack_solver_parameters(entities: &[SketchEntity]) -> Vec<f64> {
    let mut parameters = Vec::new();
    for entity in entities {
        match entity {
            SketchEntity::Line {
                start_mm, end_mm, ..
            } => parameters.extend([start_mm[0], start_mm[1], end_mm[0], end_mm[1]]),
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                let radius = distance2(*start_mm, *center_mm);
                parameters.extend([
                    center_mm[0],
                    center_mm[1],
                    radius,
                    (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]),
                    (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]),
                ]);
            }
            SketchEntity::Circle {
                center_mm,
                radius_mm,
                ..
            } => parameters.extend([center_mm[0], center_mm[1], *radius_mm]),
            SketchEntity::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
                ..
            } => parameters.extend([
                start_mm[0],
                start_mm[1],
                control_1_mm[0],
                control_1_mm[1],
                control_2_mm[0],
                control_2_mm[1],
                end_mm[0],
                end_mm[1],
            ]),
        }
    }
    parameters
}

fn unpack_solver_parameters(
    entities: &mut [SketchEntity],
    parameters: &[f64],
    active_columns: &[usize],
) -> bool {
    if parameters.iter().any(|value| !value.is_finite()) {
        return false;
    }
    let mut offset = 0;
    for entity in entities {
        let width = entity.degrees_of_freedom();
        let active = active_columns
            .iter()
            .any(|column| (offset..offset + width).contains(column));
        let Some(values) = parameters.get(offset..offset + width) else {
            return false;
        };
        if active {
            match entity {
                SketchEntity::Line {
                    start_mm, end_mm, ..
                } => {
                    *start_mm = [values[0], values[1]];
                    *end_mm = [values[2], values[3]];
                }
                SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } => {
                    let radius = values[2];
                    if radius <= EPSILON_MM {
                        return false;
                    }
                    *center_mm = [values[0], values[1]];
                    *start_mm = [
                        values[0] + radius * values[3].cos(),
                        values[1] + radius * values[3].sin(),
                    ];
                    *end_mm = [
                        values[0] + radius * values[4].cos(),
                        values[1] + radius * values[4].sin(),
                    ];
                }
                SketchEntity::Circle {
                    center_mm,
                    radius_mm,
                    ..
                } => {
                    if values[2] <= EPSILON_MM {
                        return false;
                    }
                    *center_mm = [values[0], values[1]];
                    *radius_mm = values[2];
                }
                SketchEntity::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                    ..
                } => {
                    *start_mm = [values[0], values[1]];
                    *control_1_mm = [values[2], values[3]];
                    *control_2_mm = [values[4], values[5]];
                    *end_mm = [values[6], values[7]];
                }
            }
        }
        offset += width;
    }
    offset == parameters.len()
}

fn solver_parameter_step(entities: &[SketchEntity], column: usize, base_step: f64) -> f64 {
    let mut offset = 0;
    for entity in entities {
        let width = entity.degrees_of_freedom();
        if (offset..offset + width).contains(&column) {
            if let SketchEntity::Arc {
                center_mm,
                start_mm,
                ..
            } = entity
            {
                let local = column - offset;
                let coordinate_scale = center_mm[0].abs().max(center_mm[1].abs()).max(1.0);
                let representable_mm = 64.0 * f64::EPSILON * coordinate_scale;
                if local == 2 {
                    return base_step.max(representable_mm);
                }
                if matches!(local, 3 | 4) {
                    let radius = distance2(*start_mm, *center_mm).max(EPSILON_MM);
                    return base_step.max(representable_mm / radius).min(0.25);
                }
            }
            return base_step;
        }
        offset += width;
    }
    base_step
}

struct SketchJacobianContext<'a> {
    entities: &'a [SketchEntity],
    indices: &'a BTreeMap<SketchEntityId, usize>,
    constraints: &'a [SketchConstraint],
    active_columns: &'a [usize],
    equation_variables: &'a [Vec<usize>],
    policy: SketchSolverPolicy,
}

fn numerical_sketch_jacobian(
    context: SketchJacobianContext<'_>,
    parameters: &[f64],
    baseline: &[f64],
) -> Result<Vec<Vec<f64>>, SketchError> {
    let mut jacobian = vec![vec![0.0; context.active_columns.len()]; baseline.len()];
    for (active_column, column) in context.active_columns.iter().copied().enumerate() {
        let step = solver_parameter_step(
            context.entities,
            column,
            context.policy.finite_difference_step,
        );
        let mut positive_parameters = parameters.to_vec();
        positive_parameters[column] += step;
        let mut positive_entities = context.entities.to_vec();
        let positive =
            unpack_solver_parameters(&mut positive_entities, &positive_parameters, &[column])
                .then(|| {
                    constraint_residuals(&positive_entities, context.indices, context.constraints)
                })
                .transpose()?;

        let mut negative_parameters = parameters.to_vec();
        negative_parameters[column] -= step;
        let mut negative_entities = context.entities.to_vec();
        let negative =
            unpack_solver_parameters(&mut negative_entities, &negative_parameters, &[column])
                .then(|| {
                    constraint_residuals(&negative_entities, context.indices, context.constraints)
                })
                .transpose()?;

        for row in 0..baseline.len() {
            if context.equation_variables[row]
                .binary_search(&active_column)
                .is_err()
            {
                continue;
            }
            jacobian[row][active_column] = match (&positive, &negative) {
                (Some(positive), Some(negative)) => (positive[row] - negative[row]) / (2.0 * step),
                (Some(positive), None) => (positive[row] - baseline[row]) / step,
                (None, Some(negative)) => (baseline[row] - negative[row]) / step,
                (None, None) => return Err(SketchError::NonConvergent),
            };
        }
    }
    Ok(jacobian)
}

fn sketch_least_squares_step(
    jacobian: &[Vec<f64>],
    equation_variables: &[Vec<usize>],
    residual: &[f64],
    damping: f64,
) -> Option<Vec<f64>> {
    let columns = jacobian.first().map_or(0, Vec::len);
    if columns == 0 {
        return Some(Vec::new());
    }
    let mut right = vec![0.0; columns];
    let mut diagonal = vec![0.0; columns];
    for (row, variables) in equation_variables.iter().enumerate() {
        for &column in variables {
            let derivative = jacobian[row][column];
            right[column] -= derivative * residual[row];
            diagonal[column] += derivative * derivative;
        }
    }
    let damping_diagonal = diagonal
        .iter()
        .map(|value| damping * value.abs().max(1.0))
        .collect::<Vec<_>>();
    for (value, damping) in diagonal.iter_mut().zip(&damping_diagonal) {
        *value += damping;
    }
    if diagonal
        .iter()
        .any(|value| !value.is_finite() || *value <= f64::EPSILON)
    {
        return None;
    }

    let mut solution = vec![0.0; columns];
    let mut remainder = right.clone();
    let mut preconditioned = remainder
        .iter()
        .zip(&diagonal)
        .map(|(value, diagonal)| value / diagonal)
        .collect::<Vec<_>>();
    let mut direction = preconditioned.clone();
    let mut product = dot_slice(&remainder, &preconditioned);
    let initial_norm = remainder
        .iter()
        .fold(0.0_f64, |norm, value| norm.max(value.abs()));
    if initial_norm <= f64::EPSILON {
        return Some(solution);
    }

    for _ in 0..columns {
        let applied =
            apply_sketch_normal(jacobian, equation_variables, &damping_diagonal, &direction);
        let denominator = dot_slice(&direction, &applied);
        if !denominator.is_finite() {
            return None;
        }
        if denominator <= f64::EPSILON {
            break;
        }
        let alpha = product / denominator;
        if !alpha.is_finite() {
            return None;
        }
        for index in 0..columns {
            solution[index] += alpha * direction[index];
            remainder[index] -= alpha * applied[index];
        }
        let norm = remainder
            .iter()
            .fold(0.0_f64, |norm, value| norm.max(value.abs()));
        if norm <= 1.0e-12 * initial_norm.max(1.0) {
            break;
        }
        preconditioned = remainder
            .iter()
            .zip(&diagonal)
            .map(|(value, diagonal)| value / diagonal)
            .collect();
        let next_product = dot_slice(&remainder, &preconditioned);
        if !next_product.is_finite() {
            return None;
        }
        if product.abs() <= f64::EPSILON {
            break;
        }
        let beta = next_product / product;
        for index in 0..columns {
            direction[index] = preconditioned[index] + beta * direction[index];
        }
        product = next_product;
    }
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
}

fn apply_sketch_normal(
    jacobian: &[Vec<f64>],
    equation_variables: &[Vec<usize>],
    damping_diagonal: &[f64],
    vector: &[f64],
) -> Vec<f64> {
    let mut result = vector
        .iter()
        .zip(damping_diagonal)
        .map(|(value, damping)| value * damping)
        .collect::<Vec<_>>();
    for (row, variables) in equation_variables.iter().enumerate() {
        let projected = variables
            .iter()
            .map(|column| jacobian[row][*column] * vector[*column])
            .sum::<f64>();
        for &column in variables {
            result[column] += jacobian[row][column] * projected;
        }
    }
    result
}

fn dot_slice(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn residuals_converged(residuals: &[f64], tolerance: f64) -> bool {
    residuals
        .iter()
        .all(|residual| residual.is_finite() && residual.abs() <= tolerance)
}

fn residual_objective(residuals: &[f64]) -> f64 {
    residuals.iter().map(|residual| residual * residual).sum()
}

fn project_constraint(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    constraint: &SketchConstraint,
    constraints: &[SketchConstraint],
) -> Result<(), SketchError> {
    match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_mut(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            let y = (start_mm[1] + end_mm[1]) * 0.5;
            start_mm[1] = y;
            end_mm[1] = y;
        }
        SketchConstraintKind::Vertical { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_mut(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            let x = (start_mm[0] + end_mm[0]) * 0.5;
            start_mm[0] = x;
            end_mm[0] = x;
        }
        SketchConstraintKind::Coincident { a, b } => {
            let a_position = solved_point(entities, indices, *a, constraint.id)?;
            let b_position = solved_point(entities, indices, *b, constraint.id)?;
            let midpoint = [
                (a_position[0] + b_position[0]) * 0.5,
                (a_position[1] + b_position[1]) * 0.5,
            ];
            set_solved_point(entities, indices, *a, midpoint, constraint.id)?;
            set_solved_point(entities, indices, *b, midpoint, constraint.id)?;
        }
        SketchConstraintKind::Distance { a, b, value } => {
            let expected = valid_positive_dimension(value)?;
            if let Some(entity) = arc_radius_entity(*a, *b) {
                let SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } = entity_mut(entities, indices, entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                *start_mm = point_at_radius(*start_mm, *center_mm, expected, [1.0, 0.0]);
                *end_mm = point_at_radius(*end_mm, *center_mm, expected, [0.0, 1.0]);
                return Ok(());
            }
            let a_position = solved_point(entities, indices, *a, constraint.id)?;
            let b_position = solved_point(entities, indices, *b, constraint.id)?;
            let delta = [b_position[0] - a_position[0], b_position[1] - a_position[1]];
            let length = delta[0].hypot(delta[1]);
            let direction = if length <= EPSILON_MM {
                [1.0, 0.0]
            } else {
                [delta[0] / length, delta[1] / length]
            };
            let correction = (expected - length) * 0.5;
            set_solved_point(
                entities,
                indices,
                *a,
                [
                    a_position[0] - direction[0] * correction,
                    a_position[1] - direction[1] * correction,
                ],
                constraint.id,
            )?;
            set_solved_point(
                entities,
                indices,
                *b,
                [
                    b_position[0] + direction[0] * correction,
                    b_position[1] + direction[1] * correction,
                ],
                constraint.id,
            )?;
        }
        SketchConstraintKind::Radius { entity, value } => {
            let expected = valid_positive_dimension(value)?;
            match entity_mut(entities, indices, *entity, constraint.id)? {
                SketchEntity::Circle { radius_mm, .. } => *radius_mm = expected,
                SketchEntity::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    ..
                } => {
                    *start_mm = point_at_radius(*start_mm, *center_mm, expected, [1.0, 0.0]);
                    *end_mm = point_at_radius(*end_mm, *center_mm, expected, [0.0, 1.0]);
                }
                SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                }
            }
        }
        SketchConstraintKind::FixedPoint { point, position_mm } => {
            set_fixed_point(
                entities,
                indices,
                *point,
                *position_mm,
                constraint.id,
                constraints,
            )?;
        }
        SketchConstraintKind::Parallel { .. }
        | SketchConstraintKind::Perpendicular { .. }
        | SketchConstraintKind::Tangent { .. }
        | SketchConstraintKind::Angle { .. }
        | SketchConstraintKind::Equal { .. }
        | SketchConstraintKind::Symmetric { .. }
        | SketchConstraintKind::Concentric { .. }
        | SketchConstraintKind::Collinear { .. }
        | SketchConstraintKind::Midpoint { .. }
        | SketchConstraintKind::PointOnCurve { .. } => {}
    }
    Ok(())
}

fn line_frame(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<([f64; 2], [f64; 2], f64), SketchError> {
    let SketchEntity::Line {
        start_mm, end_mm, ..
    } = entity_ref(entities, indices, id, constraint_id)?
    else {
        return Err(SketchError::InvalidConstraintReference(constraint_id));
    };
    let delta = [end_mm[0] - start_mm[0], end_mm[1] - start_mm[1]];
    let length = delta[0].hypot(delta[1]);
    if length <= EPSILON_MM {
        return Err(SketchError::InvalidConstraintReference(constraint_id));
    }
    Ok((*start_mm, [delta[0] / length, delta[1] / length], length))
}

fn circular_geometry(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<([f64; 2], f64), SketchError> {
    match entity_ref(entities, indices, id, constraint_id)? {
        SketchEntity::Arc {
            start_mm,
            center_mm,
            ..
        } => Ok((*center_mm, distance2(*start_mm, *center_mm))),
        SketchEntity::Circle {
            center_mm,
            radius_mm,
            ..
        } => Ok((*center_mm, *radius_mm)),
        SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

fn line_point_distance(point: [f64; 2], origin: [f64; 2], direction: [f64; 2]) -> f64 {
    (point[0] - origin[0]) * direction[1] - (point[1] - origin[1]) * direction[0]
}

fn arc_endpoint_penalty(entity: &SketchEntity, point: [f64; 2]) -> f64 {
    let SketchEntity::Arc {
        start_mm,
        end_mm,
        center_mm,
        clockwise,
        ..
    } = entity
    else {
        return 0.0;
    };
    let radius = distance2(*start_mm, *center_mm);
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    let point_angle = (point[1] - center_mm[1]).atan2(point[0] - center_mm[0]);
    let (sweep, travel) = if *clockwise {
        (
            (start_angle - end_angle).rem_euclid(std::f64::consts::TAU),
            (start_angle - point_angle).rem_euclid(std::f64::consts::TAU),
        )
    } else {
        (
            (end_angle - start_angle).rem_euclid(std::f64::consts::TAU),
            (point_angle - start_angle).rem_euclid(std::f64::consts::TAU),
        )
    };
    if travel <= sweep + EPSILON_MM / radius {
        0.0
    } else {
        distance2(point, *start_mm).min(distance2(point, *end_mm))
    }
}

fn bounded_arc_residual(base: f64, penalty: f64) -> f64 {
    if penalty <= EPSILON_MM {
        base
    } else {
        base.abs().hypot(penalty)
    }
}

fn tangent_residual(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    a: SketchEntityId,
    b: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match (
        entity_ref(entities, indices, a, constraint_id)?,
        entity_ref(entities, indices, b, constraint_id)?,
    ) {
        (
            SketchEntity::Line { .. },
            circular @ (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }),
        ) => {
            let (origin, direction, _) = line_frame(entities, indices, a, constraint_id)?;
            let (center, radius) = circular_geometry(entities, indices, b, constraint_id)?;
            let signed_distance = line_point_distance(center, origin, direction);
            let contact = [
                center[0] - signed_distance * direction[1],
                center[1] + signed_distance * direction[0],
            ];
            Ok(bounded_arc_residual(
                signed_distance.abs() - radius,
                arc_endpoint_penalty(circular, contact),
            ))
        }
        (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }, SketchEntity::Line { .. }) => {
            tangent_residual(entities, indices, b, a, constraint_id)
        }
        (
            SketchEntity::Arc { .. } | SketchEntity::Circle { .. },
            SketchEntity::Arc { .. } | SketchEntity::Circle { .. },
        ) => {
            let (a_center, a_radius) = circular_geometry(entities, indices, a, constraint_id)?;
            let (b_center, b_radius) = circular_geometry(entities, indices, b, constraint_id)?;
            let center_distance = distance2(a_center, b_center);
            let external = center_distance - (a_radius + b_radius);
            let radius_difference = (a_radius - b_radius).abs();
            let internal = center_distance - radius_difference;
            if center_distance <= EPSILON_MM {
                return Ok(if radius_difference <= EPSILON_MM {
                    external
                } else {
                    internal
                });
            }
            let external_branch = external.abs() <= internal.abs();
            let direction = [
                (b_center[0] - a_center[0]) / center_distance,
                (b_center[1] - a_center[1]) / center_distance,
            ];
            let (a_contact, b_contact, base) = if external_branch {
                (
                    [
                        a_center[0] + direction[0] * a_radius,
                        a_center[1] + direction[1] * a_radius,
                    ],
                    [
                        b_center[0] - direction[0] * b_radius,
                        b_center[1] - direction[1] * b_radius,
                    ],
                    external,
                )
            } else if a_radius >= b_radius {
                (
                    [
                        a_center[0] + direction[0] * a_radius,
                        a_center[1] + direction[1] * a_radius,
                    ],
                    [
                        b_center[0] + direction[0] * b_radius,
                        b_center[1] + direction[1] * b_radius,
                    ],
                    internal,
                )
            } else {
                (
                    [
                        a_center[0] - direction[0] * a_radius,
                        a_center[1] - direction[1] * a_radius,
                    ],
                    [
                        b_center[0] - direction[0] * b_radius,
                        b_center[1] - direction[1] * b_radius,
                    ],
                    internal,
                )
            };
            let penalty =
                arc_endpoint_penalty(entity_ref(entities, indices, a, constraint_id)?, a_contact)
                    .hypot(arc_endpoint_penalty(
                        entity_ref(entities, indices, b, constraint_id)?,
                        b_contact,
                    ));
            Ok(bounded_arc_residual(base, penalty))
        }
        (SketchEntity::Line { .. }, SketchEntity::Line { .. })
        | (SketchEntity::CubicBezier { .. }, _)
        | (_, SketchEntity::CubicBezier { .. }) => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

fn entity_measure(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match entity_ref(entities, indices, id, constraint_id)? {
        SketchEntity::Line {
            start_mm, end_mm, ..
        } => Ok(distance2(*start_mm, *end_mm)),
        SketchEntity::Arc {
            start_mm,
            center_mm,
            ..
        } => Ok(distance2(*start_mm, *center_mm)),
        SketchEntity::Circle { radius_mm, .. } => Ok(*radius_mm),
        SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

fn point_on_curve_residual(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    point: [f64; 2],
    curve: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<f64, SketchError> {
    match entity_ref(entities, indices, curve, constraint_id)? {
        SketchEntity::Line { .. } => {
            let (origin, direction, _) = line_frame(entities, indices, curve, constraint_id)?;
            Ok(line_point_distance(point, origin, direction))
        }
        circular @ (SketchEntity::Arc { .. } | SketchEntity::Circle { .. }) => {
            let (center, radius) = circular_geometry(entities, indices, curve, constraint_id)?;
            Ok(bounded_arc_residual(
                distance2(point, center) - radius,
                arc_endpoint_penalty(circular, point),
            ))
        }
        SketchEntity::CubicBezier { .. } => {
            Err(SketchError::InvalidConstraintReference(constraint_id))
        }
    }
}

fn constraint_residuals(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    constraints: &[SketchConstraint],
) -> Result<Vec<f64>, SketchError> {
    let mut residuals = Vec::new();
    for constraint in constraints {
        match &constraint.kind {
            SketchConstraintKind::Horizontal { entity } => {
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.push(end_mm[1] - start_mm[1]);
            }
            SketchConstraintKind::Vertical { entity } => {
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *entity, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.push(end_mm[0] - start_mm[0]);
            }
            SketchConstraintKind::Coincident { a, b } => {
                let a = solved_point(entities, indices, *a, constraint.id)?;
                let b = solved_point(entities, indices, *b, constraint.id)?;
                residuals.extend([a[0] - b[0], a[1] - b[1]]);
            }
            SketchConstraintKind::Distance { a, b, value } => {
                residuals.push(
                    distance2(
                        solved_point(entities, indices, *a, constraint.id)?,
                        solved_point(entities, indices, *b, constraint.id)?,
                    ) - valid_positive_dimension(value)?,
                );
            }
            SketchConstraintKind::Radius { entity, value } => {
                let actual = match entity_ref(entities, indices, *entity, constraint.id)? {
                    SketchEntity::Arc {
                        start_mm,
                        center_mm,
                        ..
                    } => distance2(*start_mm, *center_mm),
                    SketchEntity::Circle { radius_mm, .. } => *radius_mm,
                    SketchEntity::Line { .. } | SketchEntity::CubicBezier { .. } => {
                        return Err(SketchError::InvalidConstraintReference(constraint.id));
                    }
                };
                residuals.push(actual - valid_positive_dimension(value)?);
            }
            SketchConstraintKind::FixedPoint { point, position_mm } => {
                let actual = solved_point(entities, indices, *point, constraint.id)?;
                residuals.extend([actual[0] - position_mm[0], actual[1] - position_mm[1]]);
            }
            SketchConstraintKind::Parallel { a, b } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(a_direction[0] * b_direction[1] - a_direction[1] * b_direction[0]);
            }
            SketchConstraintKind::Perpendicular { a, b } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(a_direction[0] * b_direction[0] + a_direction[1] * b_direction[1]);
            }
            SketchConstraintKind::Tangent { a, b } => {
                residuals.push(tangent_residual(entities, indices, *a, *b, constraint.id)?);
            }
            SketchConstraintKind::Angle {
                a,
                b,
                angle_degrees,
            } => {
                let (_, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (_, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.push(
                    a_direction[0] * b_direction[0] + a_direction[1] * b_direction[1]
                        - angle_degrees.to_radians().cos(),
                );
            }
            SketchConstraintKind::Equal { a, b } => {
                residuals.push(
                    entity_measure(entities, indices, *a, constraint.id)?
                        - entity_measure(entities, indices, *b, constraint.id)?,
                );
            }
            SketchConstraintKind::Symmetric { a, b, axis } => {
                let a = solved_point(entities, indices, *a, constraint.id)?;
                let b = solved_point(entities, indices, *b, constraint.id)?;
                let (origin, direction, _) = line_frame(entities, indices, *axis, constraint.id)?;
                let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                residuals.extend([
                    line_point_distance(midpoint, origin, direction),
                    (b[0] - a[0]) * direction[0] + (b[1] - a[1]) * direction[1],
                ]);
            }
            SketchConstraintKind::Concentric { a, b } => {
                let (a_center, _) = circular_geometry(entities, indices, *a, constraint.id)?;
                let (b_center, _) = circular_geometry(entities, indices, *b, constraint.id)?;
                residuals.extend([a_center[0] - b_center[0], a_center[1] - b_center[1]]);
            }
            SketchConstraintKind::Collinear { a, b } => {
                let (a_origin, a_direction, _) = line_frame(entities, indices, *a, constraint.id)?;
                let (b_origin, b_direction, _) = line_frame(entities, indices, *b, constraint.id)?;
                residuals.extend([
                    a_direction[0] * b_direction[1] - a_direction[1] * b_direction[0],
                    line_point_distance(b_origin, a_origin, a_direction),
                ]);
            }
            SketchConstraintKind::Midpoint { point, line } => {
                let actual = solved_point(entities, indices, *point, constraint.id)?;
                let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity_ref(entities, indices, *line, constraint.id)?
                else {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                };
                residuals.extend([
                    actual[0] - (start_mm[0] + end_mm[0]) * 0.5,
                    actual[1] - (start_mm[1] + end_mm[1]) * 0.5,
                ]);
            }
            SketchConstraintKind::PointOnCurve { point, curve } => {
                residuals.push(point_on_curve_residual(
                    entities,
                    indices,
                    solved_point(entities, indices, *point, constraint.id)?,
                    *curve,
                    constraint.id,
                )?);
            }
        }
    }
    Ok(residuals)
}

fn entity_ref<'a>(
    entities: &'a [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<&'a SketchEntity, SketchError> {
    indices
        .get(&id)
        .and_then(|index| entities.get(*index))
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

fn entity_mut<'a>(
    entities: &'a mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    id: SketchEntityId,
    constraint_id: SketchConstraintId,
) -> Result<&'a mut SketchEntity, SketchError> {
    indices
        .get(&id)
        .and_then(|index| entities.get_mut(*index))
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

fn solved_point(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    constraint_id: SketchConstraintId,
) -> Result<[f64; 2], SketchError> {
    entity_ref(entities, indices, reference.entity, constraint_id)?
        .point(reference.point)
        .ok_or(SketchError::InvalidConstraintReference(constraint_id))
}

fn set_fixed_point(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    position: [f64; 2],
    constraint_id: SketchConstraintId,
    constraints: &[SketchConstraint],
) -> Result<(), SketchError> {
    let preserves_arc_radius = matches!(
        reference.point,
        SketchPointKind::Start | SketchPointKind::End
    ) && constraints.iter().any(|constraint| match &constraint.kind {
        SketchConstraintKind::Radius { entity, .. } => *entity == reference.entity,
        SketchConstraintKind::Distance { a, b, .. } => {
            arc_radius_entity(*a, *b) == Some(reference.entity)
        }
        _ => false,
    });
    if preserves_arc_radius {
        let opposite_point = match reference.point {
            SketchPointKind::Start => SketchPointKind::End,
            SketchPointKind::End => SketchPointKind::Start,
            _ => unreachable!("only arc endpoints preserve an arc radius"),
        };
        let opposite_position = constraints
            .iter()
            .find_map(|constraint| match &constraint.kind {
                SketchConstraintKind::FixedPoint { point, position_mm }
                    if point.entity == reference.entity && point.point == opposite_point =>
                {
                    Some(*position_mm)
                }
                _ => None,
            });
        let constrained_radius = constraints
            .iter()
            .find_map(|constraint| match &constraint.kind {
                SketchConstraintKind::Radius { entity, value } if *entity == reference.entity => {
                    Some(value)
                }
                SketchConstraintKind::Distance { a, b, value }
                    if arc_radius_entity(*a, *b) == Some(reference.entity) =>
                {
                    Some(value)
                }
                _ => None,
            });
        let SketchEntity::Arc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } = entity_mut(entities, indices, reference.entity, constraint_id)?
        else {
            return Err(SketchError::InvalidConstraintReference(constraint_id));
        };
        if let (Some(opposite_position), Some(radius)) = (opposite_position, constrained_radius) {
            let radius = valid_positive_dimension(radius)?;
            let (start, end) = match reference.point {
                SketchPointKind::Start => (position, opposite_position),
                SketchPointKind::End => (opposite_position, position),
                _ => unreachable!("only arc endpoints preserve an arc radius"),
            };
            let chord = [end[0] - start[0], end[1] - start[1]];
            let chord_length = chord[0].hypot(chord[1]);
            if chord_length <= EPSILON_MM || chord_length > 2.0 * radius + EPSILON_MM {
                return Err(SketchError::OverConstrained(constraint_id));
            }
            let midpoint = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
            let height = (radius * radius - (chord_length * 0.5).powi(2))
                .max(0.0)
                .sqrt();
            let normal = [-chord[1] / chord_length, chord[0] / chord_length];
            let centers = [
                [
                    midpoint[0] + normal[0] * height,
                    midpoint[1] + normal[1] * height,
                ],
                [
                    midpoint[0] - normal[0] * height,
                    midpoint[1] - normal[1] * height,
                ],
            ];
            let chosen = if distance2(centers[0], *center_mm) <= distance2(centers[1], *center_mm) {
                centers[0]
            } else {
                centers[1]
            };
            *start_mm = start;
            *end_mm = end;
            *center_mm = chosen;
            return Ok(());
        }
        let current = match reference.point {
            SketchPointKind::Start => *start_mm,
            SketchPointKind::End => *end_mm,
            _ => unreachable!("only arc endpoints preserve an arc radius"),
        };
        let translation = [position[0] - current[0], position[1] - current[1]];
        for point in [start_mm, end_mm, center_mm] {
            point[0] += translation[0];
            point[1] += translation[1];
        }
        return Ok(());
    }
    set_solved_point(entities, indices, reference, position, constraint_id)
}

fn set_solved_point(
    entities: &mut [SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    reference: SketchPointRef,
    position: [f64; 2],
    constraint_id: SketchConstraintId,
) -> Result<(), SketchError> {
    match (
        entity_mut(entities, indices, reference.entity, constraint_id)?,
        reference.point,
    ) {
        (SketchEntity::Line { start_mm, .. }, SketchPointKind::Start)
        | (SketchEntity::CubicBezier { start_mm, .. }, SketchPointKind::Start) => {
            *start_mm = position;
        }
        (SketchEntity::Line { end_mm, .. }, SketchPointKind::End)
        | (SketchEntity::CubicBezier { end_mm, .. }, SketchPointKind::End) => {
            *end_mm = position;
        }
        (SketchEntity::CubicBezier { control_1_mm, .. }, SketchPointKind::Control1) => {
            *control_1_mm = position;
        }
        (SketchEntity::CubicBezier { control_2_mm, .. }, SketchPointKind::Control2) => {
            *control_2_mm = position;
        }
        (SketchEntity::Circle { center_mm, .. }, SketchPointKind::Center) => {
            *center_mm = position;
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::Center,
        ) => {
            let translation = [position[0] - center_mm[0], position[1] - center_mm[1]];
            for point in [start_mm, end_mm] {
                point[0] += translation[0];
                point[1] += translation[1];
            }
            *center_mm = position;
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::Start,
        ) => {
            let radius = distance2(position, *center_mm);
            if radius <= EPSILON_MM {
                return Err(SketchError::InvalidConstraintReference(constraint_id));
            }
            *start_mm = position;
            *end_mm = point_at_radius(*end_mm, *center_mm, radius, [0.0, 1.0]);
        }
        (
            SketchEntity::Arc {
                start_mm,
                end_mm,
                center_mm,
                ..
            },
            SketchPointKind::End,
        ) => {
            let radius = distance2(position, *center_mm);
            if radius <= EPSILON_MM {
                return Err(SketchError::InvalidConstraintReference(constraint_id));
            }
            *end_mm = position;
            *start_mm = point_at_radius(*start_mm, *center_mm, radius, [1.0, 0.0]);
        }
        _ => return Err(SketchError::InvalidConstraintReference(constraint_id)),
    }
    Ok(())
}

fn point_at_radius(
    point: [f64; 2],
    center: [f64; 2],
    radius: f64,
    fallback_direction: [f64; 2],
) -> [f64; 2] {
    let delta = [point[0] - center[0], point[1] - center[1]];
    let length = delta[0].hypot(delta[1]);
    let direction = if length <= EPSILON_MM {
        fallback_direction
    } else {
        [delta[0] / length, delta[1] / length]
    };
    [
        center[0] + direction[0] * radius,
        center[1] + direction[1] * radius,
    ]
}

fn valid_positive_dimension(value: &Dimension) -> Result<f64, SketchError> {
    let canonical = Dimension::new(value.source_token(), value.millimetres())
        .map_err(|_| SketchError::InvalidDimension)?;
    if canonical.millimetres() <= EPSILON_MM {
        return Err(SketchError::InvalidDimension);
    }
    Ok(canonical.millimetres())
}

fn arc_radius_entity(a: SketchPointRef, b: SketchPointRef) -> Option<SketchEntityId> {
    if a.entity != b.entity {
        return None;
    }
    matches!(
        (a.point, b.point),
        (
            SketchPointKind::Center,
            SketchPointKind::Start | SketchPointKind::End
        ) | (
            SketchPointKind::Start | SketchPointKind::End,
            SketchPointKind::Center
        )
    )
    .then_some(a.entity)
}

fn coincidence_root(
    parents: &BTreeMap<SketchPointRef, SketchPointRef>,
    mut point: SketchPointRef,
) -> SketchPointRef {
    while let Some(parent) = parents.get(&point) {
        point = *parent;
    }
    point
}

fn canonical_point_pair(a: SketchPointRef, b: SketchPointRef) -> (SketchPointRef, SketchPointRef) {
    if a <= b { (a, b) } else { (b, a) }
}

fn canonical_entity_pair(a: SketchEntityId, b: SketchEntityId) -> (SketchEntityId, SketchEntityId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn overlapping_relation_signature(
    kind: &SketchConstraintKind,
) -> Option<(u8, SketchEntityId, SketchEntityId)> {
    let (relation, a, b) = match kind {
        SketchConstraintKind::Parallel { a, b } | SketchConstraintKind::Collinear { a, b } => {
            (1, *a, *b)
        }
        SketchConstraintKind::Perpendicular { a, b }
        | SketchConstraintKind::Angle {
            a,
            b,
            angle_degrees: 90.0,
        } => (2, *a, *b),
        _ => return None,
    };
    let (a, b) = canonical_entity_pair(a, b);
    Some((relation, a, b))
}

fn is_circular_entity(entity: Option<&SketchEntity>) -> bool {
    matches!(
        entity,
        Some(SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
    )
}

fn is_curve_entity(entity: Option<&SketchEntity>) -> bool {
    matches!(
        entity,
        Some(SketchEntity::Line { .. } | SketchEntity::Arc { .. } | SketchEntity::Circle { .. })
    )
}

fn supports_tangent(a: Option<&SketchEntity>, b: Option<&SketchEntity>) -> bool {
    is_curve_entity(a) && is_curve_entity(b) && (is_circular_entity(a) || is_circular_entity(b))
}

fn supports_equal(a: Option<&SketchEntity>, b: Option<&SketchEntity>) -> bool {
    matches!(
        (a, b),
        (
            Some(SketchEntity::Line { .. }),
            Some(SketchEntity::Line { .. })
        )
    ) || (is_circular_entity(a) && is_circular_entity(b))
}

fn valid_angle_degrees(angle_degrees: f64) -> bool {
    angle_degrees.is_finite() && angle_degrees > 0.0 && angle_degrees < 180.0
}

fn push_point_ref(bytes: &mut Vec<u8>, reference: SketchPointRef) {
    bytes.extend_from_slice(&reference.entity.0.to_le_bytes());
    bytes.push(match reference.point {
        SketchPointKind::Start => 1,
        SketchPointKind::End => 2,
        SketchPointKind::Center => 3,
        SketchPointKind::Control1 => 4,
        SketchPointKind::Control2 => 5,
    });
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn distance3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
