use crate::document::{
    CanonicalCommand, CommandBatch, DocumentStore, GroupId, OccurrenceId, Proposal,
    ProposalPrepareError, Snapshot, Transform,
};
use crate::mechanical_coupling::{AssemblyMotionCoupling, AssemblyMotionCouplingId};
use crate::prismatic::Aabb;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const ASSEMBLY_JOINT_SCHEMA_V1: &str = "ketchup.assembly-joint.v1";
pub const ASSEMBLY_MOTION_STUDY_SCHEMA_V1: &str = "ketchup.assembly-motion-study.v1";
pub const MAX_ASSEMBLY_MOTION_SAMPLE_INTERVALS: u32 = 10_000;
pub const MAX_ASSEMBLY_MOTION_CLEARANCE_PAIR_SAMPLES: usize = 1_000_000;

const MAX_LINEAR_POSITION_MM: f64 = 1_000_000.0;
const MAX_ANGULAR_POSITION_DEGREES: f64 = 360_000.0;
const MIN_AXIS_DIRECTION_LENGTH: f64 = 1.0e-12;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssemblyJointId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssemblyMotionStudyId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblyJointAxis {
    pub(crate) direction_in_parent: [f64; 3],
    pub(crate) pivot_in_parent_mm: [f64; 3],
}

impl AssemblyJointAxis {
    #[must_use]
    pub fn new(direction_in_parent: [f64; 3], pivot_in_parent_mm: [f64; 3]) -> Self {
        let length = direction_in_parent
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        let direction_in_parent = if length.is_finite() && length >= MIN_AXIS_DIRECTION_LENGTH {
            direction_in_parent.map(|value| value / length)
        } else {
            direction_in_parent
        };
        Self {
            direction_in_parent,
            pivot_in_parent_mm,
        }
    }

    #[must_use]
    pub const fn direction_in_parent(self) -> [f64; 3] {
        self.direction_in_parent
    }

    #[must_use]
    pub const fn pivot_in_parent_mm(self) -> [f64; 3] {
        self.pivot_in_parent_mm
    }

    #[must_use]
    pub fn is_valid(self) -> bool {
        let squared_length = self
            .direction_in_parent
            .iter()
            .map(|value| value * value)
            .sum::<f64>();
        self.direction_in_parent
            .iter()
            .all(|value| value.is_finite())
            && self
                .pivot_in_parent_mm
                .iter()
                .all(|value| value.is_finite() && value.abs() <= MAX_LINEAR_POSITION_MM)
            && (squared_length - 1.0).abs() <= 1.0e-12
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblyJointLimits {
    min: f64,
    max: f64,
}

impl AssemblyJointLimits {
    #[must_use]
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    #[must_use]
    pub const fn min(self) -> f64 {
        self.min
    }

    #[must_use]
    pub const fn max(self) -> f64 {
        self.max
    }

    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        value.is_finite() && self.min <= value && value <= self.max
    }

    fn is_valid_for(self, maximum_absolute_value: f64) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min <= self.max
            && self.min.abs() <= maximum_absolute_value
            && self.max.abs() <= maximum_absolute_value
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssemblyJointKind {
    Fixed,
    Revolute {
        axis: AssemblyJointAxis,
        limits: Option<AssemblyJointLimits>,
        position_degrees: f64,
    },
    Prismatic {
        axis: AssemblyJointAxis,
        limits: Option<AssemblyJointLimits>,
        position_mm: f64,
    },
}

impl AssemblyJointKind {
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Fixed => true,
            Self::Revolute {
                axis,
                limits,
                position_degrees,
            } => {
                axis.is_valid()
                    && position_degrees.is_finite()
                    && position_degrees.abs() <= MAX_ANGULAR_POSITION_DEGREES
                    && limits.is_none_or(|limits| {
                        limits.is_valid_for(MAX_ANGULAR_POSITION_DEGREES)
                            && limits.contains(position_degrees)
                    })
            }
            Self::Prismatic {
                axis,
                limits,
                position_mm,
            } => {
                axis.is_valid()
                    && position_mm.is_finite()
                    && position_mm.abs() <= MAX_LINEAR_POSITION_MM
                    && limits.is_none_or(|limits| {
                        limits.is_valid_for(MAX_LINEAR_POSITION_MM) && limits.contains(position_mm)
                    })
            }
        }
    }

    #[must_use]
    pub const fn axis(self) -> Option<AssemblyJointAxis> {
        match self {
            Self::Fixed => None,
            Self::Revolute { axis, .. } | Self::Prismatic { axis, .. } => Some(axis),
        }
    }

    #[must_use]
    pub const fn limits(self) -> Option<AssemblyJointLimits> {
        match self {
            Self::Fixed => None,
            Self::Revolute { limits, .. } | Self::Prismatic { limits, .. } => limits,
        }
    }

    #[must_use]
    pub const fn position(self) -> Option<f64> {
        match self {
            Self::Fixed => None,
            Self::Revolute {
                position_degrees, ..
            } => Some(position_degrees),
            Self::Prismatic { position_mm, .. } => Some(position_mm),
        }
    }

    #[must_use]
    pub const fn with_position(self, position: f64) -> Option<Self> {
        match self {
            Self::Fixed => None,
            Self::Revolute { axis, limits, .. } => Some(Self::Revolute {
                axis,
                limits,
                position_degrees: position,
            }),
            Self::Prismatic { axis, limits, .. } => Some(Self::Prismatic {
                axis,
                limits,
                position_mm: position,
            }),
        }
    }

