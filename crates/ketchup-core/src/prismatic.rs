use crate::graph::DerivedIdentity;
use std::fmt;

pub const PRISMATIC_TOLERANCE_V1: &str = "ketchup.prismatic-tolerance.v1";
const MAX_COORDINATE_MM: f64 = 1.0e12;
const ORTHONORMAL_EPSILON: f64 = 1.0e-10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TolerancePolicy {
    epsilon_mm: f64,
}

impl TolerancePolicy {
    pub fn new(epsilon_mm: f64) -> Result<Self, PrismaticError> {
        if !epsilon_mm.is_finite() || epsilon_mm <= 0.0 {
            return Err(PrismaticError::InvalidTolerance);
        }
        Ok(Self { epsilon_mm })
    }

    #[must_use]
    pub const fn id(&self) -> &'static str {
        PRISMATIC_TOLERANCE_V1
    }

    #[must_use]
    pub const fn epsilon_mm(&self) -> f64 {
        self.epsilon_mm
    }
}

impl Default for TolerancePolicy {
    fn default() -> Self {
        Self::new(1.0e-7).expect("the built-in prismatic tolerance is valid")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    min: [f64; 3],
    max: [f64; 3],
}

impl Aabb {
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Result<Self, PrismaticError> {
        if min
            .iter()
            .chain(max.iter())
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            || (0..3).any(|axis| min[axis] > max[axis])
        {
            return Err(PrismaticError::InvalidAabb);
        }
        Ok(Self { min, max })
    }

    pub fn bounded_volume(min: [f64; 3], max: [f64; 3]) -> Result<Self, PrismaticError> {
        let value = Self::new(min, max)?;
        if !value.has_positive_volume() {
            return Err(PrismaticError::EmptyVolume);
        }
        Ok(value)
    }

    #[must_use]
    pub const fn min(&self) -> [f64; 3] {
        self.min
    }

    #[must_use]
    pub const fn max(&self) -> [f64; 3] {
        self.max
    }

    #[must_use]
    pub fn extents(&self) -> [f64; 3] {
        std::array::from_fn(|axis| self.max[axis] - self.min[axis])
    }

    #[must_use]
    pub fn centre(&self) -> [f64; 3] {
        std::array::from_fn(|axis| (self.min[axis] + self.max[axis]) * 0.5)
    }

    #[must_use]
    pub fn has_positive_volume(&self) -> bool {
        (0..3).all(|axis| self.max[axis] > self.min[axis])
    }

    #[must_use]
    pub fn has_positive_area(&self) -> bool {
        self.extents()
            .into_iter()
            .filter(|extent| *extent > 0.0)
            .count()
            >= 2
    }

    #[must_use]
    pub fn volume(&self) -> f64 {
        let extent = self.extents();
        extent[0] * extent[1] * extent[2]
    }

    pub fn inflate(&self, amount: f64) -> Result<Self, PrismaticError> {
        if !amount.is_finite() || amount < 0.0 {
            return Err(PrismaticError::NumericalFailure);
        }
        let min = std::array::from_fn(|axis| self.min[axis] - amount);
        let max = std::array::from_fn(|axis| self.max[axis] + amount);
        Self::new(min, max)
    }

    pub fn conservative_overlaps(
        &self,
        other: &Self,
        tolerance: TolerancePolicy,
    ) -> Result<bool, PrismaticError> {
        validate_policy(tolerance)?;
        Ok((0..3).all(|axis| {
            self.max[axis] + tolerance.epsilon_mm >= other.min[axis]
                && other.max[axis] + tolerance.epsilon_mm >= self.min[axis]
        }))
    }

    pub fn convex_intersection(&self, other: &Self) -> Result<Option<Self>, PrismaticError> {
        let min = std::array::from_fn(|axis| self.min[axis].max(other.min[axis]));
        let max = std::array::from_fn(|axis| self.max[axis].min(other.max[axis]));
        if min.iter().chain(max.iter()).any(|value| !value.is_finite()) {
            return Err(PrismaticError::NumericalFailure);
        }
        if (0..3).any(|axis| min[axis] > max[axis]) {
            Ok(None)
        } else {
            Self::new(min, max).map(Some)
        }
    }

