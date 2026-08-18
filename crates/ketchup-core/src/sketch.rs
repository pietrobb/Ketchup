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
}

impl SketchEntity {
    #[must_use]
    pub const fn id(&self) -> SketchEntityId {
        match self {
            Self::Line { id, .. } | Self::Arc { id, .. } | Self::Circle { id, .. } => *id,
        }
    }

    fn degrees_of_freedom(&self) -> usize {
        match self {
            Self::Line { .. } => 4,
            Self::Arc { .. } => 5,
            Self::Circle { .. } => 3,
        }
    }

    fn point(&self, point: SketchPointKind) -> Option<[f64; 2]> {
        match (self, point) {
            (Self::Line { start_mm, .. } | Self::Arc { start_mm, .. }, SketchPointKind::Start) => {
                Some(*start_mm)
            }
            (Self::Line { end_mm, .. } | Self::Arc { end_mm, .. }, SketchPointKind::End) => {
                Some(*end_mm)
            }
            (
                Self::Arc { center_mm, .. } | Self::Circle { center_mm, .. },
                SketchPointKind::Center,
            ) => Some(*center_mm),
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchConstraint {
    pub id: SketchConstraintId,
    pub kind: SketchConstraintKind,
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
pub enum SolvedSketchRegionProfile {
    Polyline(Vec<[f64; 2]>),
    Circle { center_mm: [f64; 2], radius_mm: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SolvedSketchRegion {
    pub id: SketchRegionId,
    pub entity_ids: Vec<SketchEntityId>,
    pub profile: SolvedSketchRegionProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureDirection {
    AlongNormal,
    OppositeNormal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureExtent {
    Blind(Dimension),
}

impl FeatureExtent {
    pub fn validate(&self) -> Result<(), SketchError> {
        let Self::Blind(distance) = self;
        Dimension::new(distance.source_token(), distance.millimetres())
            .map_err(|_| SketchError::InvalidDimension)?;
        if distance.millimetres() <= EPSILON_MM || distance.millimetres() > MAX_ABS_MM {
            return Err(SketchError::InvalidDimension);
        }
        Ok(())
    }

    #[must_use]
    pub const fn distance(&self) -> &Dimension {
        let Self::Blind(distance) = self;
        distance
    }
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
        Ok(self.solve_geometry()?.report)
    }

    pub fn solve_geometry(&self) -> Result<SketchSolution, SketchError> {
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
            if !signatures.insert(signature) {
                return Err(SketchError::OverConstrained(constraint.id));
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
        solve_constraints(&mut solved_entities, &self.constraints)?;
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
        if solution.report.status != SketchSolveStatus::FullyConstrained {
            return Err(SketchError::SketchNotFullyConstrained);
        }
        let mut regions = Vec::new();
        let mut lines = BTreeMap::new();
        for entity in solution.entities {
            match entity {
                SketchEntity::Circle {
                    id,
                    center_mm,
                    radius_mm,
                } => regions.push(SolvedSketchRegion {
                    id: stable_region_id(&[id]),
                    entity_ids: vec![id],
                    profile: SolvedSketchRegionProfile::Circle {
                        center_mm,
                        radius_mm,
                    },
                }),
                SketchEntity::Line {
                    id,
                    start_mm,
                    end_mm,
                } => {
                    lines.insert(id, (start_mm, end_mm));
                }
                SketchEntity::Arc { .. } => return Err(SketchError::UnsupportedRegionGeometry),
            }
        }

        while let Some((&first_id, &(first_start, first_end))) = lines.first_key_value() {
            lines.remove(&first_id);
            let mut entity_ids = vec![first_id];
            let mut points = vec![first_start, first_end];
            let mut current = first_end;
            while distance2(current, first_start) > EPSILON_MM {
                let Some((next_id, start, end, reversed)) =
                    lines.iter().find_map(|(id, (start, end))| {
                        if distance2(*start, current) <= EPSILON_MM {
                            Some((*id, *start, *end, false))
                        } else if distance2(*end, current) <= EPSILON_MM {
                            Some((*id, *start, *end, true))
                        } else {
                            None
                        }
                    })
                else {
                    return Err(SketchError::OpenRegion);
                };
                lines.remove(&next_id);
                entity_ids.push(next_id);
                current = if reversed { start } else { end };
                points.push(current);
                if entity_ids.len() > MAX_SKETCH_ENTITIES {
                    return Err(SketchError::ResourceLimit);
                }
            }
            if entity_ids.len() < 3 || points.len() < 4 {
                return Err(SketchError::OpenRegion);
            }
            points.pop();
            entity_ids.sort_unstable();
            regions.push(SolvedSketchRegion {
                id: stable_region_id(&entity_ids),
                entity_ids,
                profile: SolvedSketchRegionProfile::Polyline(points),
            });
        }
        regions.sort_by_key(|region| region.id);
        if regions.is_empty() || regions.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(SketchError::InvalidRegionIdentity);
        }
        Ok(regions)
    }
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
    };
    Ok((signature, equations))
}

#[derive(Clone, Copy)]
enum VariableLayout {
    Line { base: usize },
    Arc { base: usize },
    Circle { base: usize },
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
) -> Result<(), SketchError> {
    let indices = entities
        .iter()
        .enumerate()
        .map(|(index, entity)| (entity.id(), index))
        .collect::<BTreeMap<_, _>>();
    const MAX_SOLVE_ITERATIONS: usize = 256;
    for _ in 0..MAX_SOLVE_ITERATIONS {
        for constraint in constraints {
            project_constraint(entities, &indices, constraint, constraints)?;
        }
        if constraints.iter().all(|constraint| {
            constraint_residual(entities, &indices, constraint)
                .is_ok_and(|residual| residual <= EPSILON_MM)
        }) {
            return Ok(());
        }
    }
    let conflicting = constraints
        .iter()
        .find(|constraint| {
            constraint_residual(entities, &indices, constraint)
                .is_ok_and(|residual| residual > EPSILON_MM)
        })
        .map_or(SketchConstraintId(0), |constraint| constraint.id);
    Err(SketchError::OverConstrained(conflicting))
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
                SketchEntity::Line { .. } => {
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
    }
    Ok(())
}

fn constraint_residual(
    entities: &[SketchEntity],
    indices: &BTreeMap<SketchEntityId, usize>,
    constraint: &SketchConstraint,
) -> Result<f64, SketchError> {
    match &constraint.kind {
        SketchConstraintKind::Horizontal { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_ref(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            Ok((start_mm[1] - end_mm[1]).abs())
        }
        SketchConstraintKind::Vertical { entity } => {
            let SketchEntity::Line {
                start_mm, end_mm, ..
            } = entity_ref(entities, indices, *entity, constraint.id)?
            else {
                return Err(SketchError::InvalidConstraintReference(constraint.id));
            };
            Ok((start_mm[0] - end_mm[0]).abs())
        }
        SketchConstraintKind::Coincident { a, b } => Ok(distance2(
            solved_point(entities, indices, *a, constraint.id)?,
            solved_point(entities, indices, *b, constraint.id)?,
        )),
        SketchConstraintKind::Distance { a, b, value } => {
            Ok((distance2(
                solved_point(entities, indices, *a, constraint.id)?,
                solved_point(entities, indices, *b, constraint.id)?,
            ) - valid_positive_dimension(value)?)
            .abs())
        }
        SketchConstraintKind::Radius { entity, value } => {
            let actual = match entity_ref(entities, indices, *entity, constraint.id)? {
                SketchEntity::Arc {
                    start_mm,
                    center_mm,
                    ..
                } => distance2(*start_mm, *center_mm),
                SketchEntity::Circle { radius_mm, .. } => *radius_mm,
                SketchEntity::Line { .. } => {
                    return Err(SketchError::InvalidConstraintReference(constraint.id));
                }
            };
            Ok((actual - valid_positive_dimension(value)?).abs())
        }
        SketchConstraintKind::FixedPoint { point, position_mm } => Ok(distance2(
            solved_point(entities, indices, *point, constraint.id)?,
            *position_mm,
        )),
    }
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
            SketchPointKind::Center => unreachable!("center does not preserve an arc radius"),
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
                SketchPointKind::Center => unreachable!("center does not preserve an arc radius"),
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
            SketchPointKind::Center => unreachable!("center does not preserve an arc radius"),
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
        (SketchEntity::Line { start_mm, .. }, SketchPointKind::Start) => *start_mm = position,
        (SketchEntity::Line { end_mm, .. }, SketchPointKind::End) => *end_mm = position,
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

fn push_point_ref(bytes: &mut Vec<u8>, reference: SketchPointRef) {
    bytes.extend_from_slice(&reference.entity.0.to_le_bytes());
    bytes.push(match reference.point {
        SketchPointKind::Start => 1,
        SketchPointKind::End => 2,
        SketchPointKind::Center => 3,
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
