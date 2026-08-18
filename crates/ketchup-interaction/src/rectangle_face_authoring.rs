#![forbid(unsafe_code)]

use crate::face_intent::{
    FaceIntentError, FaceWorkplaneContext, ResolvedFaceIntent, TransientFaceIntent,
};
use crate::spatial::SnapshotBinding;
use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, Dimension, DocumentStore, FeatureId, FeatureKind,
    Proposal, ProposalContext, ProposalPrepareError, Snapshot,
};
use ketchup_core::sketch::{
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};
use std::fmt;

const MIN_RECTANGLE_SIZE_MM: f64 = 0.01;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RectangleFeatureIds {
    pub workplane: FeatureId,
    pub sketch: FeatureId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RectangleDirection {
    pub positive_x: bool,
    pub positive_y: bool,
}

impl RectangleDirection {
    #[must_use]
    pub const fn positive() -> Self {
        Self {
            positive_x: true,
            positive_y: true,
        }
    }

    fn signs(self) -> [f64; 2] {
        [
            if self.positive_x { 1.0 } else { -1.0 },
            if self.positive_y { 1.0 } else { -1.0 },
        ]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectangleSize {
    width: Dimension,
    depth: Dimension,
}

impl RectangleSize {
    pub fn exact(width: Dimension, depth: Dimension) -> Result<Self, RectangleAuthoringError> {
        let size = Self { width, depth };
        size.validate()?;
        Ok(size)
    }

    fn from_pointer(
        anchor_uv_mm: [f64; 2],
        opposite_uv_mm: [f64; 2],
    ) -> Result<Self, RectangleAuthoringError> {
        let width_mm = (opposite_uv_mm[0] - anchor_uv_mm[0]).abs();
        let depth_mm = (opposite_uv_mm[1] - anchor_uv_mm[1]).abs();
        let width = Dimension::new(width_mm.to_string(), width_mm)
            .map_err(|_| RectangleAuthoringError::InvalidDimensions)?;
        let depth = Dimension::new(depth_mm.to_string(), depth_mm)
            .map_err(|_| RectangleAuthoringError::InvalidDimensions)?;
        Self::exact(width, depth)
    }

    fn validate(&self) -> Result<(), RectangleAuthoringError> {
        if !self.width.millimetres().is_finite()
            || !self.depth.millimetres().is_finite()
            || self.width.millimetres() <= MIN_RECTANGLE_SIZE_MM
            || self.depth.millimetres() <= MIN_RECTANGLE_SIZE_MM
        {
            return Err(RectangleAuthoringError::InvalidDimensions);
        }
        Ok(())
    }

    #[must_use]
    pub const fn width(&self) -> &Dimension {
        &self.width
    }

    #[must_use]
    pub const fn depth(&self) -> &Dimension {
        &self.depth
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectanglePreview {
    target: ResolvedFaceIntent,
    frame: WorkplaneFrame,
    anchor_uv_mm: [f64; 2],
    opposite_uv_mm: [f64; 2],
    size: RectangleSize,
}

impl RectanglePreview {
    #[must_use]
    pub const fn target(&self) -> &ResolvedFaceIntent {
        &self.target
    }

    #[must_use]
    pub const fn frame(&self) -> WorkplaneFrame {
        self.frame
    }

    #[must_use]
    pub const fn anchor_uv_mm(&self) -> [f64; 2] {
        self.anchor_uv_mm
    }

    #[must_use]
    pub const fn opposite_uv_mm(&self) -> [f64; 2] {
        self.opposite_uv_mm
    }

    #[must_use]
    pub const fn size(&self) -> &RectangleSize {
        &self.size
    }

    #[must_use]
    pub fn plane_corners_mm(&self) -> [[f64; 2]; 4] {
        let [ax, ay] = self.anchor_uv_mm;
        let [bx, by] = self.opposite_uv_mm;
        [[ax, ay], [bx, ay], [bx, by], [ax, by]]
    }

    #[must_use]
    pub fn world_corners_mm(&self) -> [[f64; 3]; 4] {
        self.plane_corners_mm()
            .map(|point| point_on_frame(self.frame, point))
    }
}

#[derive(Clone, Debug)]
pub struct RectangleFaceAuthoring {
    binding: SnapshotBinding,
    target: ResolvedFaceIntent,
    anchor_uv_mm: [f64; 2],
}

impl RectangleFaceAuthoring {
    pub fn begin(
        snapshot: &Snapshot,
        face_intent: &TransientFaceIntent,
        pick_through_index: usize,
        anchor_uv_mm: [f64; 2],
    ) -> Result<Self, RectangleAuthoringError> {
        validate_point(anchor_uv_mm)?;
        let target = face_intent
            .resolve(snapshot, pick_through_index)
            .map_err(RectangleAuthoringError::FaceIntent)?;
        Ok(Self {
            binding: SnapshotBinding::from_snapshot(snapshot),
            target,
            anchor_uv_mm,
        })
    }

    #[must_use]
    pub const fn body_id(&self) -> BodyId {
        self.target.target.body_id
    }

    #[must_use]
    pub fn workplane_frame(&self) -> WorkplaneFrame {
        self.target.target.workplane.frame()
    }

    pub fn preview_pointer(
        &self,
        snapshot: &Snapshot,
        opposite_uv_mm: [f64; 2],
    ) -> Result<RectanglePreview, RectangleAuthoringError> {
        self.ensure_current(snapshot)?;
        validate_point(opposite_uv_mm)?;
        let size = RectangleSize::from_pointer(self.anchor_uv_mm, opposite_uv_mm)?;
        Ok(self.preview(opposite_uv_mm, size))
    }

    pub fn preview_exact(
        &self,
        snapshot: &Snapshot,
        size: RectangleSize,
        direction: RectangleDirection,
    ) -> Result<RectanglePreview, RectangleAuthoringError> {
        self.ensure_current(snapshot)?;
        size.validate()?;
        let signs = direction.signs();
        let opposite_uv_mm = [
            self.anchor_uv_mm[0] + signs[0] * size.width.millimetres(),
            self.anchor_uv_mm[1] + signs[1] * size.depth.millimetres(),
        ];
        validate_point(opposite_uv_mm)?;
        Ok(self.preview(opposite_uv_mm, size))
    }

    pub fn plan_proposal(
        &self,
        document: &DocumentStore,
        ids: RectangleFeatureIds,
        preview: &RectanglePreview,
    ) -> Result<Proposal, RectangleAuthoringError> {
        let snapshot = document.current();
        self.ensure_current(&snapshot)?;
        if preview.target != self.target
            || preview.frame != self.workplane_frame()
            || preview.anchor_uv_mm != self.anchor_uv_mm
        {
            return Err(RectangleAuthoringError::ForeignPreview);
        }
        preview.size.validate()?;
        let target = &self.target.target;
        let support = match &target.workplane {
            FaceWorkplaneContext::Datum(plane) => WorkplaneSupport::Principal(*plane),
            FaceWorkplaneContext::PlanarFace {
                reference, health, ..
            } => WorkplaneSupport::PlanarFace {
                reference: reference.clone(),
                health: *health,
            },
        };
        let workplane = WorkplaneSpec {
            support,
            frame: preview.frame,
        };
        let sketch = rectangle_sketch(ids.workplane, preview);
        document
            .prepare_proposal_with_context(
                CommandBatch::new(vec![
                    CanonicalCommand::SetActiveBody {
                        definition_id: target.definition_id,
                        id: target.body_id,
                    },
                    CanonicalCommand::CreateFeature {
                        id: ids.workplane,
                        definition_id: target.definition_id,
                        name: "Rectangle workplane".to_owned(),
                        kind: FeatureKind::Workplane(workplane),
                    },
                    CanonicalCommand::CreateFeature {
                        id: ids.sketch,
                        definition_id: target.definition_id,
                        name: "Rectangle".to_owned(),
                        kind: FeatureKind::Sketch(sketch),
                    },
                ]),
                ProposalContext::canonical_preview(),
            )
            .map_err(RectangleAuthoringError::Proposal)
    }

    fn preview(&self, opposite_uv_mm: [f64; 2], size: RectangleSize) -> RectanglePreview {
        RectanglePreview {
            target: self.target.clone(),
            frame: self.workplane_frame(),
            anchor_uv_mm: self.anchor_uv_mm,
            opposite_uv_mm,
            size,
        }
    }

    fn ensure_current(&self, snapshot: &Snapshot) -> Result<(), RectangleAuthoringError> {
        if self.binding.is_current(snapshot) {
            Ok(())
        } else {
            Err(RectangleAuthoringError::StaleIntent)
        }
    }
}

fn rectangle_sketch(workplane: FeatureId, preview: &RectanglePreview) -> SketchSpec {
    let [a, b, c, d] = preview.plane_corners_mm();
    let point = |entity, point| SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    };
    SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: a,
                end_mm: b,
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: b,
                end_mm: c,
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: c,
                end_mm: d,
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: d,
                end_mm: a,
            },
        ],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(1),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::Vertical {
                    entity: SketchEntityId(2),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(3),
                kind: SketchConstraintKind::Horizontal {
                    entity: SketchEntityId(3),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(4),
                kind: SketchConstraintKind::Vertical {
                    entity: SketchEntityId(4),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(5),
                kind: SketchConstraintKind::Coincident {
                    a: point(1, SketchPointKind::End),
                    b: point(2, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(6),
                kind: SketchConstraintKind::Coincident {
                    a: point(2, SketchPointKind::End),
                    b: point(3, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(7),
                kind: SketchConstraintKind::Coincident {
                    a: point(3, SketchPointKind::End),
                    b: point(4, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(8),
                kind: SketchConstraintKind::Coincident {
                    a: point(4, SketchPointKind::End),
                    b: point(1, SketchPointKind::Start),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(9),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Start),
                    position_mm: a,
                },
            },
            SketchConstraint {
                id: SketchConstraintId(10),
                kind: SketchConstraintKind::Distance {
                    a: point(1, SketchPointKind::Start),
                    b: point(1, SketchPointKind::End),
                    value: preview.size.width.clone(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(11),
                kind: SketchConstraintKind::Distance {
                    a: point(2, SketchPointKind::Start),
                    b: point(2, SketchPointKind::End),
                    value: preview.size.depth.clone(),
                },
            },
        ],
    }
}

fn point_on_frame(frame: WorkplaneFrame, point: [f64; 2]) -> [f64; 3] {
    let mut world = frame.origin_mm;
    for (index, coordinate) in world.iter_mut().enumerate() {
        *coordinate += frame.x_axis[index] * point[0] + frame.y_axis[index] * point[1];
    }
    world
}

fn validate_point(point: [f64; 2]) -> Result<(), RectangleAuthoringError> {
    if point.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(RectangleAuthoringError::InvalidPoint)
    }
}

#[derive(Debug)]
pub enum RectangleAuthoringError {
    FaceIntent(FaceIntentError),
    InvalidPoint,
    InvalidDimensions,
    StaleIntent,
    ForeignPreview,
    Proposal(ProposalPrepareError),
}

impl fmt::Display for RectangleAuthoringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FaceIntent(error) => write!(formatter, "face intent rejected: {error}"),
            Self::InvalidPoint => formatter.write_str("rectangle points must be finite"),
            Self::InvalidDimensions => {
                formatter.write_str("rectangle dimensions must be finite and positive")
            }
            Self::StaleIntent => formatter.write_str("rectangle authoring intent is stale"),
            Self::ForeignPreview => {
                formatter.write_str("rectangle preview belongs to another authoring intent")
            }
            Self::Proposal(error) => write!(formatter, "rectangle proposal rejected: {error}"),
        }
    }
}

impl std::error::Error for RectangleAuthoringError {}
