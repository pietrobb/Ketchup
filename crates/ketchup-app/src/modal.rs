//! The one dialog that waits for the user. Opening a dialog replaces
//! whichever one was open, so two reviews can never be pending at once.
//! Accessed through [`crate::slot::Slot`].

use super::{
    PendingCircularPattern, PendingComponentReplacement, PendingDefinitionRename, PendingDxfImport,
    PendingIgesImport, PendingLinearPattern, PendingOccurrenceAlign, PendingOccurrenceDistribution,
    PendingOccurrenceRename, PendingRectangularPattern, PendingSketchupSceneImport,
    PendingStepImport, PendingStlImport, PendingTagAssignment, PendingTagClear, PendingTagCreation,
    PendingTagDeletion, PendingTagRename, glb_import_ui::PendingGlbImport,
};
use crate::slot::slot_variants;

/// Dialog states are boxed: the slot stays one pointer wide however large a
/// review plan grows.
#[derive(Clone, Debug)]
pub(crate) enum Modal {
    DefinitionRename(Box<PendingDefinitionRename>),
    OccurrenceRename(Box<PendingOccurrenceRename>),
    ComponentReplacement(Box<PendingComponentReplacement>),
    TagCreation(Box<PendingTagCreation>),
    TagDeletion(Box<PendingTagDeletion>),
    TagClear(Box<PendingTagClear>),
    TagRename(Box<PendingTagRename>),
    TagAssignment(Box<PendingTagAssignment>),
    OccurrenceAlign(Box<PendingOccurrenceAlign>),
    OccurrenceDistribution(Box<PendingOccurrenceDistribution>),
    LinearPattern(Box<PendingLinearPattern>),
    RectangularPattern(Box<PendingRectangularPattern>),
    CircularPattern(Box<PendingCircularPattern>),
    Import(ImportReview),
}

/// A file import waiting for review, by format.
#[derive(Clone, Debug)]
pub(crate) enum ImportReview {
    Stl(Box<PendingStlImport>),
    Dxf(Box<PendingDxfImport>),
    Step(Box<PendingStepImport>),
    Iges(Box<PendingIgesImport>),
    SketchupScene(Box<PendingSketchupSceneImport>),
    Glb(Box<PendingGlbImport>),
}

slot_variants!(Modal {
    PendingDefinitionRename => DefinitionRename,
    PendingOccurrenceRename => OccurrenceRename,
    PendingComponentReplacement => ComponentReplacement,
    PendingTagCreation => TagCreation,
    PendingTagDeletion => TagDeletion,
    PendingTagClear => TagClear,
    PendingTagRename => TagRename,
    PendingTagAssignment => TagAssignment,
    PendingOccurrenceAlign => OccurrenceAlign,
    PendingOccurrenceDistribution => OccurrenceDistribution,
    PendingLinearPattern => LinearPattern,
    PendingRectangularPattern => RectangularPattern,
    PendingCircularPattern => CircularPattern,
    PendingStlImport => Import(ImportReview::Stl),
    PendingDxfImport => Import(ImportReview::Dxf),
    PendingStepImport => Import(ImportReview::Step),
    PendingIgesImport => Import(ImportReview::Iges),
    PendingSketchupSceneImport => Import(ImportReview::SketchupScene),
    PendingGlbImport => Import(ImportReview::Glb),
});
