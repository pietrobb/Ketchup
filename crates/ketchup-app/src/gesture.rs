//! What the pointer and keyboard are in the middle of: the drawing in
//! progress, a measurement, the transform modifiers, and at most one pointer
//! drag. Pointer drags share one [`crate::slot::Slot`], so starting a zoom
//! window, a selection window or a push/pull ends whichever drag was running.

use super::{Axis, DefinitionId, OccurrenceId, Pos2, PushPullDrag, SelectionWindowDrag, Vec3};
use crate::slot::slot_variants;

#[derive(Default)]
pub(crate) struct Gesture {
    pub(crate) sketch: SketchGesture,
    pub(crate) measure: MeasureGesture,
    pub(crate) transform: TransformModifiers,
    pub(crate) drag: Option<PointerDrag>,
}

/// A line, rectangle, circle, arc, polygon or ellipse being drawn.
#[derive(Default)]
pub(crate) struct SketchGesture {
    /// The drawing tool waits for points.
    pub(crate) armed: bool,
    pub(crate) start: Option<Vec3>,
    pub(crate) end: Option<Vec3>,
    pub(crate) cursor: Option<Vec3>,
    pub(crate) chain_origin: Option<Vec3>,
    pub(crate) chain_points: Vec<Vec3>,
    pub(crate) chain_items: Vec<(DefinitionId, OccurrenceId)>,
    pub(crate) axis_lock: Option<Axis>,
    /// Corners of the next polygon, typed before its centre is placed; kept
    /// between polygons like the rest of the tool's settings.
    pub(crate) polygon_sides: Option<usize>,
    /// The held press placed the first point; releasing it after a drag
    /// places the next point where the pointer is let go.
    pub(crate) dragging_first_point: bool,
}

/// Corners of a polygon until the user types another count.
const DEFAULT_POLYGON_SIDES: usize = 6;

impl SketchGesture {
    pub(crate) fn polygon_sides(&self) -> usize {
        self.polygon_sides.unwrap_or(DEFAULT_POLYGON_SIDES)
    }
}

/// Two picked points; measuring never changes the document.
#[derive(Default)]
pub(crate) struct MeasureGesture {
    pub(crate) start: Option<Vec3>,
    pub(crate) cursor: Option<Vec3>,
    pub(crate) end: Option<Vec3>,
}

/// Copy and axis-lock modifiers of the transform tools. Axis locks are kept
/// across gestures, so a locked axis survives releasing and re-grabbing.
#[derive(Default)]
pub(crate) struct TransformModifiers {
    pub(crate) move_copy: bool,
    pub(crate) move_axis_lock: Option<Axis>,
    pub(crate) rotate_copy: bool,
    pub(crate) rotate_axis_lock: Option<Axis>,
    pub(crate) scale_axis_lock: Option<Axis>,
}

#[derive(Clone)]
pub(crate) enum PointerDrag {
    ZoomWindow(Box<ZoomWindowDrag>),
    SelectionWindow(Box<SelectionWindowDrag>),
    PushPull(Box<PushPullDrag>),
    PushPullAnchor(Box<PushPullAnchor>),
}

/// A zoom window being dragged, in screen points.
#[derive(Clone, Copy)]
pub(crate) struct ZoomWindowDrag {
    pub(crate) start: Pos2,
    pub(crate) cursor: Pos2,
}

/// A push/pull started by a click that waits for the second click.
#[derive(Clone)]
pub(crate) struct PushPullAnchor(pub(crate) PushPullDrag);

slot_variants!(PointerDrag {
    ZoomWindowDrag => ZoomWindow,
    SelectionWindowDrag => SelectionWindow,
    PushPullDrag => PushPull,
    PushPullAnchor => PushPullAnchor,
});
