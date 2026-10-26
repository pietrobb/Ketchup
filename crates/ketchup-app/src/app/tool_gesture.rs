//! One model for a tool gesture in progress: a Move, Rotate or Scale, a shape
//! being drawn, a Push/Pull, the face Mirror reflects across, a Revolve,
//! Offset, Shell/Fillet/Chamfer, Sweep or Loft preview, or the Helix. Each tool
//! answers the same questions through [`ToolGesture`], and [`GESTURES`] is
//! the one registry Esc, Ctrl+Z and a
//! typed value go through. Whether a gesture still stands, what ends it and
//! which tool a typed value is for are decided once, not once per tool.

use crate::length_input::DecimalSeparator;
use crate::*;

/// A tool that can have a gesture in progress and takes typed values.
pub(crate) trait ToolGesture: Sync {
    /// This tool has a gesture in progress.
    fn is_open(&self, app: &KetchupApp) -> bool;
    /// The open gesture still stands on the document revision and the
    /// selection it started on.
    fn is_current(&self, app: &KetchupApp) -> bool;
    /// Ends the gesture without changing the document.
    fn cancel(&self, app: &mut KetchupApp);
    /// Ctrl+Z ends the open gesture instead of undoing a document step.
    fn holds_undo(&self, app: &KetchupApp) -> bool {
        self.is_current(app)
    }
    /// A value typed now is for this tool.
    fn takes_typed(&self, app: &KetchupApp) -> bool;
    /// Applies the typed value. `false` leaves the value in the box and names
    /// the reason in the digest.
    fn apply_typed(&self, app: &mut KetchupApp) -> bool;
}

/// Every tool with a gesture, in the order a typed value is offered to them.
pub(crate) static GESTURES: [&dyn ToolGesture; 12] = [
    &DrawTool,
    &ScaleTool,
    &RotateTool,
    &PushPullTool,
    &MoveTool,
    &MirrorTool,
    &RevolveTool,
    &PlanarOffsetTool,
    &FinishTool,
    &SweepTool,
    &LoftTool,
    &HelixTool,
];

struct DrawTool;
struct ScaleTool;
struct RotateTool;
struct PushPullTool;
struct MoveTool;
struct MirrorTool;
struct RevolveTool;
struct PlanarOffsetTool;
/// Shell, Fillet and Chamfer.
struct FinishTool;
struct SweepTool;
struct LoftTool;
struct HelixTool;

impl ToolGesture for DrawTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.gesture.sketch.start.is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        let snapshot = app.document.current();
        app.gesture
            .sketch
            .source
            .is_none_or(|source| source == (snapshot.document_id(), snapshot.revision_id()))
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.cancel_rectangle_sketch();
    }

    /// A line chain's Undo takes back its last committed segment and ends the chain.
    fn holds_undo(&self, app: &KetchupApp) -> bool {
        self.is_current(app) && app.gesture.sketch.chain_items.is_empty()
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.gesture.sketch.armed
            && (app.gesture.sketch.start.is_some() || app.active_tool == ActiveTool::Polygon)
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.apply_typed_drawing()
    }
}

impl ToolGesture for ScaleTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.scale_session().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.scale_session().is_some_and(|(drag, _)| {
            app.transform_source_is_current(
                drag.source_document_id,
                drag.source_revision,
                &drag.selection,
                &drag.occurrence_paths,
                drag.group_id,
            )
        })
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.cancel_transform_session();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Scale
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.apply_typed_scale()
    }
}

impl ToolGesture for RotateTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.rotate_session().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.rotate_session().is_some_and(|(drag, _)| {
            app.transform_source_is_current(
                drag.source_document_id,
                drag.source_revision,
                &drag.selection,
                &drag.occurrence_paths,
                drag.group_id,
            )
        })
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.cancel_transform_session();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Rotate
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.apply_typed_rotate()
    }
}

impl ToolGesture for PushPullTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.gesture.drag.get::<PushPullDrag>().is_some()
            || app.gesture.drag.get::<PushPullAnchor>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.gesture
            .drag
            .get::<PushPullDrag>()
            .or_else(|| {
                app.gesture
                    .drag
                    .get::<PushPullAnchor>()
                    .map(|anchor| &anchor.0)
            })
            .is_some_and(|drag| app.push_pull_gesture_is_current(drag))
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.gesture.drag.close::<PushPullDrag>();
        app.gesture.drag.close::<PushPullAnchor>();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::PushPull
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.apply_typed_push_pull()
    }
}

