use crate::document::{
    CanonicalCommand, CommandBatch, DocumentId, DocumentStore, OccurrenceId, Proposal,
    ProposalPrepareError, Snapshot, Transform,
};
use crate::exact_product::{BodySubshapeRef, ExactReferenceResolution, ExactResultRegistry};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const ASSEMBLY_MATE_SCHEMA_V1: &str = "ketchup.assembly-mate.v1";
pub const RIGID_BODY_DEGREES_OF_FREEDOM: u8 = 6;
const MAX_ASSEMBLY_DISTANCE_MM: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct AssemblyMateId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyReferenceHealth {
    Resolved,
    Broken,
    Ambiguous { candidate_count: u32 },
    Lost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyDofStatus {
    Grounded,
    PendingSolve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssemblyDofDiagnostic {
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) status: AssemblyDofStatus,
    pub(crate) remaining_dof: Option<u8>,
    pub(crate) incident_mate_ids: Vec<AssemblyMateId>,
}

impl AssemblyDofDiagnostic {
    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn status(&self) -> AssemblyDofStatus {
        self.status
    }

    #[must_use]
    pub const fn remaining_dof(&self) -> Option<u8> {
        self.remaining_dof
    }

    #[must_use]
    pub fn incident_mate_ids(&self) -> &[AssemblyMateId] {
        &self.incident_mate_ids
    }
}

#[derive(Clone, Debug)]
pub struct PlanarFaceAttachment {
    reference: BodySubshapeRef,
    local_origin_mm: [f64; 3],
    local_unit_normal: [f64; 3],
}

impl PartialEq for PlanarFaceAttachment {
    fn eq(&self, other: &Self) -> bool {
        self.reference == other.reference
            && self
                .local_origin_mm
                .into_iter()
                .chain(self.local_unit_normal)
                .zip(
                    other
                        .local_origin_mm
                        .into_iter()
                        .chain(other.local_unit_normal),
                )
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }
}

impl Eq for PlanarFaceAttachment {}

impl PlanarFaceAttachment {
    #[must_use]
    pub fn new(
        reference: BodySubshapeRef,
        local_origin_mm: [f64; 3],
        local_unit_normal: [f64; 3],
    ) -> Option<Self> {
        let normal_length_squared = dot(local_unit_normal, local_unit_normal);
        (reference.has_valid_lineage()
            && reference.expected_type == "planar_face"
            && local_origin_mm.into_iter().all(f64::is_finite)
            && local_unit_normal.into_iter().all(f64::is_finite)
            && (normal_length_squared - 1.0).abs() <= 1.0e-12)
            .then_some(Self {
                reference,
                local_origin_mm,
                local_unit_normal,
            })
    }

    #[must_use]
    pub const fn reference(&self) -> &BodySubshapeRef {
        &self.reference
    }

    #[must_use]
    pub const fn local_origin_mm(&self) -> [f64; 3] {
        self.local_origin_mm
    }

    #[must_use]
    pub const fn local_unit_normal(&self) -> [f64; 3] {
        self.local_unit_normal
    }

    #[must_use]
    pub fn has_valid_geometry(&self) -> bool {
        Self::new(
            self.reference.clone(),
            self.local_origin_mm,
            self.local_unit_normal,
        )
        .is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyMateAttachment {
    ReferenceOnly(BodySubshapeRef),
    PlanarFace(PlanarFaceAttachment),
}

impl AssemblyMateAttachment {
    #[must_use]
    pub const fn reference(&self) -> &BodySubshapeRef {
        match self {
            Self::ReferenceOnly(reference) => reference,
            Self::PlanarFace(attachment) => attachment.reference(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMateEndpoint {
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) attachment: AssemblyMateAttachment,
    pub(crate) health: AssemblyReferenceHealth,
}

impl AssemblyMateEndpoint {
    #[must_use]
    pub fn resolved(occurrence_id: OccurrenceId, reference: BodySubshapeRef) -> Self {
        Self {
            occurrence_id,
            attachment: AssemblyMateAttachment::ReferenceOnly(reference),
            health: AssemblyReferenceHealth::Resolved,
        }
    }

    #[must_use]
    pub fn resolved_planar_face(
        occurrence_id: OccurrenceId,
        attachment: PlanarFaceAttachment,
    ) -> Self {
        Self {
            occurrence_id,
            attachment: AssemblyMateAttachment::PlanarFace(attachment),
            health: AssemblyReferenceHealth::Resolved,
        }
    }

    #[must_use]
    pub fn broken(occurrence_id: OccurrenceId, reference: BodySubshapeRef) -> Self {
        Self {
            occurrence_id,
            attachment: AssemblyMateAttachment::ReferenceOnly(reference),
            health: AssemblyReferenceHealth::Broken,
        }
    }

    #[must_use]
    pub fn ambiguous(
        occurrence_id: OccurrenceId,
        reference: BodySubshapeRef,
        candidate_count: u32,
    ) -> Self {
        Self {
            occurrence_id,
            attachment: AssemblyMateAttachment::ReferenceOnly(reference),
            health: AssemblyReferenceHealth::Ambiguous { candidate_count },
        }
    }

    #[must_use]
    pub fn lost(occurrence_id: OccurrenceId, reference: BodySubshapeRef) -> Self {
        Self {
            occurrence_id,
            attachment: AssemblyMateAttachment::ReferenceOnly(reference),
            health: AssemblyReferenceHealth::Lost,
        }
    }

    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn attachment(&self) -> &AssemblyMateAttachment {
        &self.attachment
    }

    #[must_use]
    pub const fn reference(&self) -> &BodySubshapeRef {
        self.attachment.reference()
    }

    #[must_use]
    pub const fn planar_face_attachment(&self) -> Option<&PlanarFaceAttachment> {
        match &self.attachment {
            AssemblyMateAttachment::PlanarFace(attachment) => Some(attachment),
            AssemblyMateAttachment::ReferenceOnly(_) => None,
        }
    }

    #[must_use]
    pub const fn health(&self) -> AssemblyReferenceHealth {
        self.health
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssemblyMateKind {
    CoincidentPlanar { offset_mm: f64, reversed: bool },
    ConcentricAxial { reversed: bool },
    Distance { distance_mm: f64 },
    Angle { angle_degrees: f64 },
}

impl AssemblyMateKind {
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::CoincidentPlanar { offset_mm, .. } => {
                offset_mm.is_finite() && offset_mm.abs() <= MAX_ASSEMBLY_DISTANCE_MM
            }
            Self::ConcentricAxial { .. } => true,
            Self::Distance { distance_mm } => {
                distance_mm.is_finite() && (0.0..=MAX_ASSEMBLY_DISTANCE_MM).contains(&distance_mm)
            }
            Self::Angle { angle_degrees } => {
                angle_degrees.is_finite() && (0.0..=180.0).contains(&angle_degrees)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMate {
    pub(crate) schema: String,
    pub(crate) id: AssemblyMateId,
    pub(crate) endpoint_a: Box<AssemblyMateEndpoint>,
    pub(crate) endpoint_b: Box<AssemblyMateEndpoint>,
    pub(crate) kind: AssemblyMateKind,
}

impl AssemblyMate {
    #[must_use]
    pub fn new(
        id: AssemblyMateId,
        endpoint_a: AssemblyMateEndpoint,
        endpoint_b: AssemblyMateEndpoint,
        kind: AssemblyMateKind,
    ) -> Self {
        Self {
            schema: ASSEMBLY_MATE_SCHEMA_V1.to_owned(),
            id,
            endpoint_a: Box::new(endpoint_a),
            endpoint_b: Box::new(endpoint_b),
            kind,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> AssemblyMateId {
        self.id
    }

    #[must_use]
    pub const fn endpoint_a(&self) -> &AssemblyMateEndpoint {
        &self.endpoint_a
    }

    #[must_use]
    pub const fn endpoint_b(&self) -> &AssemblyMateEndpoint {
        &self.endpoint_b
    }

    #[must_use]
    pub const fn kind(&self) -> AssemblyMateKind {
        self.kind
    }
}

pub const ASSEMBLY_SOLVER_SCHEMA_V1: &str = "ketchup.rigid-assembly-solver.v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssemblySolverPolicy {
    pub max_iterations: u16,
    pub linear_tolerance_mm: f64,
    pub angular_tolerance_radians: f64,
    pub finite_difference_step: f64,
    pub damping: f64,
}

impl Default for AssemblySolverPolicy {
    fn default() -> Self {
        Self {
            max_iterations: 32,
            linear_tolerance_mm: 1.0e-7,
            angular_tolerance_radians: 1.0e-9,
            finite_difference_step: 1.0e-6,
            damping: 1.0e-12,
        }
    }
}

impl AssemblySolverPolicy {
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.max_iterations > 0
            && self.max_iterations <= 256
            && self.linear_tolerance_mm.is_finite()
            && self.linear_tolerance_mm > 0.0
            && self.angular_tolerance_radians.is_finite()
            && self.angular_tolerance_radians > 0.0
            && self.finite_difference_step.is_finite()
            && self.finite_difference_step > 0.0
            && self.damping.is_finite()
            && self.damping > 0.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblySolveStatus {
    UnderConstrained,
    FullyConstrained,
    OverConstrained,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblySolvedOccurrence {
    occurrence_id: OccurrenceId,
    transform: Transform,
    remaining_dof: u8,
    grounded: bool,
}

impl AssemblySolvedOccurrence {
    #[must_use]
    pub const fn occurrence_id(&self) -> OccurrenceId {
        self.occurrence_id
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn remaining_dof(&self) -> u8 {
        self.remaining_dof
    }

    #[must_use]
    pub const fn grounded(&self) -> bool {
        self.grounded
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblySolveResult {
    schema: String,
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    policy: AssemblySolverPolicy,
    status: AssemblySolveStatus,
    iterations: u16,
    remaining_dof: usize,
    maximum_residual: f64,
    occurrences: Vec<AssemblySolvedOccurrence>,
    redundant_mate_ids: Vec<AssemblyMateId>,
    conflicting_mate_ids: Vec<AssemblyMateId>,
}

impl AssemblySolveResult {
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn policy(&self) -> AssemblySolverPolicy {
        self.policy
    }

    #[must_use]
    pub const fn status(&self) -> AssemblySolveStatus {
        self.status
    }

    #[must_use]
    pub const fn iterations(&self) -> u16 {
        self.iterations
    }

    #[must_use]
    pub const fn remaining_dof(&self) -> usize {
        self.remaining_dof
    }

    #[must_use]
    pub const fn maximum_residual(&self) -> f64 {
        self.maximum_residual
    }

    #[must_use]
    pub fn occurrences(&self) -> &[AssemblySolvedOccurrence] {
        &self.occurrences
    }

    #[must_use]
    pub fn occurrence(&self, id: OccurrenceId) -> Option<&AssemblySolvedOccurrence> {
        self.occurrences
            .binary_search_by_key(&id, AssemblySolvedOccurrence::occurrence_id)
            .ok()
            .map(|index| &self.occurrences[index])
    }

    #[must_use]
    pub fn redundant_mate_ids(&self) -> &[AssemblyMateId] {
        &self.redundant_mate_ids
    }

    #[must_use]
    pub fn conflicting_mate_ids(&self) -> &[AssemblyMateId] {
        &self.conflicting_mate_ids
    }

    pub fn publication_batch(
        &self,
        current: &Snapshot,
    ) -> Result<CommandBatch, AssemblySolvePublishError> {
        if current.document_id() != self.document_id
            || current.revision_id() != self.source_revision
            || current.canonical_digest() != self.source_digest
        {
            return Err(AssemblySolvePublishError::Stale);
        }
        if !matches!(
            self.status,
            AssemblySolveStatus::UnderConstrained | AssemblySolveStatus::FullyConstrained
        ) {
            return Err(AssemblySolvePublishError::SolveNotConverged(self.status));
        }
        let commands = self
            .occurrences
            .iter()
            .filter(|solved| !solved.grounded)
            .filter_map(|solved| {
                current
                    .occurrence(solved.occurrence_id)
                    .filter(|occurrence| occurrence.transform() != solved.transform)
                    .map(|_| CanonicalCommand::SetOccurrenceTransform {
                        id: solved.occurrence_id,
                        transform: solved.transform,
                    })
            })
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return Err(AssemblySolvePublishError::NoTransformChanges);
        }
        Ok(CommandBatch::new(vec![
            CanonicalCommand::ApplyAssemblySolve {
                source_revision: self.source_revision,
                source_digest: self.source_digest.clone(),
                transforms: commands
                    .into_iter()
                    .map(|command| match command {
                        CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                            (id, transform)
                        }
                        _ => {
                            unreachable!("assembly publication only contains occurrence transforms")
                        }
                    })
                    .collect(),
            },
        ]))
    }

    pub fn prepare_publication(
        &self,
        document: &DocumentStore,
    ) -> Result<Proposal, AssemblySolvePublishError> {
        let current = document.current();
        document
            .prepare_proposal(self.publication_batch(&current)?)
            .map_err(AssemblySolvePublishError::ProposalPreparation)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssemblySolveError {
    InvalidPolicy,
    InvalidRigidTransform(OccurrenceId),
    UnresolvedReference(AssemblyMateId),
    UnsupportedReference(AssemblyMateId),
    NumericalFailure,
}

impl fmt::Display for AssemblySolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy => formatter.write_str("assembly solver policy is invalid"),
            Self::InvalidRigidTransform(id) => {
                write!(
                    formatter,
                    "occurrence {} does not have a rigid transform",
                    id.0
                )
            }
            Self::UnresolvedReference(id) => {
                write!(
                    formatter,
                    "assembly mate {} has an unresolved reference",
                    id.0
                )
            }
            Self::UnsupportedReference(id) => {
                write!(
                    formatter,
                    "assembly mate {} uses an unsupported reference frame",
                    id.0
                )
            }
            Self::NumericalFailure => formatter.write_str("assembly solver failed numerically"),
        }
    }
}

impl std::error::Error for AssemblySolveError {}

#[derive(Debug, PartialEq)]
pub enum AssemblySolvePublishError {
    Stale,
    SolveNotConverged(AssemblySolveStatus),
    NoTransformChanges,
    ProposalPreparation(ProposalPrepareError),
}

impl fmt::Display for AssemblySolvePublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("assembly solve result is stale"),
            Self::SolveNotConverged(status) => {
                write!(
                    formatter,
                    "assembly solve result is not publishable: {status:?}"
                )
            }
            Self::NoTransformChanges => {
                formatter.write_str("assembly solve produced no canonical transform changes")
            }
            Self::ProposalPreparation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblySolvePublishError {}

pub const ASSEMBLY_RECOMPUTE_SCHEMA_V1: &str = "ketchup.rigid-assembly-recompute.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyRecomputeStatus {
    Solved,
    Broken,
    Ambiguous,
    Lost,
    OverConstrained,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyRecomputeResult {
    schema: String,
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    status: AssemblyRecomputeStatus,
    mates: Vec<AssemblyMate>,
    transforms: Vec<(OccurrenceId, Transform)>,
    solve: Option<AssemblySolveResult>,
}

impl AssemblyRecomputeResult {
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn status(&self) -> AssemblyRecomputeStatus {
        self.status
    }

    #[must_use]
    pub fn mates(&self) -> &[AssemblyMate] {
        &self.mates
    }

    #[must_use]
    pub fn solve(&self) -> Option<&AssemblySolveResult> {
        self.solve.as_ref()
    }

    pub fn publication_batch(
        &self,
        current: &Snapshot,
    ) -> Result<CommandBatch, AssemblyRecomputePublishError> {
        if current.document_id() != self.document_id
            || current.revision_id() != self.source_revision
            || current.canonical_digest() != self.source_digest
        {
            return Err(AssemblyRecomputePublishError::Stale);
        }
        if matches!(
            self.status,
            AssemblyRecomputeStatus::OverConstrained | AssemblyRecomputeStatus::Failed
        ) {
            return Err(AssemblyRecomputePublishError::SolveNotPublishable(
                self.status,
            ));
        }
        let mut commands = self
            .mates
            .iter()
            .filter(|mate| current.assembly_mate(mate.id()) != Some(*mate))
            .cloned()
            .map(CanonicalCommand::RebindAssemblyMate)
            .collect::<Vec<_>>();
        if commands.is_empty() && self.transforms.is_empty() {
            return Err(AssemblyRecomputePublishError::NoCanonicalChanges);
        }
        commands.insert(
            0,
            CanonicalCommand::GuardAssemblyRecompute {
                source_revision: self.source_revision,
                source_digest: self.source_digest.clone(),
            },
        );
        if !self.transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: self.source_revision,
                source_digest: self.source_digest.clone(),
                transforms: self.transforms.clone(),
            });
        }
        Ok(CommandBatch::new(commands))
    }

    pub fn prepare_publication(
        &self,
        document: &DocumentStore,
    ) -> Result<Proposal, AssemblyRecomputePublishError> {
        let current = document.current();
        document
            .prepare_proposal(self.publication_batch(&current)?)
            .map_err(AssemblyRecomputePublishError::ProposalPreparation)
    }
}

#[derive(Debug, PartialEq)]
pub enum AssemblyRecomputeError {
    Preview(crate::document::CanonicalError),
    Solve(AssemblySolveError),
}

impl fmt::Display for AssemblyRecomputeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preview(error) => error.fmt(formatter),
            Self::Solve(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblyRecomputeError {}

#[derive(Debug, PartialEq)]
pub enum AssemblyRecomputePublishError {
    Stale,
    SolveNotPublishable(AssemblyRecomputeStatus),
    NoCanonicalChanges,
    ProposalPreparation(ProposalPrepareError),
}

impl fmt::Display for AssemblyRecomputePublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("assembly recompute result is stale"),
            Self::SolveNotPublishable(status) => {
                write!(
                    formatter,
                    "assembly recompute is not publishable: {status:?}"
                )
            }
            Self::NoCanonicalChanges => {
                formatter.write_str("assembly recompute produced no canonical changes")
            }
            Self::ProposalPreparation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssemblyRecomputePublishError {}

pub fn recompute_rigid_assembly(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    policy: AssemblySolverPolicy,
) -> Result<AssemblyRecomputeResult, AssemblyRecomputeError> {
    recompute_rigid_assembly_from_snapshot(&document.current(), exact_results, policy)
}

pub(crate) fn recompute_rigid_assembly_from_snapshot(
    source: &Snapshot,
    exact_results: &ExactResultRegistry,
    policy: AssemblySolverPolicy,
) -> Result<AssemblyRecomputeResult, AssemblyRecomputeError> {
    recompute_rigid_assembly_selection(source, exact_results, policy, None)
}

pub(crate) fn recompute_rigid_assembly_mates_from_snapshot(
    source: &Snapshot,
    exact_results: &ExactResultRegistry,
    policy: AssemblySolverPolicy,
    mate_ids: &BTreeSet<AssemblyMateId>,
) -> Result<AssemblyRecomputeResult, AssemblyRecomputeError> {
    recompute_rigid_assembly_selection(source, exact_results, policy, Some(mate_ids))
}

fn recompute_rigid_assembly_selection(
    source: &Snapshot,
    exact_results: &ExactResultRegistry,
    policy: AssemblySolverPolicy,
    mate_ids: Option<&BTreeSet<AssemblyMateId>>,
) -> Result<AssemblyRecomputeResult, AssemblyRecomputeError> {
    let mut status = AssemblyRecomputeStatus::Solved;
    let mates = source
        .assembly_mates()
        .filter(|mate| mate_ids.is_none_or(|ids| ids.contains(&mate.id())))
        .map(|mate| {
            let mut endpoint = |value: &AssemblyMateEndpoint| match exact_results
                .resolve_reference(source, value.reference())
            {
                ExactReferenceResolution::Resolved { reference } => match mate.kind() {
                    AssemblyMateKind::ConcentricAxial { .. } => {
                        AssemblyMateEndpoint::resolved(value.occurrence_id(), *reference)
                    }
                    AssemblyMateKind::CoincidentPlanar { .. }
                    | AssemblyMateKind::Distance { .. }
                    | AssemblyMateKind::Angle { .. } => exact_results
                        .planar_face_attachment(source, &reference)
                        .cloned()
                        .map(|attachment| {
                            AssemblyMateEndpoint::resolved_planar_face(
                                value.occurrence_id(),
                                attachment,
                            )
                        })
                        .unwrap_or_else(|| {
                            status = AssemblyRecomputeStatus::Broken;
                            AssemblyMateEndpoint::broken(value.occurrence_id(), *reference)
                        }),
                },
                ExactReferenceResolution::Ambiguous { candidate_count } => {
                    if status != AssemblyRecomputeStatus::Broken {
                        status = AssemblyRecomputeStatus::Ambiguous;
                    }
                    AssemblyMateEndpoint::ambiguous(
                        value.occurrence_id(),
                        value.reference().clone(),
                        u32::try_from(candidate_count).unwrap_or(u32::MAX),
                    )
                }
                ExactReferenceResolution::Lost => {
                    if matches!(
                        status,
                        AssemblyRecomputeStatus::Solved | AssemblyRecomputeStatus::Lost
                    ) {
                        status = AssemblyRecomputeStatus::Lost;
                    }
                    AssemblyMateEndpoint::lost(value.occurrence_id(), value.reference().clone())
                }
                ExactReferenceResolution::Quarantined { .. } => {
                    status = AssemblyRecomputeStatus::Broken;
                    AssemblyMateEndpoint::broken(value.occurrence_id(), value.reference().clone())
                }
            };
            AssemblyMate::new(
                mate.id(),
                endpoint(mate.endpoint_a()),
                endpoint(mate.endpoint_b()),
                mate.kind(),
            )
        })
        .collect::<Vec<_>>();

    let mut transforms = Vec::new();
    let mut solve = None;
    if status == AssemblyRecomputeStatus::Solved {
        let mut solve_commands = mate_ids.map_or_else(Vec::new, |ids| {
            source
                .assembly_mates()
                .filter(|mate| !ids.contains(&mate.id()))
                .map(|mate| CanonicalCommand::DeleteAssemblyMate { id: mate.id() })
                .collect()
        });
        solve_commands.extend(
            mates
                .iter()
                .filter(|mate| source.assembly_mate(mate.id()) != Some(*mate))
                .cloned()
                .map(CanonicalCommand::RebindAssemblyMate),
        );
        let solve_source = if solve_commands.is_empty() {
            source.clone()
        } else {
            source
                .preview_batch(&CommandBatch::new(solve_commands))
                .map_err(AssemblyRecomputeError::Preview)?
        };
        let solved =
            solve_rigid_assembly(&solve_source, policy).map_err(AssemblyRecomputeError::Solve)?;
        status = match solved.status() {
            AssemblySolveStatus::UnderConstrained | AssemblySolveStatus::FullyConstrained => {
                AssemblyRecomputeStatus::Solved
            }
            AssemblySolveStatus::OverConstrained => AssemblyRecomputeStatus::OverConstrained,
            AssemblySolveStatus::Failed => AssemblyRecomputeStatus::Failed,
        };
        if status == AssemblyRecomputeStatus::Solved {
            transforms = solved
                .occurrences()
                .iter()
                .filter(|occurrence| !occurrence.grounded())
                .filter_map(|occurrence| {
                    source
                        .occurrence(occurrence.occurrence_id())
                        .filter(|current| current.transform() != occurrence.transform())
                        .map(|_| (occurrence.occurrence_id(), occurrence.transform()))
                })
                .collect();
        }
        solve = Some(solved);
    }

    Ok(AssemblyRecomputeResult {
        schema: ASSEMBLY_RECOMPUTE_SCHEMA_V1.to_owned(),
        document_id: source.document_id(),
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        status,
        mates,
        transforms,
        solve,
    })
}

#[derive(Clone, Copy)]
struct RigidPose {
    rotation: [[f64; 3]; 3],
    translation: [f64; 3],
}

impl RigidPose {
    fn from_transform(transform: Transform) -> Option<Self> {
        let matrix = transform.matrix();
        let rotation = [
            [matrix[0], matrix[1], matrix[2]],
            [matrix[4], matrix[5], matrix[6]],
            [matrix[8], matrix[9], matrix[10]],
        ];
        let pose = Self {
            rotation,
            translation: [matrix[3], matrix[7], matrix[11]],
        };
        let rows_are_unit = rotation
            .iter()
            .all(|row| (dot(*row, *row) - 1.0).abs() <= 1.0e-8);
        let rows_are_orthogonal = dot(rotation[0], rotation[1]).abs() <= 1.0e-8
            && dot(rotation[0], rotation[2]).abs() <= 1.0e-8
            && dot(rotation[1], rotation[2]).abs() <= 1.0e-8;
        let determinant = dot(rotation[0], cross(rotation[1], rotation[2]));
        (rows_are_unit && rows_are_orthogonal && (determinant - 1.0).abs() <= 1.0e-8)
            .then_some(pose)
    }

    fn to_transform(self) -> Result<Transform, AssemblySolveError> {
        Transform::from_matrix([
            self.rotation[0][0],
            self.rotation[0][1],
            self.rotation[0][2],
            self.translation[0],
            self.rotation[1][0],
            self.rotation[1][1],
            self.rotation[1][2],
            self.translation[1],
            self.rotation[2][0],
            self.rotation[2][1],
            self.rotation[2][2],
            self.translation[2],
            0.0,
            0.0,
            0.0,
            1.0,
        ])
        .map_err(|_| AssemblySolveError::NumericalFailure)
    }

    fn rotate(self, vector: [f64; 3]) -> [f64; 3] {
        [
            dot(self.rotation[0], vector),
            dot(self.rotation[1], vector),
            dot(self.rotation[2], vector),
        ]
    }

    fn transform_point(self, point: [f64; 3]) -> [f64; 3] {
        add(self.rotate(point), self.translation)
    }

    fn compose(self, local: Self) -> Self {
        Self {
            rotation: multiply_rotation(self.rotation, local.rotation),
            translation: add(self.rotate(local.translation), self.translation),
        }
    }

    fn inverse(self) -> Self {
        let rotation = transpose(self.rotation);
        Self {
            rotation,
            translation: scale(multiply_vector(rotation, self.translation), -1.0),
        }
    }

    fn perturb(&mut self, axis: usize, value: f64) {
        if axis < 3 {
            self.translation[axis] += value;
            return;
        }
        let mut rotation_vector = [0.0; 3];
        rotation_vector[axis - 3] = value;
        self.rotation = multiply_rotation(rodrigues(rotation_vector), self.rotation);
    }
}

#[derive(Clone)]
struct SolverState {
    local_poses: BTreeMap<OccurrenceId, RigidPose>,
    parent_poses: BTreeMap<OccurrenceId, RigidPose>,
}

impl SolverState {
    fn world_pose(&self, id: OccurrenceId) -> RigidPose {
        self.parent_poses[&id].compose(self.local_poses[&id])
    }
}

pub fn solve_rigid_assembly(
    snapshot: &Snapshot,
    policy: AssemblySolverPolicy,
) -> Result<AssemblySolveResult, AssemblySolveError> {
    if !policy.is_valid() {
        return Err(AssemblySolveError::InvalidPolicy);
    }
    let mut state = SolverState {
        local_poses: BTreeMap::new(),
        parent_poses: BTreeMap::new(),
    };
    for occurrence in snapshot.occurrences() {
        let local = RigidPose::from_transform(occurrence.transform())
            .ok_or(AssemblySolveError::InvalidRigidTransform(occurrence.id()))?;
        let world = RigidPose::from_transform(
            snapshot
                .world_transform_for_occurrence(occurrence.id())
                .ok_or(AssemblySolveError::InvalidRigidTransform(occurrence.id()))?,
        )
        .ok_or(AssemblySolveError::InvalidRigidTransform(occurrence.id()))?;
        state.local_poses.insert(occurrence.id(), local);
        state
            .parent_poses
            .insert(occurrence.id(), world.compose(local.inverse()));
    }
    let mut mates = snapshot.assembly_mates().collect::<Vec<_>>();
    mates.sort_by_key(|mate| mate_sort_key(mate));
    for mate in &mates {
        if mate.endpoint_a().health() != AssemblyReferenceHealth::Resolved
            || mate.endpoint_b().health() != AssemblyReferenceHealth::Resolved
        {
            return Err(AssemblySolveError::UnresolvedReference(mate.id()));
        }
        endpoint_local_frame(mate.endpoint_a(), mate.kind())
            .zip(endpoint_local_frame(mate.endpoint_b(), mate.kind()))
            .ok_or(AssemblySolveError::UnsupportedReference(mate.id()))?;
    }
    let variables = snapshot
        .occurrences()
        .filter(|occurrence| !snapshot.occurrence_is_grounded(occurrence.id()))
        .map(|occurrence| occurrence.id())
        .collect::<Vec<_>>();

    let mut numerical_failure = false;
    for _ in 0..policy.max_iterations {
        let residual = residuals(&state, &mates)?;
        let jacobian = numerical_jacobian(&state, &mates, &variables, &residual, policy)?;
        let Some(update) = least_squares_step(&jacobian, &residual, policy.damping) else {
            numerical_failure = true;
            break;
        };
        for (variable_index, occurrence_id) in variables.iter().enumerate() {
            let pose = state
                .local_poses
                .get_mut(occurrence_id)
                .expect("solver variable is a canonical occurrence");
            for axis in 0..6 {
                let limit = if axis < 3 { 100.0 } else { 0.25 };
                pose.perturb(axis, update[variable_index * 6 + axis].clamp(-limit, limit));
            }
        }
    }

    let final_residual = residuals(&state, &mates)?;
    let final_jacobian = numerical_jacobian(&state, &mates, &variables, &final_residual, policy)?;
    let rank = matrix_rank(&final_jacobian, 1.0e-8);
    let raw_remaining_dof = variables.len() * usize::from(RIGID_BODY_DEGREES_OF_FREEDOM) - rank;
    let maximum_residual = final_residual
        .iter()
        .fold(0.0_f64, |value, residual| value.max(residual.abs()));
    let conflicting_mate_ids = conflicting_mates(&state, &mates, policy)?;
    let redundant_mate_ids = redundant_mates(&state, &mates, &variables, policy)?;
    let rotationally_symmetric_occurrences = variables
        .iter()
        .enumerate()
        .filter_map(|(variable_index, occurrence_id)| {
            let local_columns = (0..6)
                .map(|axis| variable_index * 6 + axis)
                .collect::<Vec<_>>();
            let local_jacobian = select_columns(&final_jacobian, &local_columns);
            let local_remaining =
                usize::from(RIGID_BODY_DEGREES_OF_FREEDOM) - matrix_rank(&local_jacobian, 1.0e-8);
            (local_remaining == 1
                && occurrence_has_bounded_axial_symmetry(
                    *occurrence_id,
                    &mates,
                    &redundant_mate_ids,
                ))
            .then_some(*occurrence_id)
        })
        .collect::<BTreeSet<_>>();
    let remaining_dof = raw_remaining_dof.saturating_sub(rotationally_symmetric_occurrences.len());
    let status = if numerical_failure || !maximum_residual.is_finite() {
        AssemblySolveStatus::Failed
    } else if !conflicting_mate_ids.is_empty() {
        AssemblySolveStatus::OverConstrained
    } else if remaining_dof == 0 {
        AssemblySolveStatus::FullyConstrained
    } else {
        AssemblySolveStatus::UnderConstrained
    };

    let mut occurrences = Vec::new();
    for occurrence in snapshot.occurrences() {
        let grounded = snapshot.occurrence_is_grounded(occurrence.id());
        let remaining = if grounded {
            0
        } else {
            let Some(variable_index) = variables.iter().position(|id| *id == occurrence.id())
            else {
                return Err(AssemblySolveError::NumericalFailure);
            };
            let local_columns = (0..6)
                .map(|axis| variable_index * 6 + axis)
                .collect::<Vec<_>>();
            let local_jacobian = select_columns(&final_jacobian, &local_columns);
            let raw_remaining = (usize::from(RIGID_BODY_DEGREES_OF_FREEDOM)
                - matrix_rank(&local_jacobian, 1.0e-8)) as u8;
            raw_remaining.saturating_sub(u8::from(
                rotationally_symmetric_occurrences.contains(&occurrence.id()),
            ))
        };
        occurrences.push(AssemblySolvedOccurrence {
            occurrence_id: occurrence.id(),
            transform: state.local_poses[&occurrence.id()].to_transform()?,
            remaining_dof: remaining,
            grounded,
        });
    }

    Ok(AssemblySolveResult {
        schema: ASSEMBLY_SOLVER_SCHEMA_V1.to_owned(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        policy,
        status,
        iterations: policy.max_iterations,
        remaining_dof,
        maximum_residual,
        occurrences,
        redundant_mate_ids,
        conflicting_mate_ids,
    })
}

fn mate_sort_key(mate: &AssemblyMate) -> (u64, u64, u8, u64, u64, String, String) {
    let a = mate.endpoint_a().occurrence_id().0;
    let b = mate.endpoint_b().occurrence_id().0;
    let (kind, first, second) = match mate.kind() {
        AssemblyMateKind::CoincidentPlanar {
            offset_mm,
            reversed,
        } => (0, offset_mm.to_bits(), u64::from(reversed)),
        AssemblyMateKind::ConcentricAxial { reversed } => (1, u64::from(reversed), 0),
        AssemblyMateKind::Distance { distance_mm } => (2, distance_mm.to_bits(), 0),
        AssemblyMateKind::Angle { angle_degrees } => (3, angle_degrees.to_bits(), 0),
    };
    (
        a.min(b),
        a.max(b),
        kind,
        first,
        second,
        mate.endpoint_a().reference().lineage_digest.clone(),
        mate.endpoint_b().reference().lineage_digest.clone(),
    )
}

// Compatibility-only fallback for schema <= 49 endpoints and axial references that do not yet
// carry an authoritative typed frame. Typed planar attachments never consult semantic-role text.
fn legacy_reference_axis_from_semantic_role(reference: &BodySubshapeRef) -> Option<[f64; 3]> {
    let role = reference.semantic_role.as_str();
    if role.contains("west") {
        Some([-1.0, 0.0, 0.0])
    } else if role.contains("east") {
        Some([1.0, 0.0, 0.0])
    } else if role.contains("south") {
        Some([0.0, -1.0, 0.0])
    } else if role.contains("north") {
        Some([0.0, 1.0, 0.0])
    } else if role.contains("bottom") || role.contains("floor") || role.ends_with("start") {
        Some([0.0, 0.0, -1.0])
    } else if role.contains("top")
        || role.contains("rim")
        || role.ends_with("end")
        || reference.expected_type.ends_with("_face")
        || reference.expected_type.ends_with("_edge")
        || matches!(reference.expected_type.as_str(), "face" | "edge")
    {
        Some([0.0, 0.0, 1.0])
    } else {
        None
    }
}

fn endpoint_local_frame(
    endpoint: &AssemblyMateEndpoint,
    kind: AssemblyMateKind,
) -> Option<([f64; 3], [f64; 3])> {
    if matches!(kind, AssemblyMateKind::ConcentricAxial { .. }) {
        return match endpoint.attachment() {
            AssemblyMateAttachment::ReferenceOnly(reference) => {
                legacy_reference_axis_from_semantic_role(reference).map(|axis| ([0.0; 3], axis))
            }
            AssemblyMateAttachment::PlanarFace(_) => None,
        };
    }
    match endpoint.attachment() {
        AssemblyMateAttachment::PlanarFace(attachment) => attachment
            .has_valid_geometry()
            .then(|| (attachment.local_origin_mm(), attachment.local_unit_normal())),
        AssemblyMateAttachment::ReferenceOnly(_) => None,
    }
}

fn endpoint_world_frame(
    pose: RigidPose,
    endpoint: &AssemblyMateEndpoint,
    kind: AssemblyMateKind,
) -> Option<([f64; 3], [f64; 3])> {
    let (local_origin, local_axis) = endpoint_local_frame(endpoint, kind)?;
    Some((
        pose.transform_point(local_origin),
        normalize(pose.rotate(local_axis))?,
    ))
}

fn residuals(state: &SolverState, mates: &[&AssemblyMate]) -> Result<Vec<f64>, AssemblySolveError> {
    let mut result = Vec::new();
    for mate in mates {
        result.extend(mate_residual(state, mate)?);
    }
    Ok(result)
}

fn mate_residual(state: &SolverState, mate: &AssemblyMate) -> Result<Vec<f64>, AssemblySolveError> {
    let a_pose = state.world_pose(mate.endpoint_a().occurrence_id());
    let b_pose = state.world_pose(mate.endpoint_b().occurrence_id());
    let (a_origin, a_axis) = endpoint_world_frame(a_pose, mate.endpoint_a(), mate.kind())
        .ok_or(AssemblySolveError::UnsupportedReference(mate.id()))?;
    let (b_origin, b_axis) = endpoint_world_frame(b_pose, mate.endpoint_b(), mate.kind())
        .ok_or(AssemblySolveError::UnsupportedReference(mate.id()))?;
    let delta = subtract(b_origin, a_origin);
    Ok(match mate.kind() {
        AssemblyMateKind::CoincidentPlanar {
            offset_mm,
            reversed,
        } => {
            let desired_b = scale(a_axis, if reversed { 1.0 } else { -1.0 });
            let orientation = subtract(b_axis, desired_b);
            vec![
                orientation[0],
                orientation[1],
                orientation[2],
                dot(delta, a_axis) - offset_mm,
            ]
        }
        AssemblyMateKind::ConcentricAxial { reversed } => {
            let desired_b = scale(a_axis, if reversed { -1.0 } else { 1.0 });
            let orientation = subtract(b_axis, desired_b);
            let perpendicular = subtract(delta, scale(a_axis, dot(delta, a_axis)));
            vec![
                orientation[0],
                orientation[1],
                orientation[2],
                perpendicular[0],
                perpendicular[1],
                perpendicular[2],
            ]
        }
        AssemblyMateKind::Distance { distance_mm } => vec![norm(delta) - distance_mm],
        AssemblyMateKind::Angle { angle_degrees } => {
            vec![dot(a_axis, b_axis) - angle_degrees.to_radians().cos()]
        }
    })
}

fn numerical_jacobian(
    state: &SolverState,
    mates: &[&AssemblyMate],
    variables: &[OccurrenceId],
    baseline: &[f64],
    policy: AssemblySolverPolicy,
) -> Result<Vec<Vec<f64>>, AssemblySolveError> {
    let column_count = variables.len() * 6;
    let mut jacobian = vec![vec![0.0; column_count]; baseline.len()];
    for (variable_index, occurrence_id) in variables.iter().enumerate() {
        for axis in 0..6 {
            let mut positive = state.clone();
            positive
                .local_poses
                .get_mut(occurrence_id)
                .expect("solver variable is a canonical occurrence")
                .perturb(axis, policy.finite_difference_step);
            let positive_values = residuals(&positive, mates)?;
            let mut negative = state.clone();
            negative
                .local_poses
                .get_mut(occurrence_id)
                .expect("solver variable is a canonical occurrence")
                .perturb(axis, -policy.finite_difference_step);
            let negative_values = residuals(&negative, mates)?;
            for (row, (positive, negative)) in
                positive_values.iter().zip(&negative_values).enumerate()
            {
                jacobian[row][variable_index * 6 + axis] =
                    (positive - negative) / (2.0 * policy.finite_difference_step);
            }
        }
    }
    Ok(jacobian)
}

fn least_squares_step(jacobian: &[Vec<f64>], residual: &[f64], damping: f64) -> Option<Vec<f64>> {
    let columns = jacobian.first().map_or(0, Vec::len);
    if columns == 0 {
        return Some(Vec::new());
    }
    let mut normal = vec![vec![0.0; columns]; columns];
    let mut right = vec![0.0; columns];
    for row in 0..jacobian.len() {
        for column in 0..columns {
            right[column] -= jacobian[row][column] * residual[row];
            for other in 0..columns {
                normal[column][other] += jacobian[row][column] * jacobian[row][other];
            }
        }
    }
    for (index, row) in normal.iter_mut().enumerate() {
        row[index] += damping;
    }
    solve_linear_system(normal, right)
}

fn solve_linear_system(mut matrix: Vec<Vec<f64>>, mut right: Vec<f64>) -> Option<Vec<f64>> {
    let size = right.len();
    for column in 0..size {
        let pivot = (column..size).max_by(|left, right_index| {
            matrix[*left][column]
                .abs()
                .total_cmp(&matrix[*right_index][column].abs())
                .then_with(|| right_index.cmp(left))
        })?;
        if matrix[pivot][column].abs() <= f64::EPSILON {
            return None;
        }
        matrix.swap(column, pivot);
        right.swap(column, pivot);
        let divisor = matrix[column][column];
        for value in &mut matrix[column][column..] {
            *value /= divisor;
        }
        right[column] /= divisor;
        for row in 0..size {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            let pivot_values = matrix[column][column..].to_vec();
            for (value, pivot_value) in matrix[row][column..].iter_mut().zip(pivot_values) {
                *value -= factor * pivot_value;
            }
            right[row] -= factor * right[column];
        }
    }
    right.iter().all(|value| value.is_finite()).then_some(right)
}

fn matrix_rank(matrix: &[Vec<f64>], tolerance: f64) -> usize {
    if matrix.is_empty() || matrix[0].is_empty() {
        return 0;
    }
    let mut reduced = matrix.to_vec();
    let rows = reduced.len();
    let columns = reduced[0].len();
    let mut rank = 0;
    for column in 0..columns {
        let Some(pivot) = (rank..rows).max_by(|left, right| {
            reduced[*left][column]
                .abs()
                .total_cmp(&reduced[*right][column].abs())
                .then_with(|| right.cmp(left))
        }) else {
            break;
        };
        if reduced[pivot][column].abs() <= tolerance {
            continue;
        }
        reduced.swap(rank, pivot);
        let divisor = reduced[rank][column];
        for value in &mut reduced[rank][column..] {
            *value /= divisor;
        }
        for row in 0..rows {
            if row == rank {
                continue;
            }
            let factor = reduced[row][column];
            let pivot_values = reduced[rank][column..].to_vec();
            for (value, pivot_value) in reduced[row][column..].iter_mut().zip(pivot_values) {
                *value -= factor * pivot_value;
            }
        }
        rank += 1;
        if rank == rows {
            break;
        }
    }
    rank
}

fn select_columns(matrix: &[Vec<f64>], columns: &[usize]) -> Vec<Vec<f64>> {
    matrix
        .iter()
        .map(|row| columns.iter().map(|column| row[*column]).collect())
        .collect()
}

fn occurrence_has_bounded_axial_symmetry(
    occurrence_id: OccurrenceId,
    mates: &[&AssemblyMate],
    redundant_mate_ids: &[AssemblyMateId],
) -> bool {
    let mut has_planar_seat = false;
    let mut has_cylindrical_axis = false;
    for mate in mates {
        if redundant_mate_ids.contains(&mate.id()) {
            continue;
        }
        let reference = if mate.endpoint_a().occurrence_id() == occurrence_id {
            Some(mate.endpoint_a().reference())
        } else if mate.endpoint_b().occurrence_id() == occurrence_id {
            Some(mate.endpoint_b().reference())
        } else {
            None
        };
        let Some(reference) = reference else {
            continue;
        };
        match mate.kind() {
            AssemblyMateKind::CoincidentPlanar { .. } => has_planar_seat = true,
            AssemblyMateKind::ConcentricAxial { .. }
                if reference.expected_type == "cylindrical_face" =>
            {
                has_cylindrical_axis = true;
            }
            _ => {}
        }
    }
    has_planar_seat && has_cylindrical_axis
}

fn redundant_mates(
    state: &SolverState,
    mates: &[&AssemblyMate],
    variables: &[OccurrenceId],
    policy: AssemblySolverPolicy,
) -> Result<Vec<AssemblyMateId>, AssemblySolveError> {
    let mut redundant = Vec::new();
    let mut accepted = Vec::new();
    let mut rank = 0;
    for mate in mates {
        let mut candidate = accepted.clone();
        candidate.push(*mate);
        let baseline = residuals(state, &candidate)?;
        let jacobian = numerical_jacobian(state, &candidate, variables, &baseline, policy)?;
        let candidate_rank = matrix_rank(&jacobian, 1.0e-8);
        if candidate_rank == rank {
            redundant.push(mate.id());
        } else {
            accepted.push(*mate);
            rank = candidate_rank;
        }
    }
    redundant.sort_unstable();
    Ok(redundant)
}

fn conflicting_mates(
    state: &SolverState,
    mates: &[&AssemblyMate],
    policy: AssemblySolverPolicy,
) -> Result<Vec<AssemblyMateId>, AssemblySolveError> {
    let mut conflicts = Vec::new();
    for mate in mates {
        let residual = mate_residual(state, mate)?;
        let satisfied = match mate.kind() {
            AssemblyMateKind::CoincidentPlanar { .. } => {
                residual[..3]
                    .iter()
                    .all(|value| value.abs() <= policy.angular_tolerance_radians)
                    && residual[3].abs() <= policy.linear_tolerance_mm
            }
            AssemblyMateKind::ConcentricAxial { .. } => {
                residual[..3]
                    .iter()
                    .all(|value| value.abs() <= policy.angular_tolerance_radians)
                    && residual[3..]
                        .iter()
                        .all(|value| value.abs() <= policy.linear_tolerance_mm)
            }
            AssemblyMateKind::Distance { .. } => residual[0].abs() <= policy.linear_tolerance_mm,
            AssemblyMateKind::Angle { .. } => residual[0].abs() <= policy.angular_tolerance_radians,
        };
        if !satisfied {
            conflicts.push(mate.id());
        }
    }
    conflicts.sort_unstable();
    Ok(conflicts)
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale(vector: [f64; 3], factor: f64) -> [f64; 3] {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

fn norm(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}

fn normalize(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = norm(vector);
    (length.is_finite() && length > f64::EPSILON).then(|| scale(vector, length.recip()))
}

fn transpose(matrix: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [
        [matrix[0][0], matrix[1][0], matrix[2][0]],
        [matrix[0][1], matrix[1][1], matrix[2][1]],
        [matrix[0][2], matrix[1][2], matrix[2][2]],
    ]
}

fn multiply_vector(matrix: [[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    [
        dot(matrix[0], vector),
        dot(matrix[1], vector),
        dot(matrix[2], vector),
    ]
}

fn multiply_rotation(left: [[f64; 3]; 3], right: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let right = transpose(right);
    [
        [
            dot(left[0], right[0]),
            dot(left[0], right[1]),
            dot(left[0], right[2]),
        ],
        [
            dot(left[1], right[0]),
            dot(left[1], right[1]),
            dot(left[1], right[2]),
        ],
        [
            dot(left[2], right[0]),
            dot(left[2], right[1]),
            dot(left[2], right[2]),
        ],
    ]
}

fn rodrigues(rotation_vector: [f64; 3]) -> [[f64; 3]; 3] {
    let angle = norm(rotation_vector);
    if angle <= f64::EPSILON {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let axis = scale(rotation_vector, angle.recip());
    let cosine = angle.cos();
    let sine = angle.sin();
    let one_minus_cosine = 1.0 - cosine;
    [
        [
            cosine + axis[0] * axis[0] * one_minus_cosine,
            axis[0] * axis[1] * one_minus_cosine - axis[2] * sine,
            axis[0] * axis[2] * one_minus_cosine + axis[1] * sine,
        ],
        [
            axis[1] * axis[0] * one_minus_cosine + axis[2] * sine,
            cosine + axis[1] * axis[1] * one_minus_cosine,
            axis[1] * axis[2] * one_minus_cosine - axis[0] * sine,
        ],
        [
            axis[2] * axis[0] * one_minus_cosine - axis[1] * sine,
            axis[2] * axis[1] * one_minus_cosine + axis[0] * sine,
            cosine + axis[2] * axis[2] * one_minus_cosine,
        ],
    ]
}
