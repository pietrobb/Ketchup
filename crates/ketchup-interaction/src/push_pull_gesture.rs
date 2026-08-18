#![forbid(unsafe_code)]

use crate::face_intent::{
    FaceIntentError, FaceWorkplaneContext, ResolvedFaceIntent, TransientFaceIntent,
};
use crate::spatial::SnapshotBinding;
use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, Dimension, DocumentStore, FeatureId, FeatureKind,
    Proposal, ProposalContext, ProposalPrepareError, Snapshot,
};
use ketchup_core::exact_product::{BodySubshapeRef, ExactFaceRole};
use std::collections::BTreeSet;
use std::fmt;

const MIN_EXTENT_MM: f64 = 0.01;
const MAX_SNAP_CANDIDATES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PushPullSnapKind {
    Endpoint,
    Midpoint,
    Edge,
    Face,
    Coplanar,
    Grid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PushPullSnapSettings {
    enabled: BTreeSet<PushPullSnapKind>,
    grid_step_mm: f64,
    tolerance_mm: f64,
}

impl PushPullSnapSettings {
    pub fn new(grid_step_mm: f64, tolerance_mm: f64) -> Result<Self, PushPullGestureError> {
        if !grid_step_mm.is_finite()
            || grid_step_mm <= 0.0
            || !tolerance_mm.is_finite()
            || tolerance_mm < 0.0
        {
            return Err(PushPullGestureError::InvalidSnapSettings);
        }
        Ok(Self {
            enabled: BTreeSet::from([
                PushPullSnapKind::Endpoint,
                PushPullSnapKind::Midpoint,
                PushPullSnapKind::Edge,
                PushPullSnapKind::Face,
                PushPullSnapKind::Coplanar,
                PushPullSnapKind::Grid,
            ]),
            grid_step_mm,
            tolerance_mm,
        })
    }

    pub fn set_enabled(&mut self, kind: PushPullSnapKind, enabled: bool) {
        if enabled {
            self.enabled.insert(kind);
        } else {
            self.enabled.remove(&kind);
        }
    }

    #[must_use]
    pub fn is_enabled(&self, kind: PushPullSnapKind) -> bool {
        self.enabled.contains(&kind)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PushPullSnapCandidate {
    kind: PushPullSnapKind,
    signed_distance_mm: f64,
    stable_key: String,
}

impl PushPullSnapCandidate {
    pub fn new(
        kind: PushPullSnapKind,
        signed_distance_mm: f64,
        stable_key: impl Into<String>,
    ) -> Result<Self, PushPullGestureError> {
        let stable_key = stable_key.into();
        if kind == PushPullSnapKind::Grid
            || !signed_distance_mm.is_finite()
            || stable_key.trim().is_empty()
        {
            return Err(PushPullGestureError::InvalidSnapCandidate);
        }
        Ok(Self {
            kind,
            signed_distance_mm,
            stable_key,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> PushPullSnapKind {
        self.kind
    }

    #[must_use]
    pub const fn signed_distance_mm(&self) -> f64 {
        self.signed_distance_mm
    }

    #[must_use]
    pub fn stable_key(&self) -> &str {
        &self.stable_key
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PushPullSnapFeedback {
    kind: PushPullSnapKind,
    raw_signed_distance_mm: f64,
    snapped_signed_distance_mm: f64,
    stable_key: String,
}

impl PushPullSnapFeedback {
    #[must_use]
    pub const fn kind(&self) -> PushPullSnapKind {
        self.kind
    }

    #[must_use]
    pub const fn raw_signed_distance_mm(&self) -> f64 {
        self.raw_signed_distance_mm
    }

    #[must_use]
    pub const fn snapped_signed_distance_mm(&self) -> f64 {
        self.snapped_signed_distance_mm
    }

    #[must_use]
    pub fn stable_key(&self) -> &str {
        &self.stable_key
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushPullPreviewSource {
    Pointer,
    Exact,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PushPullPreview {
    target: ResolvedFaceIntent,
    target_reference: BodySubshapeRef,
    producer_feature_id: FeatureId,
    source: PushPullPreviewSource,
    signed_distance: Dimension,
    resulting_extent: Dimension,
    snap_feedback: Option<PushPullSnapFeedback>,
}

impl PushPullPreview {
    #[must_use]
    pub const fn target(&self) -> &ResolvedFaceIntent {
        &self.target
    }

    #[must_use]
    pub const fn target_reference(&self) -> &BodySubshapeRef {
        &self.target_reference
    }

    #[must_use]
    pub const fn producer_feature_id(&self) -> FeatureId {
        self.producer_feature_id
    }

    #[must_use]
    pub const fn source(&self) -> PushPullPreviewSource {
        self.source
    }

    #[must_use]
    pub const fn signed_distance(&self) -> &Dimension {
        &self.signed_distance
    }

    #[must_use]
    pub const fn resulting_extent(&self) -> &Dimension {
        &self.resulting_extent
    }

    #[must_use]
    pub const fn snap_feedback(&self) -> Option<&PushPullSnapFeedback> {
        self.snap_feedback.as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct SmartPushPullGesture {
    binding: SnapshotBinding,
    target: ResolvedFaceIntent,
    target_reference: BodySubshapeRef,
    producer_feature_id: FeatureId,
    original_extent_mm: f64,
}

impl SmartPushPullGesture {
    pub fn begin(
        snapshot: &Snapshot,
        face_intent: &TransientFaceIntent,
        pick_through_index: usize,
    ) -> Result<Self, PushPullGestureError> {
        let target = face_intent
            .resolve(snapshot, pick_through_index)
            .map_err(PushPullGestureError::FaceIntent)?;
        let FaceWorkplaneContext::PlanarFace { reference, .. } = &target.target.workplane else {
            return Err(PushPullGestureError::UnsupportedFace);
        };
        if reference.role() != Some(ExactFaceRole::Top) {
            return Err(PushPullGestureError::UnsupportedFace);
        }
        let target_reference = reference.as_ref().clone();
        let producer_feature_id = target_reference.producer_feature_id;
        let feature = snapshot
            .feature(producer_feature_id)
            .ok_or(PushPullGestureError::ProducerNotFound(producer_feature_id))?;
        let FeatureKind::Extrusion { profile, height } = feature.kind() else {
            return Err(PushPullGestureError::UnsupportedFace);
        };
        if feature.definition_id() != target.target.definition_id
            || *profile != target_reference.profile_feature_id
            || height.millimetres() <= MIN_EXTENT_MM
        {
            return Err(PushPullGestureError::UnsupportedFace);
        }
        let definition = snapshot
            .definition(target.target.definition_id)
            .ok_or(PushPullGestureError::UnsupportedFace)?;
        if definition
            .feature_body_ownership(producer_feature_id)
            .and_then(|ownership| ownership.output_body_id())
            != Some(target.target.body_id)
        {
            return Err(PushPullGestureError::TargetBodyMismatch);
        }
        Ok(Self {
            binding: SnapshotBinding::from_snapshot(snapshot),
            target,
            target_reference,
            producer_feature_id,
            original_extent_mm: height.millimetres(),
        })
    }

    #[must_use]
    pub const fn body_id(&self) -> BodyId {
        self.target.target.body_id
    }

    #[must_use]
    pub const fn producer_feature_id(&self) -> FeatureId {
        self.producer_feature_id
    }

    #[must_use]
    pub const fn target_reference(&self) -> &BodySubshapeRef {
        &self.target_reference
    }

    pub fn preview_pointer(
        &self,
        snapshot: &Snapshot,
        raw_signed_distance_mm: f64,
        settings: &PushPullSnapSettings,
        candidates: &[PushPullSnapCandidate],
    ) -> Result<PushPullPreview, PushPullGestureError> {
        self.ensure_current(snapshot)?;
        if !raw_signed_distance_mm.is_finite() {
            return Err(PushPullGestureError::InvalidDistance);
        }
        if candidates.len() > MAX_SNAP_CANDIDATES {
            return Err(PushPullGestureError::TooManySnapCandidates);
        }
        let feedback = resolve_snap(
            raw_signed_distance_mm,
            self.original_extent_mm,
            settings,
            candidates,
        );
        let signed_distance_mm = feedback.as_ref().map_or(raw_signed_distance_mm, |snap| {
            snap.snapped_signed_distance_mm
        });
        let signed_distance =
            Dimension::new(canonical_number(signed_distance_mm), signed_distance_mm)
                .map_err(|_| PushPullGestureError::InvalidDistance)?;
        self.preview(PushPullPreviewSource::Pointer, signed_distance, feedback)
    }

    pub fn preview_exact(
        &self,
        snapshot: &Snapshot,
        signed_distance: Dimension,
    ) -> Result<PushPullPreview, PushPullGestureError> {
        self.ensure_current(snapshot)?;
        self.preview(PushPullPreviewSource::Exact, signed_distance, None)
    }

    pub fn plan_proposal(
        &self,
        document: &DocumentStore,
        preview: &PushPullPreview,
    ) -> Result<Proposal, PushPullGestureError> {
        let snapshot = document.current();
        self.ensure_current(&snapshot)?;
        if preview.target != self.target
            || preview.target_reference != self.target_reference
            || preview.producer_feature_id != self.producer_feature_id
        {
            return Err(PushPullGestureError::ForeignPreview);
        }
        if preview.signed_distance.millimetres().abs() < MIN_EXTENT_MM {
            return Err(PushPullGestureError::NoChange);
        }
        let definition_id = self.target.target.definition_id;
        let body_id = self.target.target.body_id;
        let definition = snapshot
            .definition(definition_id)
            .ok_or(PushPullGestureError::TargetBodyMismatch)?;
        if definition
            .feature_body_ownership(self.producer_feature_id)
            .and_then(|ownership| ownership.output_body_id())
            != Some(body_id)
        {
            return Err(PushPullGestureError::TargetBodyMismatch);
        }
        document
            .prepare_proposal_with_context(
                CommandBatch::new(vec![
                    CanonicalCommand::SetActiveBody {
                        definition_id,
                        id: body_id,
                    },
                    CanonicalCommand::SetFeatureDimension {
                        id: self.producer_feature_id,
                        dimension: preview.resulting_extent.clone(),
                    },
                ]),
                ProposalContext::canonical_preview(),
            )
            .map_err(PushPullGestureError::Proposal)
    }

    fn preview(
        &self,
        source: PushPullPreviewSource,
        signed_distance: Dimension,
        snap_feedback: Option<PushPullSnapFeedback>,
    ) -> Result<PushPullPreview, PushPullGestureError> {
        let resulting_extent_mm = self.original_extent_mm + signed_distance.millimetres();
        if !resulting_extent_mm.is_finite() || resulting_extent_mm <= MIN_EXTENT_MM {
            return Err(PushPullGestureError::InvalidDistance);
        }
        let resulting_extent =
            Dimension::new(canonical_number(resulting_extent_mm), resulting_extent_mm)
                .map_err(|_| PushPullGestureError::InvalidDistance)?;
        Ok(PushPullPreview {
            target: self.target.clone(),
            target_reference: self.target_reference.clone(),
            producer_feature_id: self.producer_feature_id,
            source,
            signed_distance,
            resulting_extent,
            snap_feedback,
        })
    }

    fn ensure_current(&self, snapshot: &Snapshot) -> Result<(), PushPullGestureError> {
        if self.binding.is_current(snapshot) {
            Ok(())
        } else {
            Err(PushPullGestureError::StaleIntent)
        }
    }
}

fn resolve_snap(
    raw_signed_distance_mm: f64,
    original_extent_mm: f64,
    settings: &PushPullSnapSettings,
    candidates: &[PushPullSnapCandidate],
) -> Option<PushPullSnapFeedback> {
    let grid = settings.is_enabled(PushPullSnapKind::Grid).then(|| {
        let distance =
            (raw_signed_distance_mm / settings.grid_step_mm).round() * settings.grid_step_mm;
        PushPullSnapCandidate {
            kind: PushPullSnapKind::Grid,
            signed_distance_mm: distance,
            stable_key: format!("grid:{}", canonical_number(settings.grid_step_mm)),
        }
    });
    candidates
        .iter()
        .chain(grid.iter())
        .filter(|candidate| settings.is_enabled(candidate.kind))
        .filter(|candidate| original_extent_mm + candidate.signed_distance_mm > MIN_EXTENT_MM)
        .filter(|candidate| {
            (candidate.signed_distance_mm - raw_signed_distance_mm).abs() <= settings.tolerance_mm
        })
        .min_by(|left, right| compare_snap_candidates(raw_signed_distance_mm, left, right))
        .map(|candidate| PushPullSnapFeedback {
            kind: candidate.kind,
            raw_signed_distance_mm,
            snapped_signed_distance_mm: candidate.signed_distance_mm,
            stable_key: candidate.stable_key.clone(),
        })
}

fn compare_snap_candidates(
    raw_signed_distance_mm: f64,
    left: &PushPullSnapCandidate,
    right: &PushPullSnapCandidate,
) -> std::cmp::Ordering {
    (left.signed_distance_mm - raw_signed_distance_mm)
        .abs()
        .total_cmp(&(right.signed_distance_mm - raw_signed_distance_mm).abs())
        .then_with(|| left.kind.cmp(&right.kind))
        .then_with(|| left.signed_distance_mm.total_cmp(&right.signed_distance_mm))
        .then_with(|| left.stable_key.cmp(&right.stable_key))
}

fn canonical_number(value: f64) -> String {
    value.to_string()
}

#[derive(Debug)]
pub enum PushPullGestureError {
    FaceIntent(FaceIntentError),
    UnsupportedFace,
    ProducerNotFound(FeatureId),
    TargetBodyMismatch,
    InvalidSnapSettings,
    InvalidSnapCandidate,
    TooManySnapCandidates,
    InvalidDistance,
    StaleIntent,
    ForeignPreview,
    NoChange,
    Proposal(ProposalPrepareError),
}

impl fmt::Display for PushPullGestureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FaceIntent(error) => write!(formatter, "face intent rejected: {error}"),
            Self::UnsupportedFace => formatter.write_str(
                "Smart Push/Pull currently supports resolved top faces of existing extrusions",
            ),
            Self::ProducerNotFound(id) => {
                write!(formatter, "face producer feature {} does not exist", id.0)
            }
            Self::TargetBodyMismatch => {
                formatter.write_str("face producer does not own the targeted body")
            }
            Self::InvalidSnapSettings => {
                formatter.write_str("snap grid and tolerance must be finite and bounded")
            }
            Self::InvalidSnapCandidate => {
                formatter.write_str("snap candidate must carry finite distance and stable identity")
            }
            Self::TooManySnapCandidates => {
                formatter.write_str("snap candidate set exceeds the bounded interaction limit")
            }
            Self::InvalidDistance => formatter.write_str(
                "Push/Pull distance must be finite and preserve a positive solid extent",
            ),
            Self::StaleIntent => formatter.write_str("Push/Pull face intent is stale"),
            Self::ForeignPreview => {
                formatter.write_str("Push/Pull preview belongs to another gesture")
            }
            Self::NoChange => formatter.write_str("Push/Pull confirmation must change the extent"),
            Self::Proposal(error) => write!(formatter, "Push/Pull proposal rejected: {error}"),
        }
    }
}

impl std::error::Error for PushPullGestureError {}