impl ToolGesture for MoveTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.move_session().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.move_session()
            .is_some_and(|(drag, _)| app.move_preview_is_current(drag))
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.cancel_transform_session();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Move
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.apply_typed_move()
    }
}

/// The face the pointer last rested on, which a typed offset mirrors across.
impl ToolGesture for MirrorTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.gesture.mirror.is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.mirror_plane_is_current()
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.gesture.mirror = None;
    }

    /// A face under the pointer is no step of its own: Ctrl+Z undoes.
    fn holds_undo(&self, _app: &KetchupApp) -> bool {
        false
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Mirror
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.commit_mirror()
    }
}

/// The profile picked, its axis being placed, and the exact preview of the
/// typed angle.
impl ToolGesture for RevolveTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.solid_tools.revolve.is_some() || app.tool_preview.get::<RevolvePreview>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.solid_tools.revolve.as_ref().is_some_and(|tool| {
            app.selected_revolve_profile()
                .is_some_and(|current| current.source == tool.source)
        })
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.solid_tools.revolve = None;
        app.tool_preview.close::<RevolvePreview>();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Revolve
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.refresh_revolve_preview() && app.confirm_revolve_preview()
    }
}

/// The exact preview of the face offset by the typed distance.
impl ToolGesture for PlanarOffsetTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.tool_preview.get::<PlanarOffsetPreview>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.tool_preview
            .get::<PlanarOffsetPreview>()
            .is_some_and(|preview| {
                app.planar_offset_source_plan().as_ref() == Some(&preview.plan.source)
            })
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.tool_preview.close::<PlanarOffsetPreview>();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::PlanarOffset
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        app.refresh_planar_offset_preview() && app.confirm_planar_offset_preview()
    }
}

/// The exact preview of a Shell, Fillet or Chamfer by the typed amount.
impl ToolGesture for FinishTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.tool_preview.get::<GeneralFinishPreview>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.tool_preview
            .get::<GeneralFinishPreview>()
            .is_some_and(|preview| {
                let source = &preview.plan.source;
                app.general_finish_source_plan(source.kind).as_ref() == Some(source)
            })
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.tool_preview.close::<GeneralFinishPreview>();
    }

    fn takes_typed(&self, app: &KetchupApp) -> bool {
        matches!(
            app.active_tool,
            ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
        )
    }

    fn apply_typed(&self, app: &mut KetchupApp) -> bool {
        let applied = app.refresh_general_finish_preview() && app.confirm_general_finish_preview();
        app.await_program_if_planning();
        applied
    }
}

/// The exact preview of the profile swept along the picked path; it takes
/// no typed value.
impl ToolGesture for SweepTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.tool_preview.get::<SweepPreview>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.tool_preview
            .get::<SweepPreview>()
            .is_some_and(|preview| app.sweep_source_plan().as_ref() == Some(&preview.plan.source))
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.tool_preview.close::<SweepPreview>();
    }

    fn takes_typed(&self, _app: &KetchupApp) -> bool {
        false
    }

    fn apply_typed(&self, _app: &mut KetchupApp) -> bool {
        false
    }
}

/// The exact preview of the loft through the picked sections; it takes no
/// typed value.
impl ToolGesture for LoftTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.tool_preview.get::<LoftPreview>().is_some()
    }

    fn is_current(&self, app: &KetchupApp) -> bool {
        app.tool_preview
            .get::<LoftPreview>()
            .is_some_and(|preview| app.loft_source_plan().as_ref() == Some(&preview.plan.source))
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.tool_preview.close::<LoftPreview>();
    }

    fn takes_typed(&self, _app: &KetchupApp) -> bool {
        false
    }

    fn apply_typed(&self, _app: &mut KetchupApp) -> bool {
        false
    }
}

/// The Helix window and its preview: its values are typed in the window's
/// own fields, and it stands on no selection, so it is never stale.
impl ToolGesture for HelixTool {
    fn is_open(&self, app: &KetchupApp) -> bool {
        app.active_tool == ActiveTool::Helix
    }

