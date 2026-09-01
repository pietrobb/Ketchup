use crate::assembly_joint::{
    AssemblyJointId, AssemblyMotionPath, AssemblyMotionSamplingError, AssemblyMotionStudyId,
    sample_assembly_motion_study,
};
use crate::document::{DefinitionId, FeatureKind, OccurrenceId, Snapshot, Transform};
use std::collections::BTreeMap;
use std::fmt;

pub const MECHANICAL_INTERFACE_SCHEMA_V1: &str = "ketchup.mechanical-interface.v1";
pub const MECHANICAL_CONDITION_SCHEMA_V1: &str = "ketchup.mechanical-condition.v1";

const MAX_INTERFACE_COORDINATE_MM: f64 = 1_000_000.0;
const MAX_INTERFACE_AREA_MM2: f64 = 1.0e12;
const MAX_CONDITION_TOLERANCE_MM: f64 = 1_000.0;
const UNIT_NORMAL_EPSILON: f64 = 1.0e-9;
const ORTHONORMAL_EPSILON: f64 = 1.0e-9;
const FRAME_MATCH_EPSILON_MM: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MechanicalInterfaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MechanicalConditionId(pub u64);

/// What the interface is used for mechanically. Roles are structural, not product
/// specific: every role must be backed by at least one condition that proves it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MechanicalRole {
    Mounting,
    Support,
    Guide,
}

impl MechanicalRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mounting => "mounting",
            Self::Support => "support",
            Self::Guide => "guide",
        }
    }
}

/// A planar frame captured from real body geometry, expressed in body-local
/// millimetres: the face centroid, its outward unit normal, its area and the
/// local bounding box of the face itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MechanicalPlanarFrame {
    origin_mm: [f64; 3],
    normal: [f64; 3],
    area_mm2: f64,
    bounds_mm: [[f64; 3]; 2],
}

impl MechanicalPlanarFrame {
    #[must_use]
    pub const fn new(
        origin_mm: [f64; 3],
        normal: [f64; 3],
        area_mm2: f64,
        bounds_mm: [[f64; 3]; 2],
    ) -> Self {
        Self {
            origin_mm,
            normal,
            area_mm2,
            bounds_mm,
        }
    }

    #[must_use]
    pub const fn origin_mm(self) -> [f64; 3] {
        self.origin_mm
    }

    #[must_use]
    pub const fn normal(self) -> [f64; 3] {
        self.normal
    }

    #[must_use]
    pub const fn area_mm2(self) -> f64 {
        self.area_mm2
    }

    #[must_use]
    pub const fn bounds_mm(self) -> [[f64; 3]; 2] {
        self.bounds_mm
    }

    #[must_use]
    pub fn is_valid(self) -> bool {
        let finite_coordinate =
            |value: f64| value.is_finite() && value.abs() <= MAX_INTERFACE_COORDINATE_MM;
        self.origin_mm.into_iter().all(finite_coordinate)
            && self.bounds_mm.into_iter().flatten().all(finite_coordinate)
            && self.area_mm2.is_finite()
            && self.area_mm2 > 0.0
            && self.area_mm2 <= MAX_INTERFACE_AREA_MM2
            && is_unit_vector(self.normal)
            && (0..3).all(|axis| {
                self.bounds_mm[0][axis] <= self.bounds_mm[1][axis]
                    && self.origin_mm[axis] >= self.bounds_mm[0][axis] - FRAME_MATCH_EPSILON_MM
                    && self.origin_mm[axis] <= self.bounds_mm[1][axis] + FRAME_MATCH_EPSILON_MM
            })
            // A planar face is flat: it must be degenerate along its own normal.
            && (0..3)
                .map(|axis| (self.bounds_mm[1][axis] - self.bounds_mm[0][axis]) * self.normal[axis])
                .sum::<f64>()
                .abs()
                <= FRAME_MATCH_EPSILON_MM
    }
}