    #[must_use]
    pub const fn with_limits(self, replacement: Option<AssemblyJointLimits>) -> Option<Self> {
        match self {
            Self::Fixed => None,
            Self::Revolute {
                axis,
                position_degrees,
                ..
            } => Some(Self::Revolute {
                axis,
                limits: replacement,
                position_degrees,
            }),
            Self::Prismatic {
                axis, position_mm, ..
            } => Some(Self::Prismatic {
                axis,
                limits: replacement,
                position_mm,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyJoint {
    pub(crate) schema: String,
    pub(crate) id: AssemblyJointId,
    pub(crate) parent_occurrence_id: OccurrenceId,
    pub(crate) child_occurrence_id: OccurrenceId,
    pub(crate) kind: AssemblyJointKind,
}

impl AssemblyJoint {
    #[must_use]
    pub fn new(
        id: AssemblyJointId,
        parent_occurrence_id: OccurrenceId,
        child_occurrence_id: OccurrenceId,
        kind: AssemblyJointKind,
    ) -> Self {
        Self {
            schema: ASSEMBLY_JOINT_SCHEMA_V1.to_owned(),
            id,
            parent_occurrence_id,
            child_occurrence_id,
            kind,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> AssemblyJointId {
        self.id
    }

    #[must_use]
    pub const fn parent_occurrence_id(&self) -> OccurrenceId {
        self.parent_occurrence_id
    }

    #[must_use]
    pub const fn child_occurrence_id(&self) -> OccurrenceId {
        self.child_occurrence_id
    }

    #[must_use]
    pub const fn kind(&self) -> AssemblyJointKind {
        self.kind
    }

    #[must_use]
    pub fn has_valid_shape(&self) -> bool {
        self.schema == ASSEMBLY_JOINT_SCHEMA_V1
            && self.id.0 != 0
            && self.parent_occurrence_id.0 != 0
            && self.child_occurrence_id.0 != 0
            && self.parent_occurrence_id != self.child_occurrence_id
            && self.kind.is_valid()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblyMotionDriver {
    joint_id: AssemblyJointId,
    position: f64,
}

impl AssemblyMotionDriver {
    #[must_use]
    pub const fn new(joint_id: AssemblyJointId, position: f64) -> Self {
        Self { joint_id, position }
    }

    #[must_use]
    pub const fn joint_id(self) -> AssemblyJointId {
        self.joint_id
    }

    #[must_use]
    pub const fn position(self) -> f64 {
        self.position
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionStudy {
    pub(crate) schema: String,
    pub(crate) id: AssemblyMotionStudyId,
    pub(crate) name: String,
    pub(crate) drivers: Vec<AssemblyMotionDriver>,
}

impl AssemblyMotionStudy {
    #[must_use]
    pub fn new(
        id: AssemblyMotionStudyId,
        name: impl Into<String>,
        mut drivers: Vec<AssemblyMotionDriver>,
    ) -> Self {
        drivers.sort_by_key(|driver| driver.joint_id());
        Self {
            schema: ASSEMBLY_MOTION_STUDY_SCHEMA_V1.to_owned(),
            id,
            name: name.into(),
            drivers,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> AssemblyMotionStudyId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn drivers(&self) -> &[AssemblyMotionDriver] {
        &self.drivers
    }

    #[must_use]
    pub fn has_valid_shape(&self) -> bool {
        self.schema == ASSEMBLY_MOTION_STUDY_SCHEMA_V1
            && self.id.0 != 0
            && !self.name.trim().is_empty()
            && !self.drivers.is_empty()
            && self
                .drivers
                .iter()
                .all(|driver| driver.joint_id().0 != 0 && driver.position().is_finite())
            && self
                .drivers
                .windows(2)
                .all(|pair| pair[0].joint_id() < pair[1].joint_id())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionSample {
    progress: f64,
    drivers: Vec<AssemblyMotionDriver>,
    solution: AssemblyKinematicSolution,
}

impl AssemblyMotionSample {
    #[must_use]
    pub const fn progress(&self) -> f64 {
        self.progress
    }

    #[must_use]
    pub fn drivers(&self) -> &[AssemblyMotionDriver] {
        &self.drivers
    }

    #[must_use]
    pub const fn solution(&self) -> &AssemblyKinematicSolution {
        &self.solution
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionPath {
    source_revision: u64,
    source_digest: String,
    study_id: AssemblyMotionStudyId,
    sample_intervals: u32,
    samples: Vec<AssemblyMotionSample>,
}

impl AssemblyMotionPath {
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
    pub const fn sample_intervals(&self) -> u32 {
        self.sample_intervals
    }

    #[must_use]
    pub fn samples(&self) -> &[AssemblyMotionSample] {
        &self.samples
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblyMotionCollisionBody {
    occurrence_id: OccurrenceId,
    local_bounds: Aabb,
}

impl AssemblyMotionCollisionBody {
    #[must_use]
    pub const fn new(occurrence_id: OccurrenceId, local_bounds: Aabb) -> Self {
        Self {
            occurrence_id,
            local_bounds,
        }
    }

    #[must_use]
    pub const fn occurrence_id(self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn local_bounds(self) -> Aabb {
        self.local_bounds
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AssemblyMotionCollisionPair {
    first_occurrence_id: OccurrenceId,
    second_occurrence_id: OccurrenceId,
}

impl AssemblyMotionCollisionPair {
    #[must_use]
    pub fn new(
        first_occurrence_id: OccurrenceId,
        second_occurrence_id: OccurrenceId,
    ) -> Option<Self> {
        if first_occurrence_id == second_occurrence_id {
            return None;
        }
        let (first_occurrence_id, second_occurrence_id) =
            if first_occurrence_id < second_occurrence_id {
                (first_occurrence_id, second_occurrence_id)
            } else {
                (second_occurrence_id, first_occurrence_id)
            };
        Some(Self {
            first_occurrence_id,
            second_occurrence_id,
        })
    }

    #[must_use]
    pub const fn first_occurrence_id(self) -> OccurrenceId {
        self.first_occurrence_id
    }

    #[must_use]
    pub const fn second_occurrence_id(self) -> OccurrenceId {
        self.second_occurrence_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblyMotionContact {
    pair: AssemblyMotionCollisionPair,
    progress_start: f64,
    progress_end: f64,
}

impl AssemblyMotionContact {
    #[must_use]
    pub const fn pair(self) -> AssemblyMotionCollisionPair {
        self.pair
    }

    #[must_use]
    pub const fn progress_start(self) -> f64 {
        self.progress_start
    }

    #[must_use]
    pub const fn progress_end(self) -> f64 {
        self.progress_end
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionClearanceAnalysis {
    source_revision: u64,
    source_digest: String,
    minimum_clearance_mm: f64,
    minimum_clearance: AssemblyMotionContact,
    first_contact: Option<AssemblyMotionContact>,
}

impl AssemblyMotionClearanceAnalysis {
    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn minimum_clearance_mm(&self) -> f64 {
        self.minimum_clearance_mm
    }

    #[must_use]
    pub const fn minimum_clearance(&self) -> AssemblyMotionContact {
        self.minimum_clearance
    }

    #[must_use]
    pub const fn first_contact(&self) -> Option<AssemblyMotionContact> {
        self.first_contact
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionClearancePreview {
    path: AssemblyMotionPath,
    clearance: AssemblyMotionClearanceAnalysis,
}

impl AssemblyMotionClearancePreview {
    #[must_use]
    pub const fn path(&self) -> &AssemblyMotionPath {
        &self.path
    }

    #[must_use]
    pub const fn clearance(&self) -> &AssemblyMotionClearanceAnalysis {
        &self.clearance
    }

    #[must_use]
    pub fn final_solution(&self) -> &AssemblyKinematicSolution {
        self.path
            .samples()
            .last()
            .expect("validated motion path contains its final endpoint")
            .solution()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyKinematicPose {
    occurrence_id: OccurrenceId,
    local_transform: Transform,
    world_transform: Transform,
}

impl AssemblyKinematicPose {
    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn local_transform(&self) -> Transform {
        self.local_transform
    }

    #[must_use]
    pub const fn world_transform(&self) -> Transform {
        self.world_transform
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyKinematicSolveStatus {
    UnderConstrained,
    FullyConstrained,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssemblyKinematicJointDiagnostic {
    joint_id: AssemblyJointId,
    remaining_dof: u8,
    driver_count: usize,
}

impl AssemblyKinematicJointDiagnostic {
    #[must_use]
    pub const fn joint_id(&self) -> AssemblyJointId {
        self.joint_id
    }

    #[must_use]
    pub const fn remaining_dof(&self) -> u8 {
        self.remaining_dof
    }

    #[must_use]
    pub const fn driver_count(&self) -> usize {
        self.driver_count
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyKinematicSolution {
    source_revision: u64,
    source_digest: String,
    status: AssemblyKinematicSolveStatus,
    remaining_dof: usize,
    joint_diagnostics: Vec<AssemblyKinematicJointDiagnostic>,
    redundant_driver_joint_ids: Vec<AssemblyJointId>,
    driven_joint_positions: Vec<(AssemblyJointId, f64)>,
    poses: Vec<AssemblyKinematicPose>,
}

impl AssemblyKinematicSolution {
    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn status(&self) -> AssemblyKinematicSolveStatus {
        self.status
    }

    #[must_use]
    pub const fn remaining_dof(&self) -> usize {
        self.remaining_dof
    }

    #[must_use]
    pub fn joint_diagnostics(&self) -> &[AssemblyKinematicJointDiagnostic] {
        &self.joint_diagnostics
    }

    #[must_use]
    pub fn redundant_driver_joint_ids(&self) -> &[AssemblyJointId] {
        &self.redundant_driver_joint_ids
    }

    #[must_use]
    pub fn driven_joint_positions(&self) -> &[(AssemblyJointId, f64)] {
        &self.driven_joint_positions
    }

    #[must_use]
    pub fn poses(&self) -> &[AssemblyKinematicPose] {
        &self.poses
    }

    #[must_use]
    pub fn pose(&self, occurrence_id: OccurrenceId) -> Option<&AssemblyKinematicPose> {
        self.poses
            .binary_search_by_key(&occurrence_id, AssemblyKinematicPose::occurrence_id)
            .ok()
            .map(|index| &self.poses[index])
    }

    pub fn publication_batch(
        &self,
        current: &Snapshot,
    ) -> Result<CommandBatch, AssemblyKinematicPublishError> {
        if current.revision_id() != self.source_revision
            || current.canonical_digest() != self.source_digest
        {
            return Err(AssemblyKinematicPublishError::Stale);
        }
        if self.driven_joint_positions.is_empty() {
            return Err(AssemblyKinematicPublishError::NotMotionStudySolution);
        }

        let changed_joint_positions = self
            .driven_joint_positions
            .iter()
            .filter(|(id, position)| {
                current
                    .assembly_joint(*id)
                    .is_some_and(|joint| joint.kind().position() != Some(*position))
            })
            .copied()
            .collect::<Vec<_>>();
        let required_transform_ids = changed_joint_positions
            .iter()
            .filter_map(|(id, _)| {
                current
                    .assembly_joint(*id)
                    .map(AssemblyJoint::child_occurrence_id)
            })
            .collect::<BTreeSet<_>>();
        let mut commands = changed_joint_positions
            .iter()
            .map(
                |(id, position)| CanonicalCommand::SetAssemblyJointPosition {
                    id: *id,
                    position: *position,
                },
            )
            .collect::<Vec<_>>();
        let transforms = self
            .poses
            .iter()
            .filter_map(|pose| {
                current
                    .occurrence(pose.occurrence_id())
                    .filter(|occurrence| {
                        required_transform_ids.contains(&pose.occurrence_id())
                            || !transforms_equivalent(
                                occurrence.transform(),
                                pose.local_transform(),
                            )
                    })
                    .map(|_| (pose.occurrence_id(), pose.local_transform()))
            })
            .collect::<Vec<_>>();
        if let Some((id, _)) = transforms
            .iter()
            .find(|(id, _)| current.occurrence_is_grounded(*id))
        {
            return Err(AssemblyKinematicPublishError::GroundedOccurrenceWouldMove(
                *id,
            ));
        }
        if !transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: self.source_revision,
                source_digest: self.source_digest.clone(),
                transforms,
            });
        }
        if commands.is_empty() {
            return Err(AssemblyKinematicPublishError::NoCanonicalChanges);
        }
        Ok(CommandBatch::new(commands))
    }

    pub fn prepare_publication(
        &self,
        document: &DocumentStore,
    ) -> Result<Proposal, AssemblyKinematicPublishError> {
        document
            .prepare_proposal(self.publication_batch(&document.current())?)
            .map_err(AssemblyKinematicPublishError::ProposalPreparation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyMotionSamplingError {
    InvalidSampleIntervals(u32),
    Solve(AssemblyKinematicSolveError),
}

impl fmt::Display for AssemblyMotionSamplingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSampleIntervals(intervals) => write!(
                formatter,
                "assembly motion sampling intervals must be between 1 and {MAX_ASSEMBLY_MOTION_SAMPLE_INTERVALS}, got {intervals}"
            ),
            Self::Solve(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblyMotionSamplingError {}

impl From<AssemblyKinematicSolveError> for AssemblyMotionSamplingError {
    fn from(error: AssemblyKinematicSolveError) -> Self {
        Self::Solve(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyMotionClearanceError {
    InsufficientBodies,
    DuplicateOccurrence(OccurrenceId),
    InvalidBodyBounds(OccurrenceId),
    MissingPose(OccurrenceId),
    InvalidClearanceTolerance,
    AnalysisBudgetExceeded,
    NumericalFailure,
}

impl fmt::Display for AssemblyMotionClearanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientBodies => {
                formatter.write_str("assembly motion clearance requires at least two bodies")
            }
            Self::DuplicateOccurrence(id) => write!(
                formatter,
                "assembly motion clearance occurrence {} is duplicated",
                id.0
            ),
            Self::InvalidBodyBounds(id) => write!(
                formatter,
                "assembly motion clearance occurrence {} has empty bounds",
                id.0
            ),
            Self::MissingPose(id) => write!(
                formatter,
                "assembly motion clearance occurrence {} has no sampled pose",
                id.0
            ),
            Self::InvalidClearanceTolerance => {
                formatter.write_str("assembly motion clearance tolerance is invalid")
            }
            Self::AnalysisBudgetExceeded => formatter
                .write_str("assembly motion clearance exceeds the pair-sample analysis budget"),
            Self::NumericalFailure => {
                formatter.write_str("assembly motion clearance calculation failed")
            }
        }
    }
}

impl std::error::Error for AssemblyMotionClearanceError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyMotionClearancePreviewError {
    Sampling(AssemblyMotionSamplingError),
    Clearance(AssemblyMotionClearanceError),
}

impl fmt::Display for AssemblyMotionClearancePreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sampling(error) => error.fmt(formatter),
            Self::Clearance(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblyMotionClearancePreviewError {}

impl From<AssemblyMotionSamplingError> for AssemblyMotionClearancePreviewError {
    fn from(error: AssemblyMotionSamplingError) -> Self {
        Self::Sampling(error)
    }
}

impl From<AssemblyMotionClearanceError> for AssemblyMotionClearancePreviewError {
    fn from(error: AssemblyMotionClearanceError) -> Self {
        Self::Clearance(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyKinematicSolveError {
    MissingMotionStudy(AssemblyMotionStudyId),
    MissingOccurrence(OccurrenceId),
    NonInvertibleTransform(OccurrenceId),
    InvalidJointGraph,
    UnknownDriverJoint(AssemblyJointId),
    FixedJointDriven(AssemblyJointId),
    InvalidDriverPosition(AssemblyJointId),
    OverConstrainedDriver(AssemblyJointId),
    CouplingConflict(AssemblyMotionCouplingId),
    CoupledPositionOutsideLimits(AssemblyJointId),
}

impl fmt::Display for AssemblyKinematicSolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMotionStudy(id) => {
                write!(formatter, "assembly motion study {} is missing", id.0)
            }
            Self::MissingOccurrence(id) => {
                write!(formatter, "assembly occurrence {} is missing", id.0)
            }
            Self::NonInvertibleTransform(id) => write!(
                formatter,
                "assembly occurrence {} has a non-invertible transform",
                id.0
            ),
            Self::InvalidJointGraph => formatter.write_str("assembly joint graph is invalid"),
            Self::UnknownDriverJoint(id) => {
                write!(
                    formatter,
                    "assembly motion driver joint {} is missing",
                    id.0
                )
            }
            Self::FixedJointDriven(id) => write!(
                formatter,
                "fixed assembly joint {} cannot accept a motion driver",
                id.0
            ),
            Self::InvalidDriverPosition(id) => write!(
                formatter,
                "assembly motion driver for joint {} has an invalid position",
                id.0
            ),
            Self::OverConstrainedDriver(id) => write!(
                formatter,
                "assembly joint {} has conflicting motion driver positions",
                id.0
            ),
            Self::CouplingConflict(id) => write!(
                formatter,
                "assembly motion coupling {} conflicts with another driver or coupling",
                id.0
            ),
            Self::CoupledPositionOutsideLimits(id) => write!(
                formatter,
                "assembly motion coupling drives joint {} outside its valid range",
                id.0
            ),
        }
    }
}

impl std::error::Error for AssemblyKinematicSolveError {}

#[derive(Debug, PartialEq)]
pub enum AssemblyKinematicPublishError {
    Stale,
    NotMotionStudySolution,
    NoCanonicalChanges,
    GroundedOccurrenceWouldMove(OccurrenceId),
    ProposalPreparation(ProposalPrepareError),
}

impl fmt::Display for AssemblyKinematicPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("assembly kinematic solution is stale"),
            Self::NotMotionStudySolution => {
                formatter.write_str("assembly kinematic solution is not a motion-study preview")
            }
            Self::NoCanonicalChanges => {
                formatter.write_str("assembly kinematic solution has no canonical changes")
            }
            Self::GroundedOccurrenceWouldMove(id) => write!(
                formatter,
                "assembly kinematic solution would move grounded occurrence {}",
                id.0
            ),
            Self::ProposalPreparation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblyKinematicPublishError {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ValidatedDriver {
    position: f64,
    count: usize,
}

pub fn sample_assembly_motion_study(
    snapshot: &Snapshot,
    study_id: AssemblyMotionStudyId,
    sample_intervals: u32,
) -> Result<AssemblyMotionPath, AssemblyMotionSamplingError> {
    if !(1..=MAX_ASSEMBLY_MOTION_SAMPLE_INTERVALS).contains(&sample_intervals) {
        return Err(AssemblyMotionSamplingError::InvalidSampleIntervals(
            sample_intervals,
        ));
    }
    let study = snapshot
        .assembly_motion_study(study_id)
        .ok_or(AssemblyKinematicSolveError::MissingMotionStudy(study_id))?;
    let endpoints =
        study
            .drivers()
            .iter()
            .map(|driver| {
                let joint = snapshot.assembly_joint(driver.joint_id()).ok_or(
                    AssemblyKinematicSolveError::UnknownDriverJoint(driver.joint_id()),
                )?;
                let start = joint.kind().position().ok_or(
                    AssemblyKinematicSolveError::FixedJointDriven(driver.joint_id()),
                )?;
                Ok((driver.joint_id(), start, driver.position()))
            })
            .collect::<Result<Vec<_>, AssemblyKinematicSolveError>>()?;

    let samples = (0..=sample_intervals)
        .map(|index| {
            let progress = f64::from(index) / f64::from(sample_intervals);
            let drivers = endpoints
                .iter()
                .map(|(joint_id, start, target)| {
                    let position = if index == 0 {
                        *start
                    } else if index == sample_intervals {
                        *target
                    } else {
                        start + (target - start) * progress
                    };
                    AssemblyMotionDriver::new(*joint_id, position)
                })
                .collect::<Vec<_>>();
            let solution = solve_assembly_joint_kinematics_internal(
                snapshot,
                &drivers,
                true,
                &BTreeMap::new(),
            )?;
            Ok(AssemblyMotionSample {
                progress,
                drivers,
                solution,
            })
        })
        .collect::<Result<Vec<_>, AssemblyKinematicSolveError>>()?;

    Ok(AssemblyMotionPath {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        study_id,
        sample_intervals,
        samples,
    })
}

pub fn preview_assembly_motion_study_clearance(
    snapshot: &Snapshot,
    study_id: AssemblyMotionStudyId,
    sample_intervals: u32,
    bodies: &[AssemblyMotionCollisionBody],
    contact_tolerance_mm: f64,
) -> Result<AssemblyMotionClearancePreview, AssemblyMotionClearancePreviewError> {
    let path = sample_assembly_motion_study(snapshot, study_id, sample_intervals)?;
    let clearance = analyze_assembly_motion_clearance(&path, bodies, contact_tolerance_mm)?;
    Ok(AssemblyMotionClearancePreview { path, clearance })
}

pub fn analyze_assembly_motion_clearance(
    path: &AssemblyMotionPath,
    bodies: &[AssemblyMotionCollisionBody],
    contact_tolerance_mm: f64,
) -> Result<AssemblyMotionClearanceAnalysis, AssemblyMotionClearanceError> {
    if !contact_tolerance_mm.is_finite() || contact_tolerance_mm < 0.0 {
        return Err(AssemblyMotionClearanceError::InvalidClearanceTolerance);
    }
    if bodies.len() < 2 {
        return Err(AssemblyMotionClearanceError::InsufficientBodies);
    }
    let mut bodies = bodies.to_vec();
    bodies.sort_by_key(|body| body.occurrence_id());
    for (index, body) in bodies.iter().enumerate() {
        if !body.local_bounds().has_positive_volume() {
            return Err(AssemblyMotionClearanceError::InvalidBodyBounds(
                body.occurrence_id(),
            ));
        }
        if index > 0 && bodies[index - 1].occurrence_id() == body.occurrence_id() {
            return Err(AssemblyMotionClearanceError::DuplicateOccurrence(
                body.occurrence_id(),
            ));
        }
    }
    let pair_count = bodies
        .len()
        .checked_mul(bodies.len() - 1)
        .map(|value| value / 2)
        .ok_or(AssemblyMotionClearanceError::AnalysisBudgetExceeded)?;
    if pair_count
        .checked_mul(path.samples().len())
        .is_none_or(|work| work > MAX_ASSEMBLY_MOTION_CLEARANCE_PAIR_SAMPLES)
    {
        return Err(AssemblyMotionClearanceError::AnalysisBudgetExceeded);
    }

    let sampled_bounds = path
        .samples()
        .iter()
        .map(|sample| {
            bodies
                .iter()
                .map(|body| {
                    let pose = sample.solution().pose(body.occurrence_id()).ok_or(
                        AssemblyMotionClearanceError::MissingPose(body.occurrence_id()),
                    )?;
                    transform_aabb(body.local_bounds(), pose.world_transform())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    let initial_pair =
        AssemblyMotionCollisionPair::new(bodies[0].occurrence_id(), bodies[1].occurrence_id())
            .expect("validated clearance bodies are distinct");
    let mut minimum_clearance_mm = f64::INFINITY;
    let mut minimum_clearance = AssemblyMotionContact {
        pair: initial_pair,
        progress_start: 0.0,
        progress_end: 0.0,
    };
    let mut first_contact = None;
    let mut consider_minimum = |location: AssemblyMotionContact, clearance_mm: f64| {
        if clearance_mm < minimum_clearance_mm {
            minimum_clearance_mm = clearance_mm;
            minimum_clearance = location;
        }
    };

    for first in 0..bodies.len() {
        for second in first + 1..bodies.len() {
            let pair = AssemblyMotionCollisionPair::new(
                bodies[first].occurrence_id(),
                bodies[second].occurrence_id(),
            )
            .expect("pair indices are distinct");
            let location = AssemblyMotionContact {
                pair,
                progress_start: 0.0,
                progress_end: 0.0,
            };
            let clearance_mm = aabb_clearance(sampled_bounds[0][first], sampled_bounds[0][second])?;
            consider_minimum(location, clearance_mm);
            if first_contact.is_none() && clearance_mm <= contact_tolerance_mm {
                first_contact = Some(location);
            }
        }
    }

    for interval in 0..path.sample_intervals() as usize {
        let start_progress = path.samples()[interval].progress();
        let end_progress = path.samples()[interval + 1].progress();
        let mut interval_first_contact = None::<AssemblyMotionContact>;
        for first in 0..bodies.len() {
            for second in first + 1..bodies.len() {
                let pair = AssemblyMotionCollisionPair::new(
                    bodies[first].occurrence_id(),
                    bodies[second].occurrence_id(),
                )
                .expect("pair indices are distinct");
                let first_start_pose = path.samples()[interval]
                    .solution()
                    .pose(bodies[first].occurrence_id())
                    .expect("sampled bounds validated this pose");
                let first_end_pose = path.samples()[interval + 1]
                    .solution()
                    .pose(bodies[first].occurrence_id())
                    .expect("sampled bounds validated this pose");
                let second_start_pose = path.samples()[interval]
                    .solution()
                    .pose(bodies[second].occurrence_id())
                    .expect("sampled bounds validated this pose");
                let second_end_pose = path.samples()[interval + 1]
                    .solution()
                    .pose(bodies[second].occurrence_id())
                    .expect("sampled bounds validated this pose");

                let (clearance_mm, minimum_t, contact_t, conservative_interval) =
                    if same_linear_transform(
                        first_start_pose.world_transform(),
                        first_end_pose.world_transform(),
                    ) && same_linear_transform(
                        second_start_pose.world_transform(),
                        second_end_pose.world_transform(),
                    ) {
                        let (clearance_mm, minimum_t, contact_t) =
                            continuous_translational_aabb_clearance(
                                sampled_bounds[interval][first],
                                sampled_bounds[interval + 1][first],
                                sampled_bounds[interval][second],
                                sampled_bounds[interval + 1][second],
                                contact_tolerance_mm,
                            )?;
                        (clearance_mm, minimum_t, contact_t, false)
                    } else {
                        let first_swept = union_aabb(
                            sampled_bounds[interval][first],
                            sampled_bounds[interval + 1][first],
                        )?;
                        let second_swept = union_aabb(
                            sampled_bounds[interval][second],
                            sampled_bounds[interval + 1][second],
                        )?;
                        let clearance_mm = aabb_clearance(first_swept, second_swept)?;
                        (
                            clearance_mm,
                            0.0,
                            (clearance_mm <= contact_tolerance_mm).then_some(0.0),
                            true,
                        )
                    };
                let minimum_progress = start_progress + (end_progress - start_progress) * minimum_t;
                consider_minimum(
                    AssemblyMotionContact {
                        pair,
                        progress_start: minimum_progress,
                        progress_end: minimum_progress,
                    },
                    clearance_mm,
                );
                if let Some(contact_t) = contact_t {
                    let contact_progress =
                        start_progress + (end_progress - start_progress) * contact_t;
                    let candidate = AssemblyMotionContact {
                        pair,
                        progress_start: contact_progress,
                        progress_end: if conservative_interval {
                            end_progress
                        } else {
                            contact_progress
                        },
                    };
                    if interval_first_contact.is_none_or(|current| {
                        candidate.progress_start() < current.progress_start()
                            || candidate.progress_start() == current.progress_start()
                                && candidate.pair() < current.pair()
                    }) {
                        interval_first_contact = Some(candidate);
                    }
                }
            }
        }
        if first_contact.is_none() {
            first_contact = interval_first_contact;
        }
    }

    Ok(AssemblyMotionClearanceAnalysis {
        source_revision: path.source_revision(),
        source_digest: path.source_digest().to_owned(),
        minimum_clearance_mm,
        minimum_clearance,
        first_contact,
    })
}

pub fn solve_assembly_joint_kinematics(
    snapshot: &Snapshot,
) -> Result<AssemblyKinematicSolution, AssemblyKinematicSolveError> {
    solve_assembly_joint_kinematics_internal(snapshot, &[], false, &BTreeMap::new())
}

pub fn solve_assembly_joint_kinematics_with_drivers(
    snapshot: &Snapshot,
    drivers: &[AssemblyMotionDriver],
) -> Result<AssemblyKinematicSolution, AssemblyKinematicSolveError> {
    solve_assembly_joint_kinematics_internal(snapshot, drivers, false, &BTreeMap::new())
}

pub fn solve_assembly_motion_study(
    snapshot: &Snapshot,
    study_id: AssemblyMotionStudyId,
) -> Result<AssemblyKinematicSolution, AssemblyKinematicSolveError> {
    let study = snapshot
        .assembly_motion_study(study_id)
        .ok_or(AssemblyKinematicSolveError::MissingMotionStudy(study_id))?;
    solve_assembly_joint_kinematics_internal(snapshot, study.drivers(), true, &BTreeMap::new())
}

pub(crate) fn solve_assembly_joint_kinematics_with_kind_overrides(
    snapshot: &Snapshot,
    kind_overrides: &BTreeMap<AssemblyJointId, AssemblyJointKind>,
) -> Result<AssemblyKinematicSolution, AssemblyKinematicSolveError> {
    solve_assembly_joint_kinematics_internal(snapshot, &[], false, kind_overrides)
}

fn solve_assembly_joint_kinematics_internal(
    snapshot: &Snapshot,
    drivers: &[AssemblyMotionDriver],
    motion_study_solution: bool,
    kind_overrides: &BTreeMap<AssemblyJointId, AssemblyJointKind>,
) -> Result<AssemblyKinematicSolution, AssemblyKinematicSolveError> {
    let driver_overrides = validate_driver_overrides(snapshot, drivers)?;
    let position_overrides = propagate_motion_couplings(snapshot, &driver_overrides)?;
    let remaining_dof_joint_ids =
        remaining_dof_joint_ids(snapshot, position_overrides.keys().copied().collect());
    let mut source_world = BTreeMap::new();
    let mut occurrence_parents = BTreeMap::<OccurrenceId, Option<GroupId>>::new();
    for occurrence in snapshot.occurrences() {
        let world = snapshot
            .world_transform_for_occurrence(occurrence.id())
            .ok_or(AssemblyKinematicSolveError::MissingOccurrence(
                occurrence.id(),
            ))?;
        source_world.insert(occurrence.id(), world);
        occurrence_parents.insert(occurrence.id(), occurrence.parent());
    }

    let joints = snapshot.assembly_joints().collect::<Vec<_>>();
    let joint_diagnostics = joints
        .iter()
        .map(|joint| {
            let driver_count = driver_overrides
                .get(&joint.id())
                .map_or(0, |driver| driver.count);
            let remaining_dof = u8::from(remaining_dof_joint_ids.contains(&joint.id()));
            AssemblyKinematicJointDiagnostic {
                joint_id: joint.id(),
                remaining_dof,
                driver_count,
            }
        })
        .collect::<Vec<_>>();
    let remaining_dof = joint_diagnostics
        .iter()
        .map(|diagnostic| usize::from(diagnostic.remaining_dof()))
        .sum();
    let status = if remaining_dof == 0 {
        AssemblyKinematicSolveStatus::FullyConstrained
    } else {
        AssemblyKinematicSolveStatus::UnderConstrained
    };
    let redundant_driver_joint_ids = driver_overrides
        .iter()
        .filter_map(|(id, driver)| (driver.count > 1).then_some(*id))
        .collect();
    let child_ids = joints
        .iter()
        .map(|joint| joint.child_occurrence_id())
        .collect::<BTreeSet<_>>();
    let mut solved_world = source_world
        .iter()
        .filter(|(id, _)| !child_ids.contains(id))
        .map(|(id, transform)| (*id, *transform))
        .collect::<BTreeMap<_, _>>();
    let mut pending = joints;

    while !pending.is_empty() {
        let mut advanced = false;
        let mut deferred = Vec::new();
        for joint in pending {
            let parent_id = joint.parent_occurrence_id();
            let child_id = joint.child_occurrence_id();
            let Some(parent_solved_world) = solved_world.get(&parent_id).copied() else {
                deferred.push(joint);
                continue;
            };
            let parent_source_world = source_world
                .get(&parent_id)
                .copied()
                .ok_or(AssemblyKinematicSolveError::MissingOccurrence(parent_id))?;
            let child_source_world = source_world
                .get(&child_id)
                .copied()
                .ok_or(AssemblyKinematicSolveError::MissingOccurrence(child_id))?;
            let inverse_parent = invert_affine_transform(parent_source_world).ok_or(
                AssemblyKinematicSolveError::NonInvertibleTransform(parent_id),
            )?;
            let target_kind = kind_overrides.get(&joint.id()).copied().unwrap_or_else(|| {
                position_overrides
                    .get(&joint.id())
                    .map_or(joint.kind(), |driver| {
                        joint
                            .kind()
                            .with_position(driver.position)
                            .expect("validated driver targets a movable joint")
                    })
            });
            let inverse_current_motion =
                invert_affine_transform(joint_motion_transform(joint.kind()))
                    .expect("validated assembly joint motion is invertible");
            let delta_motion = joint_motion_transform(target_kind).compose(inverse_current_motion);
            let world = parent_solved_world
                .compose(delta_motion)
                .compose(inverse_parent)
                .compose(child_source_world);
            solved_world.insert(child_id, world);
            advanced = true;
        }
        if !advanced {
            return Err(AssemblyKinematicSolveError::InvalidJointGraph);
        }
        pending = deferred;
    }

    let mut poses = Vec::with_capacity(source_world.len());
    for (occurrence_id, world_transform) in solved_world {
        let local_transform = match occurrence_parents[&occurrence_id] {
            Some(group_id) => {
                let group_world = snapshot.world_transform_for_group(group_id).ok_or(
                    AssemblyKinematicSolveError::MissingOccurrence(occurrence_id),
                )?;
                invert_affine_transform(group_world)
                    .ok_or(AssemblyKinematicSolveError::NonInvertibleTransform(
                        occurrence_id,
                    ))?
                    .compose(world_transform)
            }
            None => world_transform,
        };
        poses.push(AssemblyKinematicPose {
            occurrence_id,
            local_transform,
            world_transform,
        });
    }

    Ok(AssemblyKinematicSolution {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        status,
        remaining_dof,
        joint_diagnostics,
        redundant_driver_joint_ids,
        driven_joint_positions: if motion_study_solution {
            position_overrides
                .iter()
                .map(|(id, driver)| (*id, driver.position))
                .collect()
        } else {
            Vec::new()
        },
        poses,
    })
}

fn validate_driver_overrides(
    snapshot: &Snapshot,
    drivers: &[AssemblyMotionDriver],
) -> Result<BTreeMap<AssemblyJointId, ValidatedDriver>, AssemblyKinematicSolveError> {
    let mut sorted_drivers = drivers.to_vec();
    sorted_drivers.sort_by(|left, right| {
        left.joint_id()
            .cmp(&right.joint_id())
            .then_with(|| left.position().total_cmp(&right.position()))
    });

    let mut overrides = BTreeMap::<AssemblyJointId, ValidatedDriver>::new();
    for driver in sorted_drivers {
        let joint_id = driver.joint_id();
        let joint = snapshot
            .assembly_joint(joint_id)
            .ok_or(AssemblyKinematicSolveError::UnknownDriverJoint(joint_id))?;
        let Some(kind) = joint.kind().with_position(driver.position()) else {
            return Err(AssemblyKinematicSolveError::FixedJointDriven(joint_id));
        };
        if !kind.is_valid() {
            return Err(AssemblyKinematicSolveError::InvalidDriverPosition(joint_id));
        }
        match overrides.get_mut(&joint_id) {
            Some(existing) if existing.position == driver.position() => existing.count += 1,
            Some(_) => {
                return Err(AssemblyKinematicSolveError::OverConstrainedDriver(joint_id));
            }
            None => {
                overrides.insert(
                    joint_id,
                    ValidatedDriver {
                        position: driver.position(),
                        count: 1,
                    },
                );
            }
        }
    }
    Ok(overrides)
}

fn propagate_motion_couplings(
    snapshot: &Snapshot,
    drivers: &BTreeMap<AssemblyJointId, ValidatedDriver>,
) -> Result<BTreeMap<AssemblyJointId, ValidatedDriver>, AssemblyKinematicSolveError> {
    let couplings = snapshot.assembly_motion_couplings().collect::<Vec<_>>();
    let mut positions = drivers.clone();

    for _ in 0..=couplings.len() {
        let mut advanced = false;
        for coupling in &couplings {
            let input = positions.get(&coupling.input_joint_id()).copied();
            let output = positions.get(&coupling.output_joint_id()).copied();
            match (input, output) {
                (Some(input), Some(output)) => {
                    if !coupled_positions_equal(
                        output.position,
                        coupling.output_position(input.position),
                    ) {
                        return Err(AssemblyKinematicSolveError::CouplingConflict(coupling.id()));
                    }
                }
                (Some(input), None) => {
                    insert_coupled_position(
                        snapshot,
                        &mut positions,
                        coupling,
                        coupling.output_joint_id(),
                        coupling.output_position(input.position),
                    )?;
                    advanced = true;
                }
                (None, Some(output)) => {
                    insert_coupled_position(
                        snapshot,
                        &mut positions,
                        coupling,
                        coupling.input_joint_id(),
                        coupling.input_position(output.position),
                    )?;
                    advanced = true;
                }
                (None, None) => {}
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(positions)
}

fn insert_coupled_position(
    snapshot: &Snapshot,
    positions: &mut BTreeMap<AssemblyJointId, ValidatedDriver>,
    coupling: &AssemblyMotionCoupling,
    joint_id: AssemblyJointId,
    position: f64,
) -> Result<(), AssemblyKinematicSolveError> {
    let joint = snapshot
        .assembly_joint(joint_id)
        .ok_or(AssemblyKinematicSolveError::CouplingConflict(coupling.id()))?;
    let kind = joint
        .kind()
        .with_position(position)
        .filter(|kind| kind.is_valid())
        .ok_or(AssemblyKinematicSolveError::CoupledPositionOutsideLimits(
            joint_id,
        ))?;
    let position = kind
        .position()
        .expect("validated coupled joint remains movable");
    positions.insert(joint_id, ValidatedDriver { position, count: 0 });
    Ok(())
}

fn coupled_positions_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= 1.0e-10 * scale
}

fn remaining_dof_joint_ids(
    snapshot: &Snapshot,
    driven: BTreeSet<AssemblyJointId>,
) -> BTreeSet<AssemblyJointId> {
    let movable = snapshot
        .assembly_joints()
        .filter(|joint| joint.kind().position().is_some())
        .map(AssemblyJoint::id)
        .collect::<BTreeSet<_>>();
    let mut adjacency = BTreeMap::<AssemblyJointId, BTreeSet<AssemblyJointId>>::new();
    for coupling in snapshot.assembly_motion_couplings() {
        adjacency
            .entry(coupling.input_joint_id())
            .or_default()
            .insert(coupling.output_joint_id());
        adjacency
            .entry(coupling.output_joint_id())
            .or_default()
            .insert(coupling.input_joint_id());
    }

    let mut pending = movable;
    let mut remaining = BTreeSet::new();
    while let Some(start) = pending.pop_first() {
        let mut component = BTreeSet::from([start]);
        let mut frontier = vec![start];
        while let Some(joint_id) = frontier.pop() {
            for neighbour in adjacency.get(&joint_id).into_iter().flatten() {
                if pending.remove(neighbour) {
                    component.insert(*neighbour);
                    frontier.push(*neighbour);
                }
            }
        }
        if component.is_disjoint(&driven) {
            remaining.insert(start);
        }
    }
    remaining
}

fn joint_motion_transform(kind: AssemblyJointKind) -> Transform {
    match kind {
        AssemblyJointKind::Fixed => Transform::identity(),
        AssemblyJointKind::Prismatic {
            axis, position_mm, ..
        } => {
            let direction = axis.direction_in_parent();
            Transform::from_translation(
                direction[0] * position_mm,
                direction[1] * position_mm,
                direction[2] * position_mm,
            )
            .expect("validated prismatic joint has a finite translation")
        }
        AssemblyJointKind::Revolute {
            axis,
            position_degrees,
            ..
        } => {
            let [x, y, z] = axis.direction_in_parent();
            let [px, py, pz] = axis.pivot_in_parent_mm();
            let angle = position_degrees.to_radians();
            let cosine = angle.cos();
            let sine = angle.sin();
            let complement = 1.0 - cosine;
            let rotation = [
                x * x * complement + cosine,
                x * y * complement - z * sine,
                x * z * complement + y * sine,
                y * x * complement + z * sine,
                y * y * complement + cosine,
                y * z * complement - x * sine,
                z * x * complement - y * sine,
                z * y * complement + x * sine,
                z * z * complement + cosine,
            ];
            let translation = [
                px - (rotation[0] * px + rotation[1] * py + rotation[2] * pz),
                py - (rotation[3] * px + rotation[4] * py + rotation[5] * pz),
                pz - (rotation[6] * px + rotation[7] * py + rotation[8] * pz),
            ];
            Transform::from_matrix([
                rotation[0],
                rotation[1],
                rotation[2],
                translation[0],
                rotation[3],
                rotation[4],
                rotation[5],
                translation[1],
                rotation[6],
                rotation[7],
                rotation[8],
                translation[2],
                0.0,
                0.0,
                0.0,
                1.0,
            ])
            .expect("validated revolute joint has a finite transform")
        }
    }
}

pub(crate) fn joint_motion_states_equal(left: AssemblyJointKind, right: AssemblyJointKind) -> bool {
    match (left, right) {
        (AssemblyJointKind::Fixed, AssemblyJointKind::Fixed) => true,
        (
            AssemblyJointKind::Revolute {
                axis: left_axis,
                position_degrees: left_position,
                ..
            },
            AssemblyJointKind::Revolute {
                axis: right_axis,
                position_degrees: right_position,
                ..
            },
        ) => left_axis == right_axis && left_position == right_position,
        (
            AssemblyJointKind::Prismatic {
                axis: left_axis,
                position_mm: left_position,
                ..
            },
            AssemblyJointKind::Prismatic {
                axis: right_axis,
                position_mm: right_position,
                ..
            },
        ) => left_axis == right_axis && left_position == right_position,
        _ => false,
    }
}

fn same_linear_transform(start: Transform, end: Transform) -> bool {
    const LINEAR_INDICES: [usize; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];
    LINEAR_INDICES.into_iter().all(|index| {
        let left = start.matrix()[index];
        let right = end.matrix()[index];
        (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
    })
}

fn continuous_translational_aabb_clearance(
    first_start: Aabb,
    first_end: Aabb,
    second_start: Aabb,
    second_end: Aabb,
    contact_tolerance_mm: f64,
) -> Result<(f64, f64, Option<f64>), AssemblyMotionClearanceError> {
    let mut breakpoints = vec![0.0, 1.0];
    let mut gaps = [[(0.0, 0.0); 2]; 3];
    for (axis, axis_gaps) in gaps.iter_mut().enumerate() {
        let first_before_second = (
            (second_end.min()[axis] - second_start.min()[axis])
                - (first_end.max()[axis] - first_start.max()[axis]),
            second_start.min()[axis] - first_start.max()[axis],
        );
        let second_before_first = (
            (first_end.min()[axis] - first_start.min()[axis])
                - (second_end.max()[axis] - second_start.max()[axis]),
            first_start.min()[axis] - second_start.max()[axis],
        );
        *axis_gaps = [first_before_second, second_before_first];
        for (slope, intercept) in [
            first_before_second,
            second_before_first,
            (
                first_before_second.0 - second_before_first.0,
                first_before_second.1 - second_before_first.1,
            ),
        ] {
            if slope != 0.0 {
                let root = -intercept / slope;
                if root.is_finite() && root > 0.0 && root < 1.0 {
                    breakpoints.push(root);
                }
            }
        }
    }
    breakpoints.sort_by(f64::total_cmp);
    breakpoints.dedup_by(|left, right| (*left - *right).abs() <= 1.0e-12);

    let mut minimum_squared = f64::INFINITY;
    let mut minimum_t = 0.0;
    let mut first_contact_t = None;
    let tolerance_squared = contact_tolerance_mm * contact_tolerance_mm;
    for segment in breakpoints.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let midpoint = (start + end) * 0.5;
        let mut quadratic = [0.0; 3];
        for axis_gaps in gaps {
            let left = axis_gaps[0].0 * midpoint + axis_gaps[0].1;
            let right = axis_gaps[1].0 * midpoint + axis_gaps[1].1;
            let (slope, intercept) = if left >= right && left > 0.0 {
                axis_gaps[0]
            } else if right > 0.0 {
                axis_gaps[1]
            } else {
                (0.0, 0.0)
            };
            quadratic[0] += slope * slope;
            quadratic[1] += 2.0 * slope * intercept;
            quadratic[2] += intercept * intercept;
        }
        let evaluate = |time: f64| quadratic[0] * time * time + quadratic[1] * time + quadratic[2];
        let candidate = if quadratic[0] > 0.0 {
            (-quadratic[1] / (2.0 * quadratic[0])).clamp(start, end)
        } else {
            start
        };
        for time in [start, candidate, end] {
            let squared = evaluate(time).max(0.0);
            if squared < minimum_squared {
                minimum_squared = squared;
                minimum_t = time;
            }
        }
        if first_contact_t.is_none() {
            if evaluate(start) <= tolerance_squared {
                first_contact_t = Some(start);
            } else if quadratic[0] > 0.0 {
                let adjusted_constant = quadratic[2] - tolerance_squared;
                let discriminant =
                    quadratic[1] * quadratic[1] - 4.0 * quadratic[0] * adjusted_constant;
                if discriminant >= 0.0 {
                    let root = (-quadratic[1] - discriminant.sqrt()) / (2.0 * quadratic[0]);
                    if root >= start && root <= end {
                        first_contact_t = Some(root);
                    }
                }
            }
        }
    }
    if !minimum_squared.is_finite() {
        return Err(AssemblyMotionClearanceError::NumericalFailure);
    }
    Ok((minimum_squared.sqrt(), minimum_t, first_contact_t))
}

fn transform_aabb(
    bounds: Aabb,
    transform: Transform,
) -> Result<Aabb, AssemblyMotionClearanceError> {
    let matrix = transform.matrix();
    let transformed = bounds.vertices().map(|point| {
        [
            matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
            matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
            matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
        ]
    });
    let min = std::array::from_fn(|axis| {
        transformed
            .iter()
            .map(|point| point[axis])
            .fold(f64::INFINITY, f64::min)
    });
    let max = std::array::from_fn(|axis| {
        transformed
            .iter()
            .map(|point| point[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    Aabb::new(min, max).map_err(|_| AssemblyMotionClearanceError::NumericalFailure)
}

fn union_aabb(left: Aabb, right: Aabb) -> Result<Aabb, AssemblyMotionClearanceError> {
    let min = std::array::from_fn(|axis| left.min()[axis].min(right.min()[axis]));
    let max = std::array::from_fn(|axis| left.max()[axis].max(right.max()[axis]));
    Aabb::new(min, max).map_err(|_| AssemblyMotionClearanceError::NumericalFailure)
}

fn aabb_clearance(left: Aabb, right: Aabb) -> Result<f64, AssemblyMotionClearanceError> {
    let squared = (0..3)
        .map(|axis| {
            let gap = (left.min()[axis] - right.max()[axis])
                .max(right.min()[axis] - left.max()[axis])
                .max(0.0);
            gap * gap
        })
        .sum::<f64>();
    if !squared.is_finite() {
        return Err(AssemblyMotionClearanceError::NumericalFailure);
    }
    Ok(squared.sqrt())
}

pub(crate) fn transforms_equivalent(left: Transform, right: Transform) -> bool {
    left.matrix()
        .iter()
        .zip(right.matrix())
        .all(|(left, right)| {
            let scale = left.abs().max(right.abs()).max(1.0);
            (left - right).abs() <= 1.0e-12 * scale
        })
}

fn invert_affine_transform(transform: Transform) -> Option<Transform> {
    let matrix = transform.matrix();
    let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
        - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
        + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
    let linear_scale = matrix[..12]
        .iter()
        .enumerate()
        .filter(|(index, _)| !matches!(index, 3 | 7 | 11))
        .map(|(_, value)| value.abs())
        .fold(0.0_f64, f64::max);
    if !determinant.is_finite()
        || linear_scale == 0.0
        || determinant.abs() <= f64::EPSILON * linear_scale.powi(3)
    {
        return None;
    }
    let inverse_determinant = determinant.recip();
    let inverse_linear = [
        (matrix[5] * matrix[10] - matrix[6] * matrix[9]) * inverse_determinant,
        (matrix[2] * matrix[9] - matrix[1] * matrix[10]) * inverse_determinant,
        (matrix[1] * matrix[6] - matrix[2] * matrix[5]) * inverse_determinant,
        (matrix[6] * matrix[8] - matrix[4] * matrix[10]) * inverse_determinant,
        (matrix[0] * matrix[10] - matrix[2] * matrix[8]) * inverse_determinant,
        (matrix[2] * matrix[4] - matrix[0] * matrix[6]) * inverse_determinant,
        (matrix[4] * matrix[9] - matrix[5] * matrix[8]) * inverse_determinant,
        (matrix[1] * matrix[8] - matrix[0] * matrix[9]) * inverse_determinant,
        (matrix[0] * matrix[5] - matrix[1] * matrix[4]) * inverse_determinant,
    ];
    let translation = [matrix[3], matrix[7], matrix[11]];
    let inverse_translation = [
        -(inverse_linear[0] * translation[0]
            + inverse_linear[1] * translation[1]
            + inverse_linear[2] * translation[2]),
        -(inverse_linear[3] * translation[0]
            + inverse_linear[4] * translation[1]
            + inverse_linear[5] * translation[2]),
        -(inverse_linear[6] * translation[0]
            + inverse_linear[7] * translation[1]
            + inverse_linear[8] * translation[2]),
    ];
    Transform::from_matrix([
        inverse_linear[0],
        inverse_linear[1],
        inverse_linear[2],
        inverse_translation[0],
        inverse_linear[3],
        inverse_linear[4],
        inverse_linear[5],
        inverse_translation[1],
        inverse_linear[6],
        inverse_linear[7],
        inverse_linear[8],
        inverse_translation[2],
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .ok()
}
