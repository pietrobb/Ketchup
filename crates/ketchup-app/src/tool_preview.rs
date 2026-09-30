//! The one live tool preview the viewport shows before a commit. Starting a
//! preview replaces the previous one, and cancel is a single assignment.
//! Accessed through [`crate::slot::Slot`].

use super::{
    EphemeralBoxPreview, GeneralFinishPreview, LoftPreview, OccurrenceOperationPreview,
    PlanarOffsetPreview, RevolvePreview, SweepPreview, drawn_shape::DrawnShapePreview,
};
use crate::slot::slot_variants;

#[derive(Clone, Debug)]
pub(crate) enum ToolPreview {
    PushPull(Box<EphemeralBoxPreview>),
    DrawnShape(Box<DrawnShapePreview>),
    OccurrenceOperation(Box<OccurrenceOperationPreview>),
    Revolve(Box<RevolvePreview>),
    PlanarOffset(Box<PlanarOffsetPreview>),
    Sweep(Box<SweepPreview>),
    Loft(Box<LoftPreview>),
    GeneralFinish(Box<GeneralFinishPreview>),
}

slot_variants!(ToolPreview {
    EphemeralBoxPreview => PushPull,
    DrawnShapePreview => DrawnShape,
    OccurrenceOperationPreview => OccurrenceOperation,
    RevolvePreview => Revolve,
    PlanarOffsetPreview => PlanarOffset,
    SweepPreview => Sweep,
    LoftPreview => Loft,
    GeneralFinishPreview => GeneralFinish,
});
