//! Moving and turning a selected underlay with the Move and Rotate tools: the gesture is the
//! model's, and its edit goes into the underlay's placement as a fit's does, one Undo step.

use super::transform::{move_world_edit, rotate_world_edit};
use super::underlay_edit::UnderlayEditError;
use super::underlay_snapping::{underlay_of, underlay_reference};
use crate::transform_operation::TransformTarget;
use crate::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::drawn::in_lines;
use ketchup_model::underlay::edits::UnderlayObjectError;
use ketchup_model::underlay::objects::Affine;
use ketchup_model::underlay::placement::{UnderlayFitError, plan_underlay_edit};

impl KetchupApp {
    /// The underlay selected alone, while the document still has it.
    pub(crate) fn selected_underlay(&self) -> Option<UnderlayId> {
        let id = self.selection.underlay()?;
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

    /// Selects the whole drawing of underlay `id`, no object picked in it, so Move and
    /// Rotate place the drawing.
    pub(crate) fn select_whole_underlay(&mut self, id: UnderlayId) {
        self.select_underlay(id);
        if self.selected_underlay() == Some(id) {
            self.digest = self.catalog.text("digest-underlay-whole-selected");
        }
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
        let snapshot = self.document.current();
        if snapshot
            .underlay(id)
            .is_some_and(|underlay| !underlay.lies_flat())
        {
            self.refuse_underlay_edit(self.catalog.text("error-underlay-standing"));
            return true;
        }
        let Err(error) = plan_underlay_edit(&snapshot, id, edit) else {
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
        let copy = copy || self.gesture.transform.move_copy;
        // Picked objects of the drawing are copied; the drawing itself is only placed.
        let objects = self.picked_underlay_objects().is_some();
        if !objects && self.underlay_copy_refused(copy) {
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
                copy: objects && copy,
            },
        );
        true
    }

    /// The underlay a live move or turn drags, and its edit so far: the viewport draws it
    /// there before the edit is committed. With objects picked in it the drag is theirs,
    /// and the drawing stays.
    pub(crate) fn underlay_drag_edit(&self) -> Option<(UnderlayId, Transform)> {
        if self.picked_underlay_objects().is_some() {
            return None;
        }
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

    /// The edit so far of a live move or turn, or copy, of the objects picked in a
    /// drawing: the viewport draws them there before the edit is committed.
    pub(crate) fn picked_objects_drag_edit(&self) -> Option<Transform> {
        match self.transform_tool.session.as_ref()? {
            ToolSession::Move { drag, .. } => {
                self.dragged_underlay_objects(&drag.selection)?;
                move_world_edit(drag)
            }
            ToolSession::Rotate { drag, .. } => {
                self.dragged_underlay_objects(&drag.selection)?;
                rotate_world_edit(drag)
            }
            ToolSession::Scale { .. } => None,
        }
    }

    /// The objects picked in the drawing `selection` names: a move or turn of it edits
    /// them, not where the drawing lies.
    pub(crate) fn dragged_underlay_objects(
        &self,
        selection: &SelectionId,
    ) -> Option<(UnderlayId, Vec<u32>)> {
        let id = underlay_of(selection)?;
        self.picked_underlay_objects()
            .filter(|(picked, _)| *picked == id)
    }

    /// Moves or turns the objects `ids` of the drawing of `underlay` by the document edit
    /// `edit`, or adds copies of them so placed when `copy`, as one Undo step, and keeps
    /// them picked.
    pub(crate) fn commit_underlay_objects_edit(
        &mut self,
        underlay: UnderlayId,
        ids: &[u32],
        edit: Option<Transform>,
        copy: bool,
    ) -> bool {
        let map = edit.and_then(|edit| {
            let found = self.document.current().underlay(underlay)?.clone();
            in_lines(&plane_part(edit), &found.plane_map())
        });
        let result = match map {
            Some(map) => self.transform_underlay_objects(underlay, ids, &map, copy),
            None => Err(UnderlayEditError::Plan(UnderlayObjectError::InvalidMap)),
        };
        match result {
            Ok(()) => {
                self.select_underlay(underlay);
                for id in ids {
                    self.selection.pick_underlay_object(*id, true);
                }
                self.status_key = "status-object-moved";
                self.digest = self.catalog.format(
                    if copy {
                        "digest-underlay-objects-copied"
                    } else {
                        "digest-underlay-objects-moved"
                    },
                    &BTreeMap::from([("count", ids.len().to_string())]),
                );
                true
            }
            Err(error) => {
                self.refuse_underlay_edit(error.to_string());
                false
            }
        }
    }
}

/// The map of the ground plane a document edit that keeps it makes.
fn plane_part(edit: Transform) -> Affine {
    let matrix = edit.matrix();
    [
        [matrix[0], matrix[1], matrix[3]],
        [matrix[4], matrix[5], matrix[7]],
    ]
}