    fn is_current(&self, _app: &KetchupApp) -> bool {
        true
    }

    fn cancel(&self, app: &mut KetchupApp) {
        app.clear_helix_preview();
        app.active_tool = ActiveTool::Select;
    }

    fn takes_typed(&self, _app: &KetchupApp) -> bool {
        false
    }

    fn apply_typed(&self, _app: &mut KetchupApp) -> bool {
        false
    }
}

impl KetchupApp {
    /// A gesture is in progress in any tool, current or not.
    pub(crate) fn gesture_open(&self) -> bool {
        GESTURES.iter().any(|tool| tool.is_open(self))
    }

    /// Ctrl+Z ends a gesture in progress rather than undoing a document step.
    /// A gesture the document or selection moved away from holds nothing: it
    /// is no longer drawn, and Ctrl+Z undoes the last step.
    pub(crate) fn gesture_holds_undo(&self) -> bool {
        GESTURES
            .iter()
            .any(|tool| tool.is_open(self) && tool.holds_undo(self))
    }

    /// Ends every gesture whose document revision or selection moved on;
    /// `true` when one was ended.
    fn drop_stale_gestures(&mut self) -> bool {
        let mut dropped = false;
        for tool in GESTURES {
            if tool.is_open(self) && !tool.is_current(self) {
                tool.cancel(self);
                dropped = true;
            }
        }
        dropped
    }

