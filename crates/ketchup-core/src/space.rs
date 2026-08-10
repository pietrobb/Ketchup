use crate::document::{InstancePath, Snapshot};
use crate::exact_product::ExactResultRegistry;
use crate::exact_validation::{GeneralBodyParticipant, GeneralBodyValidationError};
use crate::graph::{DerivedIdentity, SlotResolution};
use crate::prismatic::{Aabb, PrismaticError, TolerancePolicy, collide_axis_aligned_prisms};
use crate::validation::EvidenceCounts;
use std::fmt;

const MAX_SEMANTIC_TEXT_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct SpaceId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalSpace {
    id: SpaceId,
    purpose: String,
    volume: Aabb,
    adjacent_to: Vec<SpaceId>,
    accessible_to: Vec<SpaceId>,
}

impl CanonicalSpace {
    pub fn new(
        id: SpaceId,
        purpose: impl Into<String>,
        volume: Aabb,
        mut adjacent_to: Vec<SpaceId>,
        mut accessible_to: Vec<SpaceId>,
    ) -> Result<Self, SpaceError> {
        ensure_id(id.0)?;
        let purpose = purpose.into();
        ensure_semantic_text(&purpose)?;
        if !volume.has_positive_volume() {
            return Err(SpaceError::InvalidVolume);
        }
        canonicalize_relations(id, &mut adjacent_to)?;
        canonicalize_relations(id, &mut accessible_to)?;
        Ok(Self {
            id,
            purpose,
            volume,
            adjacent_to,
            accessible_to,
        })
    }

    #[must_use]
    pub const fn id(&self) -> SpaceId {
        self.id
    }

    #[must_use]
    pub fn purpose(&self) -> &str {
        &self.purpose
    }

    #[must_use]
    pub const fn volume(&self) -> Aabb {
        self.volume
    }

    #[must_use]
    pub fn adjacent_to(&self) -> &[SpaceId] {
        &self.adjacent_to
    }

