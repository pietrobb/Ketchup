//! Moving and turning a selected underlay with the Move and Rotate tools: the gesture is the
//! model's, and its edit goes into the underlay's placement as a fit's does, one Undo step.

use super::underlay_snapping::{underlay_of, underlay_reference};
use crate::transform_operation::TransformTarget;
use crate::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::placement::{UnderlayFitError, plan_underlay_edit};

impl KetchupApp {
    /// The underlay selected alone, while the document still has it.
    pub(crate) fn selected_underlay(&self) -> Option<UnderlayId> {
        let id = self.selection.underlay?;
        (self.selection.occurrences.is_empty()
            && self.selection.primary.is_none()
            && self.selection.selected_group.is_none()
            && self.document.current().underlay(id).is_some())
        .then_some(id)
    }

    /// Selects the underlay `id` alone, so Move and Rotate place it.
    pub(crate) fn select_underlay(&mut self, id: UnderlayId) {
        let Some(name) = self
            .document
            .current()
            .underlay(id)
            .map(|underlay| underlay.name.clone())
        else {
            return;
        };
        self.end_transform_correction();
        self.selection.select_underlay(id);
        self.digest = self.catalog.format(
            "digest-underlay-selected",
            &BTreeMap::from([("name", name)]),
        );
    }

    fn refuse_underlay_edit(&mut self, reason: String) {
        self.digest = self.catalog.format(
            "digest-underlay-edit-failed",
            &BTreeMap::from([("reason", reason)]),
        );
    }

    /// Refuses, in the status line, a move or turn of an underlay that would leave the
    /// ground plane.
    pub(crate) fn underlay_drag_refused(
        &mut self,
        selection: &SelectionId,
        off_plane: bool,
    ) -> bool {
        let refused = off_plane && underlay_of(selection).is_some();
        if refused {
            self.refuse_underlay_edit(UnderlayFitError::OffPlane.to_string());
        }
        refused
    }

    /// Refuses, in the status line and by its own reason, a move or turn by `edit` of the
    /// underlay `selection` names that its placement does not take, such as one past the
    /// model's range: the plan of it is none, which the commit would call a stale preview
    /// (review 2026-10-24 UI-24).
    pub(crate) fn underlay_edit_refused(
        &mut self,
        selection: &SelectionId,
        edit: Option<Transform>,
    ) -> bool {
        let (Some(id), Some(edit)) = (underlay_of(selection), edit) else {
            return false;
        };
        let Err(error) = plan_underlay_edit(&self.document.current(), id, edit) else {
            return false;
        };
        self.refuse_underlay_edit(error.to_string());
        true
    }

    /// Refuses, in the status line, a copy of an underlay: it is placed, not copied.
    pub(crate) fn underlay_copy_refused(&mut self, copy: bool) -> bool {
        if copy {
            self.refuse_underlay_edit(self.catalog.text("error-underlay-copy"));
        }
        copy
    }

    /// Starts moving the underlay `id` from the snapped point under the pointer, or from
    /// where the pointer meets the ground plane.
    pub(crate) fn begin_underlay_move(
        &mut self,
        id: UnderlayId,
        pointer: Pos2,
        rect: Rect,
        copy: bool,
    ) -> bool {
        if self.underlay_copy_refused(copy || self.gesture.transform.move_copy) {
            return false;
        }
        let start = self
            .hover
            .snap
            .as_ref()
            .filter(|snap| snap.kind != SnapKind::Face)
            .map(|snap| snap.position_mm)
            .or_else(|| self.screen_to_plane(pointer, rect, 0.0));
        let Some(start) = start else {
            self.digest = self.catalog.text("digest-move-start-missed");
            return false;
        };
        let axis = self.gesture.transform.move_axis_lock;
        let axis_reference =
            axis.and_then(|axis| axis_travel_along(&self.view_ray(pointer, rect)?, start, axis));
        let snapshot = self.document.current();
        self.value_box.input = "0".to_owned();
        self.set_move_session(
            ToolSessionPhase::Gesture,
            MoveDrag {
                source_document_id: snapshot.document_id(),
                source_revision: snapshot.revision_id(),
                selection: underlay_reference(id, 0),
                occurrence_paths: BTreeSet::new(),
                group_id: None,
                profile_target: None,
                pointer_start_world: start,
                plane_z: start.z,
                axis,
                axis_reference,
                delta_mm: Vec3::ZERO,
                copy: false,
            },
        );
        true
    }

    /// The underlay a live move or turn drags, and its edit so far: the viewport draws it
    /// there before the edit is committed.
    pub(crate) fn underlay_drag_edit(&self) -> Option<(UnderlayId, Transform)> {
        let plan = match self.transform_tool.session.as_ref()? {
            ToolSession::Move { drag, .. } => self.move_transform_plan(drag),
            ToolSession::Rotate { drag, .. } => self.rotate_transform_plan(drag),
            ToolSession::Scale { .. } => None,
        }?;
        match plan.target() {
            TransformTarget::Underlay(id) => Some((*id, plan.world_edit())),
            _ => None,
        }
    }
}