    /// Applies the value box to the active tool. The same rule holds for
    /// every tool: a value typed for a gesture the document or selection moved
    /// away from is refused with the reason and the gesture ends, so the value
    /// never lands on another part or along a way nobody showed. Confirming
    /// it again applies it to what is selected now.
    pub(crate) fn apply_value_input(&mut self) -> bool {
        if self.drop_stale_gestures() {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        // A comma is refused the same way in every field, before any tool
        // reads it, so nothing is guessed from `80,60`.
        if self.value_box_refuses_comma() {
            self.digest = self.catalog.text("value-comma-not-decimal");
            return false;
        }
        if self.value_box_separates_one_length() {
            self.digest = self.catalog.text("value-x-separates");
            return false;
        }
        if let Some(tool) = GESTURES.iter().find(|tool| tool.takes_typed(self)) {
            return tool.apply_typed(self);
        }
        self.apply_typed_without_gesture()
    }

    fn apply_typed_drawing(&mut self) -> bool {
        if self.gesture.sketch.start.is_none() {
            // Polygon armed: the value is the next polygon's corner count.
            return self.set_polygon_sides_from_value_box();
        }
        match self.active_tool {
            ActiveTool::Line => self.complete_exact_line(),
            ActiveTool::Circle => self.complete_exact_circle(),
            ActiveTool::Arc => self.complete_exact_arc(),
            ActiveTool::Polygon => self.complete_exact_polygon(),
            ActiveTool::Ellipse => self.complete_exact_ellipse(),
            ActiveTool::Spline => self.complete_exact_spline(),
            _ => self.complete_exact_rectangle(),
        }
    }

    fn apply_typed_scale(&mut self) -> bool {
        let Some(input) = ScaleInput::parse(&self.value_box.input, self.value_box.decimal) else {
            self.digest = self.catalog.text("digest-scale-invalid-factor");
            return false;
        };
        if let Some((drag, _)) = self.scale_session() {
            let mut drag = drag.clone();
            // A refused value leaves the gesture open for another try.
            match input.factors(drag.drives, drag.extent_mm) {
                Ok(factors) => drag.factors = factors,
                Err(key) => {
                    self.digest = self.catalog.text(key);
                    return false;
                }
            }
            self.take_scale_session(None);
            return self.commit_scale_drag(&drag);
        }
        // A refused correction leaves the last step open for another value.
        if self.current_scale_correction().is_some() {
            return self.correct_last_scale(input);
        }
        self.scale_selected_by(input)
    }

    fn apply_typed_rotate(&mut self) -> bool {
        let Some(angle_degrees) =
            super::commands::parse_angle_degrees(&self.value_box.input, self.value_box.decimal)
                .filter(|angle| angle.abs() <= 360.0)
        else {
            self.digest = self.catalog.text("digest-rotate-invalid-angle");
            return false;
        };
        // With no gesture in flight the value box edits the turn that just
        // happened rather than adding a second one.
        if self.rotate_session().is_none() && self.correct_last_rotation(angle_degrees) {
            return true;
        }
        if !rotation_is_meaningful(angle_degrees) {
            self.digest = self.catalog.text("digest-rotate-invalid-angle");
            return false;
        }
        // A gesture in flight already knows its centre and axis; typing an
        // angle replaces the one read from the pointer.
        if let Some(mut drag) = self.take_rotate_session(None) {
            drag.angle_degrees = angle_degrees;
            return self.commit_rotate_drag(&drag);
        }
        self.rotate_selected(angle_degrees)
    }

    fn apply_typed_push_pull(&mut self) -> bool {
        // An unreadable value is refused before the gesture or the Push/Pull
        // it would correct is touched. Zero is refused too, except as a
        // correction, which returns the face where it was, as with Rotate.
        let corrects_last = self.current_last_push_pull().is_some_and(|last| {
            self.selection
                .primary
                .as_ref()
                .is_none_or(|primary| *primary == last.selection)
        });
        if parse_distance_mm(&self.value_box.input, self.value_box.decimal)
            .is_none_or(|distance| !corrects_last && distance.abs() <= limits::MIN_LENGTH_MM)
        {
            self.digest = self.catalog.text("digest-push-pull-invalid-distance");
            return false;
        }
        // A value typed right after an exact-face Push/Pull corrects that
        // face, whether the selection was cleared or still holds the pushed
        // face (re-bound by Undo and Redo, so its recorded binding is used).
        if let Some(last) = self.current_last_push_pull().cloned().filter(|last| {
            self.selection
                .primary
                .as_ref()
                .is_none_or(|primary| *primary == last.selection)
        }) {
            self.selection.primary = Some(last.selection);
            self.selection.topological = last.topological;
        }
        let Some(selection) = self.selection.primary.clone() else {
            self.status_key = "error-push-pull-selection-required";
            self.digest = self.catalog.text("error-push-pull-selection-required");
            return false;
        };
        // A value typed straight after a Push/Pull is planned as an absolute
        // replacement against its guarded parent. The committed tip remains
        // untouched throughout chooser and preview interaction.
        let correction = self
            .current_last_push_pull()
            .filter(|operation| operation.selection == selection);
        let planning = correction
            .and_then(|_| self.document.tip_replacement_parent().ok())
            .map_or(SmartPushPullPlanning::Append, |parent| {
                SmartPushPullPlanning::TipReplacement(parent)
            });
        PushPullTool.cancel(self);
        self.push_pull.distance_input = self.value_box.input.clone();
        // A shape on a part whose exact solid is still being evaluated parks
        // the value below for the frame that publishes the solid, as a drag
        // does: waiting here froze the window for the whole evaluation.
        if self.start_preview_for(planning) && self.confirm_push_pull_preview() {
            self.digest = self.catalog.format(
                "digest-exact-value-applied",
                &BTreeMap::from([(
                    "value",
                    parse_distance_mm(&self.value_box.input, self.value_box.decimal)
                        .map_or_else(String::new, format_signed_mm),
                )]),
            );
            return true;
        }
        if self.face_offset_failed() {
            // The status line already names why the exact offset failed.
            return false;
        }
        self.await_program_if_planning();
        // The failed preview already named why nothing was applied.
        false
    }

    fn apply_typed_move(&mut self) -> bool {
        let value = self.value_box.input.trim();
        if is_move_copy_array(value) {
            let limit = MAX_CAD_GENERATED_OCCURRENCES.to_string();
            let Some((mode, count)) = parse_move_copy_array(value, self.value_box.decimal) else {
                self.digest = self.catalog.format(
                    "digest-copy-array-invalid",
                    &BTreeMap::from([("limit", limit)]),
                );
                return false;
            };
            // The limit counts every new part, so ×N repeats each selected part N times.
            if let Some((_, previous)) = self.current_move_copy_correction() {
                let copies = count.saturating_mul(previous.source_occurrence_ids.len());
                if copies > MAX_CAD_GENERATED_OCCURRENCES {
                    self.digest = self.catalog.format(
                        "digest-copy-array-limit-exceeded",
                        &BTreeMap::from([("copies", copies.to_string()), ("limit", limit)]),
                    );
                    return false;
                }
            }
            if self.apply_current_move_copy_array(mode, count) {
                return true;
            }
            self.digest = self.catalog.text("digest-copy-array-unavailable");
            return false;
        }
        match self.typed_move() {
            Some(TypedMove::AlongAxis { delta_mm, .. }) => {
                self.apply_typed_move_vector(delta_mm, false)
            }
            Some(TypedMove::Vector(delta_mm)) => self.apply_typed_move_vector(delta_mm, true),
            Some(TypedMove::Distance(distance_mm)) => self.apply_typed_move_distance(distance_mm),
            None => {
                self.digest = self.catalog.text("digest-nothing-to-apply");
                false
            }
        }
    }

    /// What the text in the Move field does, read once for both the move and
    /// the preview above the field.
    pub(crate) fn typed_move(&self) -> Option<TypedMove> {
        let input = &self.value_box.input;
        // A pinned axis turns a plain number into travel along that axis,
        // which is how a part gets set down exactly 25 mm higher.
        let axis_travel = self.gesture.transform.move_axis_lock.and_then(|axis| {
            let distance_mm = parse_distance_mm(input, self.value_box.decimal)?;
            (distance_mm.abs() >= 0.01).then(|| TypedMove::AlongAxis {
                delta_mm: axis_direction(axis) * distance_mm,
                distance_mm,
            })
        });
        axis_travel
            .or_else(|| parse_move_vector(input, self.value_box.decimal).map(TypedMove::Vector))
            .or_else(|| parse_distance_mm(input, self.value_box.decimal).map(TypedMove::Distance))
    }

    /// Moves by `delta_mm`; `exact_vector` when it was typed as a vector, which
    /// a later value may correct as a vector too.
    fn apply_typed_move_vector(&mut self, delta_mm: Vec3, exact_vector: bool) -> bool {
        let applied = format_vector_mm(delta_mm);
        // A gesture in flight already knows its target and its copy mode;
        // typing a value replaces the one the pointer is showing.
        if length(delta_mm) > 0.0
            && let Some(mut drag) = self.take_move_session(None)
        {
            drag.delta_mm = delta_mm;
            if self.commit_move_drag(&drag) {
                self.set_move_vector_correction_enabled(exact_vector);
                return self.move_applied(applied);
            }
            return false;
        }
        if self.current_move_copy_correction().is_some() {
            if self.correct_move_copy_delta(delta_mm) {
                return self.move_applied(applied);
            }
            self.end_transform_correction();
        }
        let previous = self
            .current_move_correction()
            .filter(|(_, operation)| operation.accepts_vector_correction);
        if self.transform_tool.correction.is_some() && previous.is_none() {
            self.end_transform_correction();
        }
        if let Some((selection, previous)) = previous {
            if self.correct_move_delta(selection, previous, delta_mm, true) {
                return self.move_applied(applied);
            }
            self.end_transform_correction();
            return false;
        }
        if self.move_selected(delta_mm) {
            self.set_move_vector_correction_enabled(exact_vector);
            return self.move_applied(applied);
        }
        self.digest = self.catalog.text("digest-nothing-to-apply");
        false
    }

    /// Moves `distance_mm` the way the gesture points, or corrects the last
    /// Move to that distance along its own way.
    fn apply_typed_move_distance(&mut self, distance_mm: f64) -> bool {
        let applied = format_signed_mm(distance_mm);
        // A gesture in flight travels the typed distance the way the pointer
        // shows; before the pointer shows a way there is none.
        if let Some((drag, _)) = self.move_session() {
            let shown_mm = length(drag.delta_mm);
            if shown_mm < 0.01 || distance_mm.abs() < 0.01 {
                self.digest = self.catalog.text("digest-move-direction-missing");
                return false;
            }
            let Some(mut drag) = self.take_move_session(None) else {
                return false;
            };
            drag.delta_mm = drag.delta_mm * (distance_mm / shown_mm);
            return self.commit_move_drag(&drag) && self.move_applied(applied);
        }
        if let Some((_, previous)) = self.current_move_copy_correction() {
            let previous_distance_mm = length(previous.delta_mm);
            if previous_distance_mm > 0.0
                && self.correct_move_copy_delta(
                    previous.delta_mm * (distance_mm / previous_distance_mm),
                )
            {
                return self.move_applied(applied);
            }
            self.end_transform_correction();
        }
        let previous = self.current_move_correction();
        if self.transform_tool.correction.is_some() && previous.is_none() {
            self.end_transform_correction();
        }
        if let Some((selection, previous)) = previous {
            let correction_mm = distance_mm - previous.applied_distance_mm;
            if correction_mm.abs() < 0.01
                || self.correct_move_delta(
                    selection,
                    previous.clone(),
                    previous.direction * distance_mm,
                    false,
                )
            {
                return self.move_applied(applied);
            }
            self.end_transform_correction();
            return false;
        }
        if self.move_selected(Vec3::new(distance_mm, 0.0, 0.0)) {
            return self.move_applied(applied);
        }
        self.digest = self.catalog.text("digest-nothing-to-apply");
        false
    }

    fn move_applied(&mut self, value: String) -> bool {
        self.digest = self.catalog.format(
            "digest-exact-move-applied",
            &BTreeMap::from([("value", value)]),
        );
        true
    }

    /// A typed value for a tool without a gesture: Hole and Draft.
    fn apply_typed_without_gesture(&mut self) -> bool {
        match self.active_tool {
            ActiveTool::Hole => {
                // The size is typed first; the hole lands where the face is clicked.
                self.digest = self.catalog.text("status-hole-face");
                false
            }
            ActiveTool::Draft => {
                // The angle is typed first; the draft is sent when the neutral face is clicked.
                self.digest = self.catalog.text("status-draft-neutral");
                false
            }
            _ => {
                self.digest = self.catalog.text("digest-nothing-to-apply");
                false
            }
        }
    }
}

const MULTIPLY_MARKS: [char; 4] = ['x', 'X', '*', '×'];

/// Whether typed Move text is a copy array count: `x5`, `5x`, `/4` or `4/`.
pub(crate) fn is_move_copy_array(input: &str) -> bool {
    let marks = ['x', 'X', '*', '×', '/'];
    let trimmed = input.trim();
    trimmed.starts_with(marks) || trimmed.ends_with(marks)
}

fn parse_move_copy_array(
    input: &str,
    decimal: DecimalSeparator,
) -> Option<(MoveCopyArrayMode, usize)> {
    let trimmed = input.trim();
    // Before or after the count, as SketchUp takes both: `x5` and `5x`.
    let marked = |marks: &[char]| {
        trimmed
            .strip_prefix(marks)
            .or_else(|| trimmed.strip_suffix(marks))
    };
    let (mode, count) = if let Some(count) = marked(&MULTIPLY_MARKS) {
        (MoveCopyArrayMode::Multiply, count)
    } else {
        (MoveCopyArrayMode::Divide, marked(&['/'])?)
    };
    let count = crate::length_input::read_whole::<usize>(count, decimal).ok()?;
    // The limit is checked against the number of new parts, which the
    // caller knows.
    (count >= 1).then_some((mode, count))
}

/// A typed Move value: a distance along the pinned axis, a vector, or a
/// distance the way the gesture points.
pub(crate) enum TypedMove {
    AlongAxis { delta_mm: Vec3, distance_mm: f64 },
    Vector(Vec3),
    Distance(f64),
}

/// A typed Move vector: two or three lengths separated like any values of
/// the window (`12,5;0`, `30x40x0`, `2:5`). One value is a distance, never a
/// vector.
fn parse_move_vector(input: &str, decimal: DecimalSeparator) -> Option<Vec3> {
    let values = crate::length_input::parse_length_list(input, decimal, 2..=3)?;
    let vector = match values.as_slice() {
        [x, y] => Vec3::new(*x, *y, 0.0),
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => return None,
    };
    (vector.x.is_finite() && vector.y.is_finite() && vector.z.is_finite()).then_some(vector)
}