    #[must_use]
    pub fn accessible_to(&self) -> &[SpaceId] {
        &self.accessible_to
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClearanceOwner {
    Occurrence(InstancePath),
    Space(SpaceId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearanceSeverity {
    Advisory,
    Required,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearanceCoordinateFrame {
    World,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct ClearanceVolumeId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalClearanceVolume {
    id: ClearanceVolumeId,
    owner: ClearanceOwner,
    reason: String,
    volume: Aabb,
    coordinate_frame: ClearanceCoordinateFrame,
    tolerance: TolerancePolicy,
    severity: ClearanceSeverity,
    derived_from: Option<DerivedIdentity>,
}

impl CanonicalClearanceVolume {
    pub fn new(
        id: ClearanceVolumeId,
        owner: ClearanceOwner,
        reason: impl Into<String>,
        volume: Aabb,
        tolerance: TolerancePolicy,
        severity: ClearanceSeverity,
        derived_from: Option<DerivedIdentity>,
    ) -> Result<Self, SpaceError> {
        ensure_id(id.0)?;
        if matches!(&owner, ClearanceOwner::Space(id) if id.0 == 0) {
            return Err(SpaceError::ReservedId);
        }
        let reason = reason.into();
        ensure_semantic_text(&reason)?;
        if !volume.has_positive_volume() {
            return Err(SpaceError::InvalidVolume);
        }
        TolerancePolicy::new(tolerance.epsilon_mm()).map_err(SpaceError::Prismatic)?;
        Ok(Self {
            id,
            owner,
            reason,
            volume,
            coordinate_frame: ClearanceCoordinateFrame::World,
            tolerance,
            severity,
            derived_from,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ClearanceVolumeId {
        self.id
    }

    #[must_use]
    pub const fn owner(&self) -> &ClearanceOwner {
        &self.owner
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub const fn volume(&self) -> Aabb {
        self.volume
    }

    #[must_use]
    pub const fn coordinate_frame(&self) -> ClearanceCoordinateFrame {
        self.coordinate_frame
    }

    #[must_use]
    pub const fn tolerance(&self) -> TolerancePolicy {
        self.tolerance
    }

    #[must_use]
    pub const fn severity(&self) -> ClearanceSeverity {
        self.severity
    }

    #[must_use]
    pub const fn derived_from(&self) -> Option<&DerivedIdentity> {
        self.derived_from.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceOccupancyResult {
    pub clearance_volume_id: ClearanceVolumeId,
    pub occupants: Vec<InstancePath>,
    pub evidence_counts: EvidenceCounts,
}

impl ClearanceOccupancyResult {
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.occupants.is_empty()
    }
}

pub fn validate_clearance_occupancy(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
    clearance_volume_id: ClearanceVolumeId,
    occupancy_paths: impl IntoIterator<Item = InstancePath>,
) -> Result<ClearanceOccupancyResult, ClearanceValidationError> {
    let clearance = snapshot
        .clearance_volume(clearance_volume_id)
        .ok_or(ClearanceValidationError::ClearanceVolumeNotFound)?;
    match clearance.owner() {
        ClearanceOwner::Occurrence(path) => snapshot
            .resolve_instance_path(path)
            .map(|_| ())
            .map_err(|_| ClearanceValidationError::OwnerUnavailable)?,
        ClearanceOwner::Space(id) => {
            if snapshot.space(*id).is_none() {
                return Err(ClearanceValidationError::OwnerUnavailable);
            }
        }
    }
    if let Some(identity) = clearance.derived_from()
        && snapshot.resolve_slot(identity) != SlotResolution::Resolved
    {
        return Err(ClearanceValidationError::UnresolvedDerivedIdentity);
    }

    let mut paths = occupancy_paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut occupants = Vec::new();
    let mut evidence_counts = EvidenceCounts::default();
    for path in paths {
        if matches!(clearance.owner(), ClearanceOwner::Occurrence(owner) if owner == &path) {
            continue;
        }
        let participant =
            GeneralBodyParticipant::accept(snapshot, registry, path.clone(), clearance.tolerance())
                .map_err(ClearanceValidationError::Geometry)?;
        evidence_counts.record(participant.evidence_class());
        let intersection = collide_axis_aligned_prisms(
            participant.bounds(),
            clearance.volume(),
            clearance.tolerance(),
        )
        .map_err(ClearanceValidationError::Prismatic)?
        .physical_intersection;
        if intersection.is_some_and(|value| value.has_positive_volume()) {
            occupants.push(path);
        }
    }
    Ok(ClearanceOccupancyResult {
        clearance_volume_id,
        occupants,
        evidence_counts,
    })
}

fn ensure_id(id: u64) -> Result<(), SpaceError> {
    if id == 0 {
        Err(SpaceError::ReservedId)
    } else {
        Ok(())
    }
}

fn ensure_semantic_text(value: &str) -> Result<(), SpaceError> {
    if value.trim().is_empty() || value.len() > MAX_SEMANTIC_TEXT_BYTES {
        Err(SpaceError::InvalidSemanticText)
    } else {
        Ok(())
    }
}

fn canonicalize_relations(id: SpaceId, relations: &mut Vec<SpaceId>) -> Result<(), SpaceError> {
    if relations
        .iter()
        .any(|target| target.0 == 0 || *target == id)
    {
        return Err(SpaceError::InvalidRelation);
    }
    relations.sort_unstable();
    relations.dedup();
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpaceError {
    ReservedId,
    InvalidSemanticText,
    InvalidVolume,
    InvalidRelation,
    AsymmetricAdjacency,
    MissingSpace,
    InvalidOwner,
    Prismatic(PrismaticError),
}

impl fmt::Display for SpaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReservedId => "space or clearance-volume ID zero is reserved",
            Self::InvalidSemanticText => "space purpose or clearance reason is empty or too large",
            Self::InvalidVolume => "space or clearance bounds must have finite positive volume",
            Self::InvalidRelation => "space adjacency/access relation is invalid",
            Self::AsymmetricAdjacency => "space adjacency must be declared symmetrically",
            Self::MissingSpace => "space adjacency/access target does not exist",
            Self::InvalidOwner => "clearance owner does not resolve in the canonical document",
            Self::Prismatic(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for SpaceError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClearanceValidationError {
    ClearanceVolumeNotFound,
    OwnerUnavailable,
    UnresolvedDerivedIdentity,
    Geometry(GeneralBodyValidationError),
    Prismatic(PrismaticError),
}

impl fmt::Display for ClearanceValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ClearanceVolumeNotFound => "clearance volume does not exist",
            Self::OwnerUnavailable => "clearance owner is unavailable",
            Self::UnresolvedDerivedIdentity => {
                "rule-derived clearance identity is lost or ambiguous"
            }
            Self::Geometry(_) => "clearance occupancy geometry is unavailable, stale, or invalid",
            Self::Prismatic(_) => "clearance occupancy intersection could not be certified",
        })
    }
}

impl std::error::Error for ClearanceValidationError {}