/// A persisted mechanical interface: a real face of a real body, anchored to the
/// geometry evidence it was captured from.
///
/// * imported bodies are anchored by `face_ordinal` plus the body result
///   fingerprint, so re-importing different geometry invalidates the interface;
/// * authored extruded-profile bodies carry an empty fingerprint and the ordinal
///   selects one of the six canonical box faces, which the validator recomputes
///   from the document itself.
#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalInterface {
    pub(crate) schema: String,
    pub(crate) id: MechanicalInterfaceId,
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) role: MechanicalRole,
    pub(crate) face_ordinal: u32,
    pub(crate) geometry_fingerprint: String,
    pub(crate) frame: MechanicalPlanarFrame,
}

impl MechanicalInterface {
    #[must_use]
    pub fn new(
        id: MechanicalInterfaceId,
        occurrence_id: OccurrenceId,
        role: MechanicalRole,
        face_ordinal: u32,
        geometry_fingerprint: impl Into<String>,
        frame: MechanicalPlanarFrame,
    ) -> Self {
        Self {
            schema: MECHANICAL_INTERFACE_SCHEMA_V1.to_owned(),
            id,
            occurrence_id,
            role,
            face_ordinal,
            geometry_fingerprint: geometry_fingerprint.into(),
            frame,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> MechanicalInterfaceId {
        self.id
    }

    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn role(&self) -> MechanicalRole {
        self.role
    }

    #[must_use]
    pub const fn face_ordinal(&self) -> u32 {
        self.face_ordinal
    }

    #[must_use]
    pub fn geometry_fingerprint(&self) -> &str {
        &self.geometry_fingerprint
    }

    #[must_use]
    pub const fn frame(&self) -> MechanicalPlanarFrame {
        self.frame
    }

    #[must_use]
    pub fn has_valid_shape(&self) -> bool {
        self.schema == MECHANICAL_INTERFACE_SCHEMA_V1
            && self.id.0 != 0
            && self.occurrence_id.0 != 0
            && self.geometry_fingerprint.len() <= 256
            && self.frame.is_valid()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MechanicalAxisAlignment {
    Parallel,
    Perpendicular,
}

impl MechanicalAxisAlignment {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MechanicalConditionKind {
    /// Two faces must stay coplanar at a fixed offset with opposed normals over
    /// the whole path — the general form of "this part is bolted onto that wall".
    PlanarContact {
        first: MechanicalInterfaceId,
        second: MechanicalInterfaceId,
        offset_mm: f64,
        tolerance_mm: f64,
    },
    /// Contact plus lateral containment: the supported face must actually rest on
    /// the supporting face, not merely lie in its plane.
    Support {
        supported: MechanicalInterfaceId,
        supporting: MechanicalInterfaceId,
        tolerance_mm: f64,
    },
    /// The joint axis must keep a fixed geometric relation to an interface normal.
    JointAxisAlignment {
        joint_id: AssemblyJointId,
        interface: MechanicalInterfaceId,
        alignment: MechanicalAxisAlignment,
        tolerance_degrees: f64,
    },
    /// The joint must be able to move over at least the required range.
    JointTravel {
        joint_id: AssemblyJointId,
        minimum: f64,
        maximum: f64,
    },
}

impl MechanicalConditionKind {
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::PlanarContact {
                first,
                second,
                offset_mm,
                tolerance_mm,
            } => {
                first.0 != 0
                    && second.0 != 0
                    && first != second
                    && offset_mm.is_finite()
                    && offset_mm.abs() <= MAX_INTERFACE_COORDINATE_MM
                    && valid_tolerance(tolerance_mm)
            }
            Self::Support {
                supported,
                supporting,
                tolerance_mm,
            } => {
                supported.0 != 0
                    && supporting.0 != 0
                    && supported != supporting
                    && valid_tolerance(tolerance_mm)
            }
            Self::JointAxisAlignment {
                joint_id,
                interface,
                tolerance_degrees,
                ..
            } => {
                joint_id.0 != 0
                    && interface.0 != 0
                    && tolerance_degrees.is_finite()
                    && (0.0..=45.0).contains(&tolerance_degrees)
            }
            Self::JointTravel {
                joint_id,
                minimum,
                maximum,
            } => {
                joint_id.0 != 0
                    && minimum.is_finite()
                    && maximum.is_finite()
                    && minimum <= maximum
                    && minimum.abs() <= MAX_INTERFACE_COORDINATE_MM
                    && maximum.abs() <= MAX_INTERFACE_COORDINATE_MM
            }
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlanarContact { .. } => "planar_contact",
            Self::Support { .. } => "support",
            Self::JointAxisAlignment { .. } => "joint_axis_alignment",
            Self::JointTravel { .. } => "joint_travel",
        }
    }

    #[must_use]
    pub fn interfaces(self) -> Vec<MechanicalInterfaceId> {
        match self {
            Self::PlanarContact { first, second, .. } => vec![first, second],
            Self::Support {
                supported,
                supporting,
                ..
            } => vec![supported, supporting],
            Self::JointAxisAlignment { interface, .. } => vec![interface],
            Self::JointTravel { .. } => Vec::new(),
        }
    }

    #[must_use]
    pub const fn joint_id(self) -> Option<AssemblyJointId> {
        match self {
            Self::PlanarContact { .. } | Self::Support { .. } => None,
            Self::JointAxisAlignment { joint_id, .. } | Self::JointTravel { joint_id, .. } => {
                Some(joint_id)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalCondition {
    pub(crate) schema: String,
    pub(crate) id: MechanicalConditionId,
    pub(crate) kind: MechanicalConditionKind,
}

impl MechanicalCondition {
    #[must_use]
    pub fn new(id: MechanicalConditionId, kind: MechanicalConditionKind) -> Self {
        Self {
            schema: MECHANICAL_CONDITION_SCHEMA_V1.to_owned(),
            id,
            kind,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> MechanicalConditionId {
        self.id
    }

    #[must_use]
    pub const fn kind(&self) -> MechanicalConditionKind {
        self.kind
    }

    #[must_use]
    pub fn has_valid_shape(&self) -> bool {
        self.schema == MECHANICAL_CONDITION_SCHEMA_V1 && self.id.0 != 0 && self.kind.is_valid()
    }
}

fn valid_tolerance(value: f64) -> bool {
    value.is_finite() && (0.0..=MAX_CONDITION_TOLERANCE_MM).contains(&value)
}

fn is_unit_vector(vector: [f64; 3]) -> bool {
    vector.into_iter().all(f64::is_finite)
        && (vector.into_iter().map(|value| value * value).sum::<f64>() - 1.0).abs()
            <= UNIT_NORMAL_EPSILON
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MechanicalViolationKind {
    /// The declared frame does not match the geometry it claims to describe.
    UnverifiableFrame,
    /// The body evidence the frame was captured from is no longer current.
    StaleGeometryEvidence,
    ContactGap {
        measured_mm: f64,
        allowed_mm: f64,
    },
    ContactOrientation {
        measured_cosine: f64,
    },
    SupportLost {
        overlap_mm: f64,
    },
    AxisMisaligned {
        measured_degrees: f64,
        allowed_degrees: f64,
    },
    TravelNotCovered {
        required_minimum: f64,
        required_maximum: f64,
    },
    RoleWithoutCondition {
        role: MechanicalRole,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MechanicalViolation {
    condition_id: Option<MechanicalConditionId>,
    interface_id: Option<MechanicalInterfaceId>,
    progress: f64,
    kind: MechanicalViolationKind,
}

impl MechanicalViolation {
    #[must_use]
    pub const fn condition_id(self) -> Option<MechanicalConditionId> {
        self.condition_id
    }

    #[must_use]
    pub const fn interface_id(self) -> Option<MechanicalInterfaceId> {
        self.interface_id
    }

    #[must_use]
    pub const fn progress(self) -> f64 {
        self.progress
    }

    #[must_use]
    pub const fn kind(self) -> MechanicalViolationKind {
        self.kind
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalContractReport {
    source_revision: u64,
    source_digest: String,
    study_id: AssemblyMotionStudyId,
    evaluated_samples: usize,
    evaluated_conditions: usize,
    violations: Vec<MechanicalViolation>,
}

impl MechanicalContractReport {
    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn study_id(&self) -> AssemblyMotionStudyId {
        self.study_id
    }

    #[must_use]
    pub const fn evaluated_samples(&self) -> usize {
        self.evaluated_samples
    }

    #[must_use]
    pub const fn evaluated_conditions(&self) -> usize {
        self.evaluated_conditions
    }

    #[must_use]
    pub fn violations(&self) -> &[MechanicalViolation] {
        &self.violations
    }

    #[must_use]
    pub fn is_satisfied(&self) -> bool {
        self.violations.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MechanicalContractError {
    StalePath,
    NoConditions,
    UnknownInterface(MechanicalInterfaceId),
    UnknownJoint(AssemblyJointId),
    MissingOccurrence(OccurrenceId),
    UnsupportedOccurrenceTransform(OccurrenceId),
    Sampling(AssemblyMotionSamplingError),
}

impl fmt::Display for MechanicalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StalePath => {
                formatter.write_str("motion path was sampled from a different document revision")
            }
            Self::NoConditions => {
                formatter.write_str("the document declares no mechanical conditions")
            }
            Self::UnknownInterface(id) => {
                write!(formatter, "mechanical interface {} does not exist", id.0)
            }
            Self::UnknownJoint(id) => write!(formatter, "assembly joint {} does not exist", id.0),
            Self::MissingOccurrence(id) => {
                write!(formatter, "occurrence {} has no solved pose", id.0)
            }
            Self::UnsupportedOccurrenceTransform(id) => write!(
                formatter,
                "occurrence {} does not use a rigid orthonormal transform",
                id.0
            ),
            Self::Sampling(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MechanicalContractError {}

impl From<AssemblyMotionSamplingError> for MechanicalContractError {
    fn from(value: AssemblyMotionSamplingError) -> Self {
        Self::Sampling(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WorldFrame {
    origin_mm: [f64; 3],
    normal: [f64; 3],
    corners_mm: [[f64; 3]; 8],
}

pub fn preview_mechanical_contract(
    snapshot: &Snapshot,
    study_id: AssemblyMotionStudyId,
    sample_intervals: u32,
) -> Result<MechanicalContractReport, MechanicalContractError> {
    let path = sample_assembly_motion_study(snapshot, study_id, sample_intervals)?;
    evaluate_mechanical_contract(snapshot, &path)
}

pub fn evaluate_mechanical_contract(
    snapshot: &Snapshot,
    path: &AssemblyMotionPath,
) -> Result<MechanicalContractReport, MechanicalContractError> {
    if path.source_revision() != snapshot.revision_id()
        || path.source_digest() != snapshot.canonical_digest()
    {
        return Err(MechanicalContractError::StalePath);
    }
    let interfaces = snapshot
        .mechanical_interfaces()
        .map(|interface| (interface.id(), interface))
        .collect::<BTreeMap<_, _>>();
    let conditions = snapshot.mechanical_conditions().collect::<Vec<_>>();
    if conditions.is_empty() {
        return Err(MechanicalContractError::NoConditions);
    }

    for condition in &conditions {
        for interface_id in condition.kind().interfaces() {
            if !interfaces.contains_key(&interface_id) {
                return Err(MechanicalContractError::UnknownInterface(interface_id));
            }
        }
        if let Some(joint_id) = condition.kind().joint_id()
            && snapshot.assembly_joint(joint_id).is_none()
        {
            return Err(MechanicalContractError::UnknownJoint(joint_id));
        }
    }

    let mut violations = Vec::new();

    // 1. Every declared frame must be provable against the geometry it names.
    for interface in interfaces.values() {
        if let Some(kind) = verify_interface_frame(snapshot, interface) {
            violations.push(MechanicalViolation {
                condition_id: None,
                interface_id: Some(interface.id()),
                progress: 0.0,
                kind,
            });
        }
    }

    // 2. Every role must be backed by a condition that actually proves it.
    for interface in interfaces.values() {
        let proven = conditions.iter().any(|condition| {
            let uses_interface = condition.kind().interfaces().contains(&interface.id());
            uses_interface
                && matches!(
                    (interface.role(), condition.kind()),
                    (
                        MechanicalRole::Mounting,
                        MechanicalConditionKind::PlanarContact { .. }
                    ) | (
                        MechanicalRole::Support,
                        MechanicalConditionKind::Support { .. }
                    ) | (
                        MechanicalRole::Guide,
                        MechanicalConditionKind::JointAxisAlignment { .. },
                    )
                )
        });
        if !proven {
            violations.push(MechanicalViolation {
                condition_id: None,
                interface_id: Some(interface.id()),
                progress: 0.0,
                kind: MechanicalViolationKind::RoleWithoutCondition {
                    role: interface.role(),
                },
            });
        }
    }

    // 3. Static conditions on the kinematic definition itself.
    for condition in &conditions {
        let MechanicalConditionKind::JointTravel {
            joint_id,
            minimum,
            maximum,
        } = condition.kind()
        else {
            continue;
        };
        let joint = snapshot
            .assembly_joint(joint_id)
            .ok_or(MechanicalContractError::UnknownJoint(joint_id))?;
        let covered = match joint.kind().limits() {
            Some(limits) => limits.min() <= minimum && limits.max() >= maximum,
            None => joint.kind().position().is_some(),
        };
        if !covered {
            violations.push(MechanicalViolation {
                condition_id: Some(condition.id()),
                interface_id: None,
                progress: 0.0,
                kind: MechanicalViolationKind::TravelNotCovered {
                    required_minimum: minimum,
                    required_maximum: maximum,
                },
            });
        }
    }

    // 4. Pose-dependent conditions, evaluated at every sample of the path.
    for sample in path.samples() {
        let mut world_frames = BTreeMap::new();
        for (id, interface) in &interfaces {
            let pose = sample.solution().pose(interface.occurrence_id()).ok_or(
                MechanicalContractError::MissingOccurrence(interface.occurrence_id()),
            )?;
            world_frames.insert(
                *id,
                world_frame(interface, pose.world_transform()).ok_or(
                    MechanicalContractError::UnsupportedOccurrenceTransform(
                        interface.occurrence_id(),
                    ),
                )?,
            );
        }

        for condition in &conditions {
            match condition.kind() {
                MechanicalConditionKind::PlanarContact {
                    first,
                    second,
                    offset_mm,
                    tolerance_mm,
                } => {
                    let a = world_frames[&first];
                    let b = world_frames[&second];
                    let cosine = dot(a.normal, b.normal);
                    if cosine > -1.0 + 1.0e-6 {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(second),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::ContactOrientation {
                                measured_cosine: cosine,
                            },
                        });
                        continue;
                    }
                    let gap_mm = dot(subtract(b.origin_mm, a.origin_mm), a.normal);
                    if (gap_mm - offset_mm).abs() > tolerance_mm {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(second),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::ContactGap {
                                measured_mm: gap_mm,
                                allowed_mm: offset_mm,
                            },
                        });
                    }
                }
                MechanicalConditionKind::Support {
                    supported,
                    supporting,
                    tolerance_mm,
                } => {
                    let base = world_frames[&supporting];
                    let resting = world_frames[&supported];
                    let cosine = dot(base.normal, resting.normal);
                    if cosine > -1.0 + 1.0e-6 {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(supported),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::ContactOrientation {
                                measured_cosine: cosine,
                            },
                        });
                        continue;
                    }
                    let gap_mm = dot(subtract(resting.origin_mm, base.origin_mm), base.normal);
                    if gap_mm.abs() > tolerance_mm {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(supported),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::ContactGap {
                                measured_mm: gap_mm,
                                allowed_mm: 0.0,
                            },
                        });
                        continue;
                    }
                    let overlap_mm = planar_overlap(base, resting);
                    if overlap_mm <= 0.0 {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(supported),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::SupportLost { overlap_mm },
                        });
                    }
                }
                MechanicalConditionKind::JointAxisAlignment {
                    joint_id,
                    interface,
                    alignment,
                    tolerance_degrees,
                } => {
                    let joint = snapshot
                        .assembly_joint(joint_id)
                        .ok_or(MechanicalContractError::UnknownJoint(joint_id))?;
                    let Some(axis) = joint.kind().axis() else {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(interface),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::AxisMisaligned {
                                measured_degrees: 90.0,
                                allowed_degrees: tolerance_degrees,
                            },
                        });
                        continue;
                    };
                    let parent = joint.parent_occurrence_id();
                    let pose = sample
                        .solution()
                        .pose(parent)
                        .ok_or(MechanicalContractError::MissingOccurrence(parent))?;
                    let world_axis =
                        transform_direction(pose.world_transform(), axis.direction_in_parent())
                            .ok_or(MechanicalContractError::UnsupportedOccurrenceTransform(
                                parent,
                            ))?;
                    let cosine = dot(world_axis, world_frames[&interface].normal).clamp(-1.0, 1.0);
                    let measured_degrees = match alignment {
                        MechanicalAxisAlignment::Parallel => cosine.abs().acos().to_degrees(),
                        MechanicalAxisAlignment::Perpendicular => {
                            (std::f64::consts::FRAC_PI_2 - cosine.abs().acos()).to_degrees()
                        }
                    };
                    if measured_degrees > tolerance_degrees {
                        violations.push(MechanicalViolation {
                            condition_id: Some(condition.id()),
                            interface_id: Some(interface),
                            progress: sample.progress(),
                            kind: MechanicalViolationKind::AxisMisaligned {
                                measured_degrees,
                                allowed_degrees: tolerance_degrees,
                            },
                        });
                    }
                }
                MechanicalConditionKind::JointTravel { .. } => {}
            }
        }
    }

    Ok(MechanicalContractReport {
        source_revision: path.source_revision(),
        source_digest: path.source_digest().to_owned(),
        study_id: path.study_id(),
        evaluated_samples: path.samples().len(),
        evaluated_conditions: conditions.len(),
        violations,
    })
}

/// Captures a planar frame from an authored extruded-profile body, so authors and
/// agents anchor interfaces to real model geometry instead of typed-in numbers.
/// Face ordinals follow the canonical box order 0=-X, 1=+X, 2=-Y, 3=+Y, 4=-Z, 5=+Z.
#[must_use]
pub fn capture_authored_face_frame(
    snapshot: &Snapshot,
    occurrence_id: OccurrenceId,
    face_ordinal: u32,
) -> Option<MechanicalPlanarFrame> {
    let occurrence = snapshot.occurrence(occurrence_id)?;
    let box_mm = authored_box(snapshot, occurrence.definition_id())?;
    authored_box_face(box_mm, face_ordinal)
}

/// Proves the declared frame against the geometry it claims to describe.
/// Returns `None` when the frame is provable.
fn verify_interface_frame(
    snapshot: &Snapshot,
    interface: &MechanicalInterface,
) -> Option<MechanicalViolationKind> {
    if !interface.has_valid_shape() {
        return Some(MechanicalViolationKind::UnverifiableFrame);
    }
    let occurrence = snapshot.occurrence(interface.occurrence_id())?;
    let definition_id = occurrence.definition_id();
    if let Some(evidence) = imported_body_evidence(snapshot, definition_id) {
        let (fingerprint, face_count, bounds_mm) = evidence;
        if interface.geometry_fingerprint() != fingerprint {
            return Some(MechanicalViolationKind::StaleGeometryEvidence);
        }
        if interface.face_ordinal() >= face_count
            || !within_bounds(interface.frame().bounds_mm(), bounds_mm)
        {
            return Some(MechanicalViolationKind::UnverifiableFrame);
        }
        return None;
    }
    let Some(box_mm) = authored_box(snapshot, definition_id) else {
        return Some(MechanicalViolationKind::UnverifiableFrame);
    };
    if !interface.geometry_fingerprint().is_empty() {
        return Some(MechanicalViolationKind::StaleGeometryEvidence);
    }
    let Some(expected) = authored_box_face(box_mm, interface.face_ordinal()) else {
        return Some(MechanicalViolationKind::UnverifiableFrame);
    };
    let frame = interface.frame();
    let matches = (0..3).all(|axis| {
        (frame.origin_mm()[axis] - expected.origin_mm()[axis]).abs() <= FRAME_MATCH_EPSILON_MM
            && (frame.normal()[axis] - expected.normal()[axis]).abs() <= FRAME_MATCH_EPSILON_MM
            && (frame.bounds_mm()[0][axis] - expected.bounds_mm()[0][axis]).abs()
                <= FRAME_MATCH_EPSILON_MM
            && (frame.bounds_mm()[1][axis] - expected.bounds_mm()[1][axis]).abs()
                <= FRAME_MATCH_EPSILON_MM
    }) && (frame.area_mm2() - expected.area_mm2()).abs() <= FRAME_MATCH_EPSILON_MM;
    (!matches).then_some(MechanicalViolationKind::UnverifiableFrame)
}

fn imported_body_evidence(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<(String, u32, [[f64; 3]; 2])> {
    snapshot
        .features()
        .filter(|feature| feature.definition_id() == definition_id)
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((
                spec.result_fingerprint.clone(),
                spec.topology_counts.map_or(0, |counts| counts[2]),
                spec.bounds_mm,
            )),
            _ => None,
        })
}

/// The local box of an authored rectangular profile plus extrusion, when the
/// definition is exactly that. Anything else is not verifiable here.
fn authored_box(snapshot: &Snapshot, definition_id: DefinitionId) -> Option<[[f64; 3]; 2]> {
    let mut profile_points = None;
    let mut height_mm = None;
    for feature in snapshot
        .features()
        .filter(|feature| feature.definition_id() == definition_id)
    {
        match feature.kind() {
            FeatureKind::Profile { points_mm } => {
                if profile_points.is_some() {
                    return None;
                }
                profile_points = Some(points_mm.clone());
            }
            FeatureKind::Extrusion { height, .. } => {
                if height_mm.is_some() {
                    return None;
                }
                height_mm = Some(height.millimetres());
            }
            _ => return None,
        }
    }
    let points = profile_points?;
    let height_mm = height_mm?;
    if points.len() != 4 || height_mm <= 0.0 {
        return None;
    }
    let min_x = points
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let is_rectangle = points.iter().all(|point| {
        (point[0] == min_x || point[0] == max_x) && (point[1] == min_y || point[1] == max_y)
    }) && max_x > min_x
        && max_y > min_y;
    if !is_rectangle {
        return None;
    }
    Some([[min_x, min_y, 0.0], [max_x, max_y, height_mm]])
}

/// Canonical face order of an authored box: 0=-X, 1=+X, 2=-Y, 3=+Y, 4=-Z, 5=+Z.
fn authored_box_face(box_mm: [[f64; 3]; 2], ordinal: u32) -> Option<MechanicalPlanarFrame> {
    let axis = (ordinal / 2) as usize;
    if axis >= 3 {
        return None;
    }
    let positive = ordinal % 2 == 1;
    let coordinate = box_mm[usize::from(positive)][axis];
    let mut normal = [0.0; 3];
    normal[axis] = if positive { 1.0 } else { -1.0 };
    let mut min = box_mm[0];
    let mut max = box_mm[1];
    min[axis] = coordinate;
    max[axis] = coordinate;
    let origin_mm = std::array::from_fn(|index| (min[index] + max[index]) * 0.5);
    let extents = (0..3)
        .filter(|index| *index != axis)
        .map(|index| box_mm[1][index] - box_mm[0][index])
        .product::<f64>();
    Some(MechanicalPlanarFrame::new(
        origin_mm,
        normal,
        extents,
        [min, max],
    ))
}

fn within_bounds(inner: [[f64; 3]; 2], outer: [[f64; 3]; 2]) -> bool {
    (0..3).all(|axis| {
        inner[0][axis] >= outer[0][axis] - FRAME_MATCH_EPSILON_MM
            && inner[1][axis] <= outer[1][axis] + FRAME_MATCH_EPSILON_MM
    })
}

fn world_frame(interface: &MechanicalInterface, transform: Transform) -> Option<WorldFrame> {
    let frame = interface.frame();
    let normal = transform_direction(transform, frame.normal())?;
    let origin_mm = transform_point(transform, frame.origin_mm());
    let bounds = frame.bounds_mm();
    let corners_mm = std::array::from_fn(|index| {
        let corner = [
            bounds[index & 1][0],
            bounds[(index >> 1) & 1][1],
            bounds[(index >> 2) & 1][2],
        ];
        transform_point(transform, corner)
    });
    Some(WorldFrame {
        origin_mm,
        normal,
        corners_mm,
    })
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    std::array::from_fn(|row| {
        matrix[row * 4] * point[0]
            + matrix[row * 4 + 1] * point[1]
            + matrix[row * 4 + 2] * point[2]
            + matrix[row * 4 + 3]
    })
}

/// Rotates a direction. Only rigid orthonormal linear parts are accepted, so a
/// normal stays a normal instead of being silently skewed.
fn transform_direction(transform: Transform, direction: [f64; 3]) -> Option<[f64; 3]> {
    let matrix = transform.matrix();
    let rows: [[f64; 3]; 3] =
        std::array::from_fn(|row| std::array::from_fn(|column| matrix[row * 4 + column]));
    for (index, row) in rows.iter().enumerate() {
        if (dot(*row, *row) - 1.0).abs() > ORTHONORMAL_EPSILON {
            return None;
        }
        for other in rows.iter().skip(index + 1) {
            if dot(*row, *other).abs() > ORTHONORMAL_EPSILON {
                return None;
            }
        }
    }
    let rotated: [f64; 3] = std::array::from_fn(|row| dot(rows[row], direction));
    rotated.into_iter().all(f64::is_finite).then_some(rotated)
}

fn planar_overlap(base: WorldFrame, resting: WorldFrame) -> f64 {
    let (first_axis, second_axis) = in_plane_axes(base.normal);
    [first_axis, second_axis]
        .into_iter()
        .map(|axis| {
            let base_span = projected_span(base.corners_mm, axis);
            let resting_span = projected_span(resting.corners_mm, axis);
            base_span[1].min(resting_span[1]) - base_span[0].max(resting_span[0])
        })
        .fold(f64::INFINITY, f64::min)
}

fn projected_span(corners: [[f64; 3]; 8], axis: [f64; 3]) -> [f64; 2] {
    corners
        .into_iter()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |span, corner| {
            let value = dot(corner, axis);
            [span[0].min(value), span[1].max(value)]
        })
}

fn in_plane_axes(normal: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let seed = if normal[0].abs() <= normal[1].abs() && normal[0].abs() <= normal[2].abs() {
        [1.0, 0.0, 0.0]
    } else if normal[1].abs() <= normal[2].abs() {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let first = normalize(cross(normal, seed));
    let second = normalize(cross(normal, first));
    (first, second)
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 {
        vector.map(|value| value / length)
    } else {
        vector
    }
}

fn cross(first: [f64; 3], second: [f64; 3]) -> [f64; 3] {
    [
        first[1] * second[2] - first[2] * second[1],
        first[2] * second[0] - first[0] * second[2],
        first[0] * second[1] - first[1] * second[0],
    ]
}

fn dot(first: [f64; 3], second: [f64; 3]) -> f64 {
    first[0] * second[0] + first[1] * second[1] + first[2] * second[2]
}

fn subtract(first: [f64; 3], second: [f64; 3]) -> [f64; 3] {
    [
        first[0] - second[0],
        first[1] - second[1],
        first[2] - second[2],
    ]
}