    #[must_use]
    pub fn subset_of(&self, container: &Self) -> bool {
        (0..3).all(|axis| {
            self.min[axis] >= container.min[axis] && self.max[axis] <= container.max[axis]
        })
    }

    pub fn subset_of_expanded(
        &self,
        container: &Self,
        tolerance: TolerancePolicy,
    ) -> Result<bool, PrismaticError> {
        validate_policy(tolerance)?;
        let expanded = container.inflate(tolerance.epsilon_mm)?;
        Ok((0..3).all(|axis| {
            self.min[axis] >= expanded.min[axis] && self.max[axis] <= expanded.max[axis]
        }))
    }

    #[must_use]
    pub fn vertices(&self) -> [[f64; 3]; 8] {
        std::array::from_fn(|index| {
            std::array::from_fn(|axis| {
                if index & (1 << axis) == 0 {
                    self.min[axis]
                } else {
                    self.max[axis]
                }
            })
        })
    }

    pub fn distance_to_point(&self, point: [f64; 3]) -> Result<f64, PrismaticError> {
        ensure_finite(point)?;
        let squared = (0..3)
            .map(|axis| {
                if point[axis] < self.min[axis] {
                    self.min[axis] - point[axis]
                } else if point[axis] > self.max[axis] {
                    point[axis] - self.max[axis]
                } else {
                    0.0
                }
            })
            .map(|distance| distance * distance)
            .sum::<f64>();
        if !squared.is_finite() {
            return Err(PrismaticError::NumericalFailure);
        }
        Ok(squared.sqrt())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct PrismaticComponentKey {
    pub feature_ordinal: u32,
    pub fragment_ordinal: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrismaticComponent {
    pub key: PrismaticComponentKey,
    pub bounds: Aabb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactPrismaticBody {
    stock: Aabb,
    components: Vec<PrismaticComponent>,
}

impl ExactPrismaticBody {
    pub fn solid(stock: Aabb) -> Result<Self, PrismaticError> {
        Self::from_components(
            stock,
            vec![PrismaticComponent {
                key: PrismaticComponentKey {
                    feature_ordinal: 0,
                    fragment_ordinal: 0,
                },
                bounds: stock,
            }],
        )
    }

    pub fn from_components(
        stock: Aabb,
        mut components: Vec<PrismaticComponent>,
    ) -> Result<Self, PrismaticError> {
        if components.is_empty() {
            return Err(PrismaticError::InvalidDecomposition);
        }
        components.sort_by_key(|component| component.key);
        for (index, component) in components.iter().enumerate() {
            if !component.bounds.has_positive_volume()
                || !component.bounds.subset_of(&stock)
                || index > 0 && components[index - 1].key == component.key
            {
                return Err(PrismaticError::InvalidDecomposition);
            }
            for other in &components[..index] {
                if component
                    .bounds
                    .convex_intersection(&other.bounds)?
                    .is_some_and(|intersection| intersection.has_positive_volume())
                {
                    return Err(PrismaticError::InvalidDecomposition);
                }
            }
        }
        Ok(Self { stock, components })
    }

    #[must_use]
    pub const fn stock(&self) -> Aabb {
        self.stock
    }

    #[must_use]
    pub fn components(&self) -> &[PrismaticComponent] {
        &self.components
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    centre: [f64; 3],
    axes: [[f64; 3]; 3],
    half_extents: [f64; 3],
}

impl Obb {
    pub fn new(
        centre: [f64; 3],
        axes: [[f64; 3]; 3],
        half_extents: [f64; 3],
    ) -> Result<Self, PrismaticError> {
        if centre
            .iter()
            .chain(axes.iter().flatten())
            .chain(half_extents.iter())
            .any(|value| !value.is_finite())
            || centre.iter().any(|value| value.abs() > MAX_COORDINATE_MM)
            || half_extents.iter().any(|value| *value < 0.0)
        {
            return Err(PrismaticError::InvalidObb);
        }
        for axis in &axes {
            if (dot(*axis, *axis) - 1.0).abs() > ORTHONORMAL_EPSILON {
                return Err(PrismaticError::InvalidObb);
            }
        }
        for left in 0..3 {
            for right in (left + 1)..3 {
                if dot(axes[left], axes[right]).abs() > ORTHONORMAL_EPSILON {
                    return Err(PrismaticError::InvalidObb);
                }
            }
        }
        Ok(Self {
            centre,
            axes,
            half_extents,
        })
    }

    pub fn from_aabb(value: Aabb) -> Result<Self, PrismaticError> {
        let extents = value.extents();
        Self::new(
            value.centre(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            std::array::from_fn(|axis| extents[axis] * 0.5),
        )
    }

    #[must_use]
    pub const fn centre(&self) -> [f64; 3] {
        self.centre
    }

    #[must_use]
    pub const fn axes(&self) -> [[f64; 3]; 3] {
        self.axes
    }

    #[must_use]
    pub const fn half_extents(&self) -> [f64; 3] {
        self.half_extents
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollisionRelation {
    Separated,
    Intersecting,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrismaticCollision {
    pub relation: CollisionRelation,
    pub physical_intersection: Option<Aabb>,
    pub conservative_intersection: Option<Aabb>,
}

pub fn obb_sat(
    left: &Obb,
    right: &Obb,
    tolerance: TolerancePolicy,
) -> Result<CollisionRelation, PrismaticError> {
    validate_policy(tolerance)?;
    let mut rotation = [[0.0; 3]; 3];
    let mut absolute = [[0.0; 3]; 3];
    for left_axis in 0..3 {
        for right_axis in 0..3 {
            rotation[left_axis][right_axis] = dot(left.axes[left_axis], right.axes[right_axis]);
            absolute[left_axis][right_axis] =
                rotation[left_axis][right_axis].abs() + tolerance.epsilon_mm;
        }
    }
    let world_translation = subtract(right.centre, left.centre);
    let translation: [f64; 3] = std::array::from_fn(|axis| dot(world_translation, left.axes[axis]));
    ensure_finite(rotation.iter().flatten().copied())?;
    ensure_finite(absolute.iter().flatten().copied())?;
    ensure_finite(translation)?;

    for axis in 0..3 {
        let left_radius = left.half_extents[axis];
        let right_radius = (0..3)
            .map(|index| right.half_extents[index] * absolute[axis][index])
            .sum::<f64>();
        if separated(
            translation[axis].abs(),
            left_radius,
            right_radius,
            tolerance,
        )? {
            return Ok(CollisionRelation::Separated);
        }
    }
    for axis in 0..3 {
        let left_radius = (0..3)
            .map(|index| left.half_extents[index] * absolute[index][axis])
            .sum::<f64>();
        let right_radius = right.half_extents[axis];
        let distance = (0..3)
            .map(|index| translation[index] * rotation[index][axis])
            .sum::<f64>()
            .abs();
        if separated(distance, left_radius, right_radius, tolerance)? {
            return Ok(CollisionRelation::Separated);
        }
    }
    for left_axis in 0..3 {
        for right_axis in 0..3 {
            let left_next = (left_axis + 1) % 3;
            let left_last = (left_axis + 2) % 3;
            let right_next = (right_axis + 1) % 3;
            let right_last = (right_axis + 2) % 3;
            let left_radius = left.half_extents[left_next] * absolute[left_last][right_axis]
                + left.half_extents[left_last] * absolute[left_next][right_axis];
            let right_radius = right.half_extents[right_next] * absolute[left_axis][right_last]
                + right.half_extents[right_last] * absolute[left_axis][right_next];
            let distance = (translation[left_last] * rotation[left_next][right_axis]
                - translation[left_next] * rotation[left_last][right_axis])
                .abs();
            if separated(distance, left_radius, right_radius, tolerance)? {
                return Ok(CollisionRelation::Separated);
            }
        }
    }
    Ok(CollisionRelation::Intersecting)
}

pub fn collide_axis_aligned_prisms(
    left: Aabb,
    right: Aabb,
    tolerance: TolerancePolicy,
) -> Result<PrismaticCollision, PrismaticError> {
    if !left.conservative_overlaps(&right, tolerance)? {
        return Ok(PrismaticCollision {
            relation: CollisionRelation::Separated,
            physical_intersection: None,
            conservative_intersection: None,
        });
    }
    let left_obb = Obb::from_aabb(left)?;
    let right_obb = Obb::from_aabb(right)?;
    if obb_sat(&left_obb, &right_obb, tolerance)? == CollisionRelation::Separated {
        return Ok(PrismaticCollision {
            relation: CollisionRelation::Separated,
            physical_intersection: None,
            conservative_intersection: None,
        });
    }
    let physical_intersection = left.convex_intersection(&right)?;
    let conservative_intersection = left
        .inflate(tolerance.epsilon_mm)?
        .convex_intersection(&right.inflate(tolerance.epsilon_mm)?)?;
    if conservative_intersection.is_none() {
        return Err(PrismaticError::NumericalFailure);
    }
    Ok(PrismaticCollision {
        relation: CollisionRelation::Intersecting,
        physical_intersection,
        conservative_intersection,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct JointId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalJoint {
    id: JointId,
    participant_a: DerivedIdentity,
    participant_b: DerivedIdentity,
    volume: Aabb,
}

impl CanonicalJoint {
    pub fn new(
        id: JointId,
        participant_a: DerivedIdentity,
        participant_b: DerivedIdentity,
        volume: Aabb,
    ) -> Result<Self, PrismaticError> {
        if id.0 == 0 {
            return Err(PrismaticError::ReservedJointId);
        }
        if participant_a == participant_b {
            return Err(PrismaticError::DuplicateJointParticipant);
        }
        if !volume.has_positive_volume() || !volume.volume().is_finite() {
            return Err(PrismaticError::EmptyVolume);
        }
        Ok(Self {
            id,
            participant_a,
            participant_b,
            volume,
        })
    }

    #[must_use]
    pub const fn id(&self) -> JointId {
        self.id
    }

    #[must_use]
    pub const fn participant_a(&self) -> &DerivedIdentity {
        &self.participant_a
    }

    #[must_use]
    pub const fn participant_b(&self) -> &DerivedIdentity {
        &self.participant_b
    }

    #[must_use]
    pub const fn volume(&self) -> Aabb {
        self.volume
    }

    #[must_use]
    pub fn connects(&self, left: &DerivedIdentity, right: &DerivedIdentity) -> bool {
        (&self.participant_a == left && &self.participant_b == right)
            || (&self.participant_a == right && &self.participant_b == left)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JointValidationOutcome {
    OverlapInsideDeclaredJointOk,
    OverlapOutsideDeclaredJointError,
    OverlapWithoutJointError,
    DeclaredJointWithEmptyIntersectionError,
}

impl JointValidationOutcome {
    #[must_use]
    pub const fn is_ok(self) -> bool {
        matches!(self, Self::OverlapInsideDeclaredJointOk)
    }
}

pub fn validate_joint_geometry(
    left: &ExactPrismaticBody,
    right: &ExactPrismaticBody,
    declared_joint: Option<&CanonicalJoint>,
    tolerance: TolerancePolicy,
) -> Result<Option<JointValidationOutcome>, PrismaticError> {
    validate_policy(tolerance)?;
    let mut has_penetration = false;
    let mut has_declared_contact = false;
    let mut outside_declared_volume = false;
    for left_component in left.components() {
        for right_component in right.components() {
            let Some(intersection) = left_component
                .bounds
                .convex_intersection(&right_component.bounds)?
            else {
                continue;
            };
            let is_penetration = intersection.has_positive_volume();
            let is_contact = intersection.has_positive_area();
            has_penetration |= is_penetration;
            if declared_joint.is_some() && is_contact {
                has_declared_contact = true;
            }
            if (is_penetration || declared_joint.is_some() && is_contact)
                && let Some(joint) = declared_joint
            {
                for vertex in intersection.vertices() {
                    outside_declared_volume |=
                        joint.volume.distance_to_point(vertex)? > tolerance.epsilon_mm;
                }
            }
        }
    }
    match declared_joint {
        Some(_) if outside_declared_volume => Ok(Some(
            JointValidationOutcome::OverlapOutsideDeclaredJointError,
        )),
        Some(_) if has_penetration || has_declared_contact => {
            Ok(Some(JointValidationOutcome::OverlapInsideDeclaredJointOk))
        }
        Some(_) => Ok(Some(
            JointValidationOutcome::DeclaredJointWithEmptyIntersectionError,
        )),
        None if has_penetration => Ok(Some(JointValidationOutcome::OverlapWithoutJointError)),
        None => Ok(None),
    }
}

pub fn validate_joint_overlap(
    left: Aabb,
    right: Aabb,
    declared_joint: Option<&CanonicalJoint>,
    tolerance: TolerancePolicy,
) -> Result<Option<JointValidationOutcome>, PrismaticError> {
    let collision = collide_axis_aligned_prisms(left, right, tolerance)?;
    let physical = collision
        .physical_intersection
        .filter(Aabb::has_positive_volume);
    match (physical, declared_joint) {
        (Some(intersection), Some(joint)) => {
            let inside = intersection
                .vertices()
                .into_iter()
                .map(|vertex| joint.volume.distance_to_point(vertex))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .all(|distance| distance <= tolerance.epsilon_mm);
            if inside {
                Ok(Some(JointValidationOutcome::OverlapInsideDeclaredJointOk))
            } else {
                Ok(Some(
                    JointValidationOutcome::OverlapOutsideDeclaredJointError,
                ))
            }
        }
        (Some(_), None) => Ok(Some(JointValidationOutcome::OverlapWithoutJointError)),
        (None, Some(_)) => Ok(Some(
            JointValidationOutcome::DeclaredJointWithEmptyIntersectionError,
        )),
        (None, None) => Ok(None),
    }
}

fn validate_policy(tolerance: TolerancePolicy) -> Result<(), PrismaticError> {
    TolerancePolicy::new(tolerance.epsilon_mm).map(|_| ())
}

fn separated(
    distance: f64,
    left_radius: f64,
    right_radius: f64,
    tolerance: TolerancePolicy,
) -> Result<bool, PrismaticError> {
    ensure_finite([distance, left_radius, right_radius])?;
    let bound = left_radius + right_radius + tolerance.epsilon_mm;
    if !bound.is_finite() {
        return Err(PrismaticError::NumericalFailure);
    }
    Ok(distance > bound)
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

fn ensure_finite(values: impl IntoIterator<Item = f64>) -> Result<(), PrismaticError> {
    if values.into_iter().all(f64::is_finite) {
        Ok(())
    } else {
        Err(PrismaticError::NumericalFailure)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrismaticError {
    InvalidTolerance,
    InvalidAabb,
    InvalidObb,
    EmptyVolume,
    InvalidDecomposition,
    ReservedJointId,
    DuplicateJointParticipant,
    NumericalFailure,
}

impl fmt::Display for PrismaticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTolerance => "prismatic tolerance must be finite and positive",
            Self::InvalidAabb => "axis-aligned bounds are invalid or outside the bounded envelope",
            Self::InvalidObb => "oriented bounds are invalid or not orthonormal",
            Self::EmptyVolume => "joint bounds must have finite positive volume",
            Self::InvalidDecomposition => {
                "exact prismatic components must be uniquely keyed, disjoint, and inside stock"
            }
            Self::ReservedJointId => "joint id zero is reserved",
            Self::DuplicateJointParticipant => "a joint must connect two distinct participants",
            Self::NumericalFailure => "prismatic collision arithmetic was numerically uncertain",
        })
    }
}

impl std::error::Error for PrismaticError {}
