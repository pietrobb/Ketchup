//! What the pointer and keyboard are in the middle of: the drawing in
//! progress, a measurement, the transform modifiers, and at most one pointer
//! drag. Pointer drags share one [`crate::slot::Slot`], so starting a zoom
//! window, a selection window or a push/pull ends whichever drag was running.
//! Also the Move, Rotate and Scale sessions and the corrections a finished
//! transform or push/pull leaves open to a typed value.

use std::collections::BTreeSet;

use super::{
    Axis, BodyId, DefinitionId, DocumentId, ElementId, FeatureId, GroupId, InstancePath,
    OccurrenceId, Pos2, PushPullSourcePlan, SelectionId, SnapshotBoundTopologicalSelection, Vec2,
    Vec3, planar_push_pull,
};
use crate::slot::slot_variants;

#[derive(Default)]
pub(crate) struct Gesture {
    pub(crate) sketch: SketchGesture,
    pub(crate) measure: MeasureGesture,
    pub(crate) transform: TransformModifiers,
    /// The face plane under the pointer while the Mirror tool is active.
    pub(crate) mirror: Option<MirrorPlane>,
    /// While the Offset tool is active: the pointer's distance from the selected
    /// profile's outline in its plane, negative inside.
    pub(crate) planar_offset_mm: Option<f64>,
    pub(crate) drag: Option<PointerDrag>,
}

/// A point on a picked face and the face's outward normal; the Mirror tool
/// reflects across this plane, shifted along the normal by the typed offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MirrorPlane {
    pub(crate) point_mm: Vec3,
    pub(crate) normal: Vec3,
    /// The document and revision the face was read from.
    pub(crate) source: (DocumentId, u64),
}

/// A line, rectangle, circle, arc, polygon or ellipse being drawn.
#[derive(Default)]
pub(crate) struct SketchGesture {
    /// The drawing tool waits for points.
    pub(crate) armed: bool,
    pub(crate) start: Option<Vec3>,
    /// The document and revision `start` was placed on; a commit against any
    /// other is refused, like a stale transform preview.
    pub(crate) source: Option<(DocumentId, u64)>,
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
    /// The start point and pointer reading last written into the value box;
    /// an unchanged reading of the same shape leaves the box alone, so a
    /// refused typed value stays readable.
    pub(crate) shown_reading: Option<(Vec3, String)>,
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
    /// While Ctrl is held: the whole edge under the pointer and its length in mm.
    pub(crate) edge: Option<(Vec<Vec3>, f64)>,
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

#[derive(Clone)]
pub(crate) struct PushPullDrag {
    pub(crate) source_document_id: DocumentId,
    pub(crate) source_revision: u64,
    pub(crate) source_digest: String,
    pub(crate) selection: SelectionId,
    pub(crate) pointer_start: Pos2,
    pub(crate) extent_start_mm: f64,
    pub(crate) screen_normal: Vec2,
    pub(crate) pixels_per_mm: f32,
}

#[derive(Clone)]
pub(crate) struct LastPushPull {
    pub(crate) selection: SelectionId,
    /// The exact face it pushed, empty for a box face. An exact-face Push/Pull
    /// clears the selection, so a correction restores the face from here.
    pub(crate) topological: Vec<(SelectionId, SnapshotBoundTopologicalSelection)>,
    /// That exact face as planned against the Push/Pull's parent revision.
    pub(crate) face: Option<(PushPullSourcePlan, planar_push_pull::PlanarFace)>,
    /// Document revision produced by that Push/Pull. A typed value is treated as
    /// a correction of this operation only while the document still sits on it.
    pub(crate) revision: u64,
    pub(crate) canonical_digest: String,
}

#[derive(Clone)]
pub(crate) struct MoveProfileTarget {
    pub(crate) definition_id: DefinitionId,
    pub(crate) body_id: BodyId,
    pub(crate) profile_id: FeatureId,
    pub(crate) world_origin: Vec3,
    pub(crate) world_x_axis: Vec3,
    pub(crate) world_y_axis: Vec3,
}

#[derive(Clone, Copy)]
pub(crate) struct SelectionWindowDrag {
    pub(crate) start: Pos2,
    pub(crate) cursor: Pos2,
    pub(crate) additive: bool,
}

#[derive(Clone)]
pub(crate) struct MoveDrag {
    pub(crate) source_document_id: DocumentId,
    pub(crate) source_revision: u64,
    pub(crate) selection: SelectionId,
    pub(crate) occurrence_paths: BTreeSet<InstancePath>,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) profile_target: Option<MoveProfileTarget>,
    pub(crate) pointer_start_world: Vec3,
    pub(crate) plane_z: f64,
    /// The axis the arrow keys pinned this gesture to, if any. Without a pin
    /// the pointer stays on the horizontal plane it started on.
    pub(crate) axis: Option<Axis>,
    /// Where along the pinned axis the gesture started, established from the
    /// first pointer sample after the pin so the body never jumps.
    pub(crate) axis_reference: Option<f64>,
    pub(crate) delta_mm: Vec3,
    pub(crate) copy: bool,
}

