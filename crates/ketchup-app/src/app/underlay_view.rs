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
    /// bound to it follows in the same step.
    pub(crate) fn set_underlay_view(&mut self, id: UnderlayId, plane: ViewPlane) -> bool {
        let snapshot = self.document.current();
        let Some(name) = snapshot.underlay(id).map(|underlay| underlay.name.clone()) else {
            return false;
        };
        let view = UnderlayView { plane, at_mm: 0.0 };
        let result = plan_underlay_view(&snapshot, id, view)
            .map_err(|error| error.to_string())
            .and_then(|batch| {
                self.apply_batch_with_work_recovery(&batch)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
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
                true
            }
            Err(reason) => {
                self.digest = self.catalog.format(
                    "digest-underlay-view-failed",
                    &BTreeMap::from([("reason", reason)]),
                );
                false
            }
        }
    }
}
