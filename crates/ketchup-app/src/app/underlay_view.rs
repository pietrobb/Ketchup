//! Tools > Drawing view: each imported drawing laid in the plan or stood up as a front,
//! back, left or right view, one Undo step, as MCP `drawing action=view` does. A drawing
//! that stands is no longer picked in the window, so it is reached here by its name.

use crate::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::view::{UnderlayView, ViewPlane, plan_underlay_view};

const fn plane_key(plane: ViewPlane) -> &'static str {
    match plane {
        ViewPlane::Top => "underlay-view-top",
        ViewPlane::Front => "underlay-view-front",
        ViewPlane::Back => "underlay-view-back",
        ViewPlane::Left => "underlay-view-left",
        ViewPlane::Right => "underlay-view-right",
    }
}

impl KetchupApp {
    /// The submenu of Tools: per drawing its name and the planes it can stand on.
    pub(crate) fn underlay_view_menu(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        let rows = snapshot
            .underlays()
            .map(|underlay| (underlay.id, underlay.name.clone(), underlay.view.plane))
            .collect::<Vec<_>>();
        ui.add_enabled_ui(!rows.is_empty(), |ui| {
            ui.menu_button(self.catalog.text("menu-underlay-view"), |ui| {
                for (id, name, current) in rows {
                    ui.label(name);
                    ui.horizontal(|ui| {
                        for plane in ViewPlane::ALL {
                            let label = self.catalog.text(plane_key(plane));
                            if ui.selectable_label(current == plane, label).clicked()
                                && current != plane
                            {
                                self.set_underlay_view(id, plane);
                            }
                        }
                    });
                }
            });
        });
    }

    /// Sets the drawing `id` on `plane` through the origin, one Undo step; a program part
    /// bound to it follows in the same step. True also while it waits for the regions of
    /// such parts.
    pub(crate) fn set_underlay_view(&mut self, id: UnderlayId, plane: ViewPlane) -> bool {
        let snapshot = self.document.current();
        let Some(name) = snapshot.underlay(id).map(|underlay| underlay.name.clone()) else {
            return false;
        };
        let view = UnderlayView { plane, at_mm: 0.0 };
        // The status line is where the error ends: its text is all that is shown of it.
        let failure = match plan_underlay_view(&snapshot, id, view) {
            // Once the regions of the areas that follow it are found off the UI thread
            // (review 2026-10-29 UO-40).
            Ok(batch)
                if !self.followed_regions_or_await(&batch, move |app| {
                    app.set_underlay_view(id, plane);
                }) =>
            {
                return true;
            }
            Ok(batch) => self
                .apply_batch_with_work_recovery(&batch)
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(error.to_string()),
        };
        match failure {
            None => {
                if self.selection.underlay() == Some(id) && plane != ViewPlane::Top {
                    self.selection.clear();
                }
                self.digest = self.catalog.format(
                    "digest-underlay-view",
                    &BTreeMap::from([
                        ("name", name),
                        ("plane", self.catalog.text(plane_key(plane))),
                    ]),
                );
                // A drawing being drawn into stands elsewhere now: drawing into it ends
                // (review 2026-10-29 UI-64).
                if plane != ViewPlane::Top
                    && self
                        .gesture
                        .underlay_draw
                        .take_if(|target| target.underlay == id)
                        .is_some()
                {
                    self.digest.push_str(" · ");
                    self.digest
                        .push_str(&self.catalog.text("digest-underlay-draw-off"));
                }
                true
            }
            Some(reason) => {
                self.digest = self.catalog.format(
                    "digest-underlay-view-failed",
                    &BTreeMap::from([("reason", reason)]),
                );
                false
            }
        }
    }
}