/// One in-progress Scale gesture about the pivot `centre_mm`.
///
/// The pointer drives one factor: from a grip, along the screen line from the
/// pivot to the grip; elsewhere, by its distance from the pivot. `drives`
/// names the world axes that factor stretches, the others stay at 1; a typed
/// `x;y;z` sets each axis on its own. `extent_mm` is the selection's world
/// size, which turns a typed length into a factor.
#[derive(Clone)]
pub(crate) struct ScaleDrag {
    pub(crate) source_document_id: DocumentId,
    pub(crate) source_revision: u64,
    pub(crate) selection: SelectionId,
    pub(crate) occurrence_paths: BTreeSet<InstancePath>,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) centre_mm: Vec3,
    pub(crate) centre_screen: Pos2,
    pub(crate) reference_screen: Vec2,
    pub(crate) along_reference: bool,
    pub(crate) drives: [bool; 3],
    pub(crate) factors: [f64; 3],
    pub(crate) extent_mm: [f64; 3],
}

/// One in-progress Rotate gesture.
///
/// The rotation happens in the plane through `centre_mm` whose normal is
/// `axis`, so the live angle is read from where the pointer meets that plane
/// rather than from raw screen motion.
#[derive(Clone)]
pub(crate) struct RotateDrag {
    pub(crate) source_document_id: DocumentId,
    pub(crate) source_revision: u64,
    pub(crate) selection: SelectionId,
    pub(crate) occurrence_paths: BTreeSet<InstancePath>,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) centre_mm: Vec3,
    pub(crate) axis: Axis,
    /// The starting arm, measured from `centre_mm` inside the rotation plane.
    ///
    /// The first click chooses the pivot; a second click establishes this arm.
    /// Until then the gesture reads zero and accepts a typed angle.
    pub(crate) reference_mm: Option<Vec3>,
    pub(crate) angle_degrees: f64,
    pub(crate) copy: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolSessionPhase {
    Gesture,
    Anchor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransformInputEvent {
    ToggleCopy,
    CopyRequested,
}

#[derive(Default)]
pub(crate) struct TransformInputInterpreter {
    pub(crate) command_down: bool,
    pub(crate) command_chord_seen: bool,
}

impl TransformInputInterpreter {
    pub(crate) fn interpret_command(
        &mut self,
        enabled: bool,
        command_down: bool,
        command_chord: bool,
    ) -> Option<TransformInputEvent> {
        if !enabled {
            self.command_down = command_down;
            self.command_chord_seen = true;
            return None;
        }
        if command_down {
            if !self.command_down {
                self.command_chord_seen = command_chord;
            } else {
                self.command_chord_seen |= command_chord;
            }
            self.command_down = true;
            return None;
        }

        let event = (self.command_down && !self.command_chord_seen)
            .then_some(TransformInputEvent::ToggleCopy);
        self.command_down = false;
        self.command_chord_seen = false;
        event
    }

    pub(crate) fn interpret_pointer_copy(command_down: bool) -> Option<TransformInputEvent> {
        command_down.then_some(TransformInputEvent::CopyRequested)
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone)]
pub(crate) enum ToolSession {
    Move {
        phase: ToolSessionPhase,
        drag: MoveDrag,
    },
    Rotate {
        phase: ToolSessionPhase,
        drag: RotateDrag,
    },
    Scale {
        phase: ToolSessionPhase,
        drag: ScaleDrag,
    },
}

/// The on-screen protractor a frame draws for the Rotate tool.
#[derive(Clone, Copy)]
pub(crate) struct RotationGuide {
    pub(crate) centre_mm: Vec3,
    pub(crate) axis: Axis,
    pub(crate) radius_mm: f64,
    /// Where the starting arm points inside the rotation plane, once a gesture
    /// has established one. `None` means the tool is merely armed.
    pub(crate) start_degrees: Option<f64>,
    pub(crate) angle_degrees: f64,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum CorrectionSelection {
    Occurrences {
        occurrence_ids: BTreeSet<OccurrenceId>,
        primary_occurrence_id: Option<OccurrenceId>,
    },
    Group(GroupId),
}

#[derive(Clone)]
pub(crate) struct MoveCorrection {
    pub(crate) direction: Vec3,
    pub(crate) applied_distance_mm: f64,
    pub(crate) accepts_vector_correction: bool,
}

#[derive(Clone)]
pub(crate) struct MoveCopyCorrection {
    pub(crate) source_occurrence_ids: Vec<OccurrenceId>,
    pub(crate) first_copy_occurrence_id: OccurrenceId,
    pub(crate) primary_source_index: usize,
    pub(crate) element: ElementId,
    pub(crate) delta_mm: Vec3,
    pub(crate) array_mode: MoveCopyArrayMode,
    pub(crate) array_count: usize,
}

#[derive(Clone)]
pub(crate) enum CorrectionOperation {
    Move(MoveCorrection),
    MoveCopy(MoveCopyCorrection),
    Rotate(RotateCorrection),
    Scale(ScaleCorrection),
}

#[derive(Clone)]
pub(crate) struct CorrectionSession {
    pub(crate) revision: u64,
    pub(crate) canonical_digest: String,
    pub(crate) selection: CorrectionSelection,
    pub(crate) operation: CorrectionOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MoveCopyArrayMode {
    Multiply,
    Divide,
}

#[derive(Clone)]
pub(crate) struct RotateCorrection {
    pub(crate) copy_source_occurrence_ids: Option<Vec<OccurrenceId>>,
    pub(crate) centre_mm: Vec3,
    pub(crate) axis: Axis,
}

#[derive(Clone)]
pub(crate) struct ScaleCorrection {
    pub(crate) centre_mm: Vec3,
    pub(crate) drives: [bool; 3],
    pub(crate) extent_mm: [f64; 3],
}
