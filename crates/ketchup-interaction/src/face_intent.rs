#![forbid(unsafe_code)]

use crate::spatial::SnapshotBinding;
use ketchup_core::document::{BodyId, DefinitionId, Snapshot};
use ketchup_core::exact_product::{BodySubshapeRef, ExactFeatureChainRequest};
use ketchup_core::sketch::{PrincipalPlane, WorkplaneFrame, WorkplaneSupportHealth};
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum FaceWorkplaneContext {
    Datum(PrincipalPlane),
    PlanarFace {
        reference: Box<BodySubshapeRef>,
        health: WorkplaneSupportHealth,
        frame: WorkplaneFrame,
    },
}

impl FaceWorkplaneContext {
    #[must_use]
    pub fn frame(&self) -> WorkplaneFrame {
        match self {
            Self::Datum(plane) => WorkplaneFrame::principal(*plane),
            Self::PlanarFace { frame, .. } => *frame,
        }
    }

    #[must_use]
    pub fn reference(&self) -> Option<&BodySubshapeRef> {
        match self {
            Self::Datum(_) => None,
            Self::PlanarFace { reference, .. } => Some(reference),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceIntentTarget {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub workplane: FaceWorkplaneContext,
}

impl FaceIntentTarget {
    #[must_use]
    pub const fn datum(
        definition_id: DefinitionId,
        body_id: BodyId,
        plane: PrincipalPlane,
    ) -> Self {
        Self {
            definition_id,
            body_id,
            workplane: FaceWorkplaneContext::Datum(plane),
        }
    }

    #[must_use]
    pub fn planar_face(
        definition_id: DefinitionId,
        body_id: BodyId,
        reference: BodySubshapeRef,
        health: WorkplaneSupportHealth,
        frame: WorkplaneFrame,
    ) -> Self {
        Self {
            definition_id,
            body_id,
            workplane: FaceWorkplaneContext::PlanarFace {
                reference: Box::new(reference),
                health,
                frame,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoverFaceCandidate {
    pub target: FaceIntentTarget,
    pub ray_distance_mm: f64,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaceIntentSource {
    Hover { pick_through_index: usize },
    StableSelection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFaceIntent {
    pub source: FaceIntentSource,
    pub target: FaceIntentTarget,
}

#[derive(Clone, Debug)]
pub struct TransientFaceIntent {
    binding: SnapshotBinding,
    definition_id: DefinitionId,
    ordered_hover: Vec<HoverFaceCandidate>,
    stable_selection: Option<FaceIntentTarget>,
}

impl TransientFaceIntent {
    pub fn new(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        mut hover: Vec<HoverFaceCandidate>,
        stable_selection: Option<FaceIntentTarget>,
    ) -> Result<Self, FaceIntentError> {
        if hover.iter().any(|candidate| {
            !candidate.ray_distance_mm.is_finite() || candidate.ray_distance_mm < 0.0
        }) {
            return Err(FaceIntentError::InvalidRayDistance);
        }
        hover.sort_by(compare_hover_candidates);
        Ok(Self {
            binding: SnapshotBinding::from_snapshot(snapshot),
            definition_id,
            ordered_hover: hover,
            stable_selection,
        })
    }

    #[must_use]
    pub fn ordered_hover_candidates(&self) -> &[HoverFaceCandidate] {
        &self.ordered_hover
    }

    pub fn resolve(
        &self,
        snapshot: &Snapshot,
        pick_through_index: usize,
    ) -> Result<ResolvedFaceIntent, FaceIntentError> {
        if !self.binding.is_current(snapshot) {
            return Err(FaceIntentError::StaleSnapshot);
        }
        let (source, target, visible) = if self.ordered_hover.is_empty() {
            (
                FaceIntentSource::StableSelection,
                self.stable_selection
                    .as_ref()
                    .ok_or(FaceIntentError::NoTarget)?,
                true,
            )
        } else {
            let index = pick_through_index % self.ordered_hover.len();
            let candidate = &self.ordered_hover[index];
            (
                FaceIntentSource::Hover {
                    pick_through_index: index,
                },
                &candidate.target,
                candidate.visible,
            )
        };
        self.validate_target(snapshot, target, visible)?;
        Ok(ResolvedFaceIntent {
            source,
            target: target.clone(),
        })
    }

    fn validate_target(
        &self,
        snapshot: &Snapshot,
        target: &FaceIntentTarget,
        candidate_visible: bool,
    ) -> Result<(), FaceIntentError> {
        if target.definition_id != self.definition_id {
            return Err(FaceIntentError::CrossContext);
        }
        let definition = snapshot
            .definition(target.definition_id)
            .ok_or(FaceIntentError::DefinitionNotFound(target.definition_id))?;
        let body = definition
            .body(target.body_id)
            .ok_or(FaceIntentError::BodyNotFound(target.body_id))?;
        if !candidate_visible || !body.visible() {
            return Err(FaceIntentError::HiddenTarget);
        }
        if body.consumed_by().is_some() {
            return Err(FaceIntentError::ConsumedBody(target.body_id));
        }
        target
            .workplane
            .frame()
            .validate()
            .map_err(|_| FaceIntentError::InvalidWorkplane)?;
        let FaceWorkplaneContext::PlanarFace {
            reference, health, ..
        } = &target.workplane
        else {
            return Ok(());
        };
        if *health != WorkplaneSupportHealth::Resolved {
            return Err(FaceIntentError::UnresolvedReference(*health));
        }
        if reference.document_id != snapshot.document_id()
            || reference.definition_id != target.definition_id
        {
            return Err(FaceIntentError::CrossContext);
        }
        if reference.expected_type != "planar_face" || !reference.has_valid_lineage() {
            return Err(FaceIntentError::InvalidReference);
        }
        let request = ExactFeatureChainRequest::from_snapshot_for_producer(
            snapshot,
            target.definition_id,
            reference.producer_feature_id,
        )
        .map_err(|_| FaceIntentError::ReferenceUnavailable)?;
        if !reference.matches_request(&request) {
            return Err(FaceIntentError::StaleReference);
        }
        let current = snapshot
            .exact_reference_by_lineage(&reference.lineage_digest)
            .ok_or(FaceIntentError::ReferenceUnavailable)?;
        if current != reference.as_ref() {
            return Err(FaceIntentError::StaleReference);
        }
        if definition
            .feature_body_ownership(reference.producer_feature_id)
            .and_then(|ownership| ownership.output_body_id())
            != Some(target.body_id)
        {
            return Err(FaceIntentError::ReferenceBodyMismatch);
        }
        Ok(())
    }
}

fn compare_hover_candidates(left: &HoverFaceCandidate, right: &HoverFaceCandidate) -> Ordering {
    left.ray_distance_mm
        .total_cmp(&right.ray_distance_mm)
        .then_with(|| left.target.definition_id.cmp(&right.target.definition_id))
        .then_with(|| left.target.body_id.cmp(&right.target.body_id))
        .then_with(|| compare_workplanes(&left.target.workplane, &right.target.workplane))
        .then_with(|| left.visible.cmp(&right.visible))
}

fn compare_workplanes(left: &FaceWorkplaneContext, right: &FaceWorkplaneContext) -> Ordering {
    match (left, right) {
        (FaceWorkplaneContext::Datum(left), FaceWorkplaneContext::Datum(right)) => {
            principal_plane_rank(*left).cmp(&principal_plane_rank(*right))
        }
        (FaceWorkplaneContext::Datum(_), FaceWorkplaneContext::PlanarFace { .. }) => Ordering::Less,
        (FaceWorkplaneContext::PlanarFace { .. }, FaceWorkplaneContext::Datum(_)) => {
            Ordering::Greater
        }
        (
            FaceWorkplaneContext::PlanarFace {
                reference: left, ..
            },
            FaceWorkplaneContext::PlanarFace {
                reference: right, ..
            },
        ) => left.lineage_digest.cmp(&right.lineage_digest),
    }
}

const fn principal_plane_rank(plane: PrincipalPlane) -> u8 {
    match plane {
        PrincipalPlane::Xy => 0,
        PrincipalPlane::Yz => 1,
        PrincipalPlane::Xz => 2,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaceIntentError {
    NoTarget,
    InvalidRayDistance,
    StaleSnapshot,
    CrossContext,
    DefinitionNotFound(DefinitionId),
    BodyNotFound(BodyId),
    HiddenTarget,
    ConsumedBody(BodyId),
    InvalidWorkplane,
    UnresolvedReference(WorkplaneSupportHealth),
    InvalidReference,
    ReferenceUnavailable,
    StaleReference,
    ReferenceBodyMismatch,
}

impl fmt::Display for FaceIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTarget => formatter.write_str("no hovered or stably selected face target"),
            Self::InvalidRayDistance => {
                formatter.write_str("hover distance must be finite and nonnegative")
            }
            Self::StaleSnapshot => {
                formatter.write_str("face intent is bound to a stale canonical snapshot")
            }
            Self::CrossContext => formatter
                .write_str("face target belongs to a different document or definition context"),
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} does not exist", id.0),
            Self::BodyNotFound(id) => write!(formatter, "body {} does not exist", id.0),
            Self::HiddenTarget => formatter.write_str("hidden face targets are not actionable"),
            Self::ConsumedBody(id) => write!(formatter, "body {} has been consumed", id.0),
            Self::InvalidWorkplane => {
                formatter.write_str("face target has an invalid workplane frame")
            }
            Self::UnresolvedReference(health) => {
                write!(formatter, "face reference is not resolved: {health:?}")
            }
            Self::InvalidReference => {
                formatter.write_str("face target does not carry one valid planar-face lineage")
            }
            Self::ReferenceUnavailable => {
                formatter.write_str("face reference has no current exact evidence")
            }
            Self::StaleReference => {
                formatter.write_str("face reference evidence no longer matches the target")
            }
            Self::ReferenceBodyMismatch => {
                formatter.write_str("face producer does not own the targeted body")
            }
        }
    }
}

impl std::error::Error for FaceIntentError {}
