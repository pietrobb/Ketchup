//! The `drawing` tool: 2D CAD on an imported drawing. `remove` takes top-level objects
//! (ids of `inspect action=underlay_objects`, or all that meet a box) out of it as one Undo
//! step; everything that
//! draws, snaps to or asks about the underlay then sees the drawing without them.

use super::*;
use crate::app::underlay_edit::UnderlayObjectRemovalError;
use ketchup_model::underlay::edits::UnderlayObjectError;
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};

impl LiveBridge {
    /// `drawing_remove`: takes `object_ids` out of the underlay's drawing.
    pub(super) fn drawing_remove(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::DrawingRemove {
            expected,
            underlay_id,
            object_ids,
            within_mm,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let id = underlay.id;
        let object_ids = match (object_ids, within_mm) {
            (Some(ids), None) => ids.clone(),
            (None, Some(area)) if area.iter().flatten().all(|value| value.is_finite()) => {
                let lines = app
                    .cached_underlay_lines(underlay)
                    .ok_or("entity_not_found")?;
                let place = super::underlay_objects::Placed { underlay };
                ObjectIndex::new(&lines).find(&ObjectFilter {
                    within_mm: Some(place.local_area(*area)),
                    top_level: true,
                    ..ObjectFilter::default()
                })
            }
            _ => return Err("invalid_params"),
        };
        if object_ids.is_empty() && within_mm.is_some() {
            return Err("entity_not_found");
        }
        Self::require_request_authority(cancelled)?;
        app.remove_underlay_objects(id, &object_ids)
            .map_err(removal_failure)?;
        self.query.invalidate();
        let snapshot = app.document.current();
        let left = snapshot
            .underlay(id)
            .and_then(|underlay| app.cached_underlay_lines(underlay))
            .map(|lines| ObjectIndex::new(&lines).present());
        let removed = object_ids.iter().collect::<BTreeSet<_>>();
        Ok(json!({
            "removed": removed.len(),
            "object_ids": removed,
            "underlay_id": id.0,
            "objects_left": left,
            "dirty": app.is_dirty(),
        }))
    }
}

/// The rejection of a removal, with the object to remove instead of one an insert or
/// dimension placed. The whole error is kept as the rejection's cause.
fn removal_failure(error: UnderlayObjectRemovalError) -> &'static str {
    let code = match &error {
        UnderlayObjectRemovalError::Commit(..) => "commit_rejected",
        UnderlayObjectRemovalError::Plan(UnderlayObjectError::NotFound(..)) => "entity_not_found",
        UnderlayObjectRemovalError::NoLines | UnderlayObjectRemovalError::Plan(..) => {
            "invalid_params"
        }
    };
    let details = match &error {
        UnderlayObjectRemovalError::Plan(UnderlayObjectError::Placed { object, parent }) => {
            json!({"object_id": object, "remove_instead": parent})
        }
        UnderlayObjectRemovalError::Plan(UnderlayObjectError::NotFound(object)) => {
            json!({"object_id": object})
        }
        _ => json!({}),
    };
    record_rejection(&rejected(code).caused_by(error), details);
    code
}
