//! The one dialog that waits for the user. Opening a dialog replaces
//! whichever one was open, so two reviews can never be pending at once.

use super::{
    PendingCircularPattern, PendingComponentReplacement, PendingDefinitionRename, PendingDxfImport,
    PendingIgesImport, PendingLinearPattern, PendingOccurrenceAlign, PendingOccurrenceDistribution,
    PendingOccurrenceRename, PendingRectangularPattern, PendingSketchupSceneImport,
    PendingStepImport, PendingStlImport, PendingTagAssignment, PendingTagClear, PendingTagCreation,
    PendingTagDeletion, PendingTagRename, glb_import_ui::PendingGlbImport,
};

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

/// A dialog state that lives in [`Modal`].
pub(crate) trait ModalDialog: Sized + Into<Modal> {
    fn of(modal: &Modal) -> Option<&Self>;
    fn of_mut(modal: &mut Modal) -> Option<&mut Self>;
    fn from_modal(modal: Modal) -> Result<Self, Modal>;
}

macro_rules! modal_dialog {
    ($($ty:ty => $($path:ident)::+),* $(,)?) => {$(
        impl From<$ty> for Modal {
            fn from(dialog: $ty) -> Self {
                let dialog = Box::new(dialog);
                modal_dialog!(@wrap dialog, $($path)::+)
            }
        }
        impl ModalDialog for $ty {
            fn of(modal: &Modal) -> Option<&Self> {
                match modal {
                    modal_dialog!(@pat dialog, $($path)::+) => Some(dialog),
                    _ => None,
                }
            }
            fn of_mut(modal: &mut Modal) -> Option<&mut Self> {
                match modal {
                    modal_dialog!(@pat dialog, $($path)::+) => Some(dialog),
                    _ => None,
                }
            }
            fn from_modal(modal: Modal) -> Result<Self, Modal> {
                match modal {
                    modal_dialog!(@pat dialog, $($path)::+) => Ok(*dialog),
                    other => Err(other),
                }
            }
        }
    )*};
    (@wrap $value:ident, $variant:ident) => { Modal::$variant($value) };
    (@wrap $value:ident, Import :: $format:ident) => {
        Modal::Import(ImportReview::$format($value))
    };
    (@pat $value:ident, $variant:ident) => { Modal::$variant($value) };
    (@pat $value:ident, Import :: $format:ident) => {
        Modal::Import(ImportReview::$format($value))
    };
}

modal_dialog! {
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
    PendingStlImport => Import::Stl,
    PendingDxfImport => Import::Dxf,
    PendingStepImport => Import::Step,
    PendingIgesImport => Import::Iges,
    PendingSketchupSceneImport => Import::SketchupScene,
    PendingGlbImport => Import::Glb,
}

/// Typed access to the dialog slot; borrows only the slot, so the rest of
/// the app stays usable while a dialog is read or edited.
pub(crate) trait ModalSlot {
    fn get<D: ModalDialog>(&self) -> Option<&D>;
    fn get_mut<D: ModalDialog>(&mut self) -> Option<&mut D>;
    /// Show `dialog`, replacing whichever dialog was open.
    fn open<D: ModalDialog>(&mut self, dialog: D);
    /// Close the dialog if it is a `D`; another open dialog stays.
    fn close<D: ModalDialog>(&mut self);
    /// Close the dialog if it is a `D` and hand it back.
    fn remove<D: ModalDialog>(&mut self) -> Option<D>;
}

impl ModalSlot for Option<Modal> {
    fn get<D: ModalDialog>(&self) -> Option<&D> {
        self.as_ref().and_then(D::of)
    }

    fn get_mut<D: ModalDialog>(&mut self) -> Option<&mut D> {
        self.as_mut().and_then(D::of_mut)
    }

    fn open<D: ModalDialog>(&mut self, dialog: D) {
        *self = Some(dialog.into());
    }

    fn close<D: ModalDialog>(&mut self) {
        if self.get::<D>().is_some() {
            *self = None;
        }
    }

    fn remove<D: ModalDialog>(&mut self) -> Option<D> {
        match D::from_modal(self.take()?) {
            Ok(dialog) => Some(dialog),
            Err(other) => {
                *self = Some(other);
                None
            }
        }
    }
}
