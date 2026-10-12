//! The dock's spaces and clearance volumes: listed, outlined in the viewport
//! while picked, deleted by the typed operations MCP uses.

use crate::*;
use ketchup_assistant::sidecar::AssistantCadEditOperation;
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::space::{ClearanceSeverity, ClearanceVolumeId, SpaceId};
use ketchup_rejection::{Rejection, RejectionPhase};

/// A row of the spaces section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpaceRow {
    Space(SpaceId),
    Clearance(ClearanceVolumeId),
}

fn dimensions(volume: Aabb) -> String {
    let [x, y, z] = volume.extents();
    format!(
        "{} × {} × {}",
        format_height(x),
        format_height(y),
        format_height(z)
    )
}

impl KetchupApp {
    /// The picked row and its box, while no part or group is selected and the
    /// row still exists: a part picked later takes over Delete and the outline.
    pub(crate) fn picked_space_row(&self) -> Option<(SpaceRow, Aabb)> {
        let row = self.picked_space?;
        if !self.selected_instance_paths().is_empty() || self.selection.selected_group.is_some() {
            return None;
        }
        let snapshot = self.document.current();
        let volume = match row {
            SpaceRow::Space(id) => snapshot.spaces().find(|space| space.id() == id)?.volume(),
            SpaceRow::Clearance(id) => snapshot
                .clearance_volumes()
                .find(|volume| volume.id() == id)?
                .volume(),
        };
        Some((row, volume))
    }

    pub(crate) fn show_spaces_section(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        let severity = |severity| match severity {
            ClearanceSeverity::Required => "spaces-severity-required",
            ClearanceSeverity::Advisory => "spaces-severity-advisory",
        };
        let rows: Vec<_> = snapshot
            .spaces()
            .map(|space| {
                let label = self.catalog.format(
                    "spaces-space-row",
                    &BTreeMap::from([
                        ("id", space.id().0.to_string()),
                        ("name", space.purpose().to_owned()),
                        ("dimensions", dimensions(space.volume())),
                    ]),
                );
                (SpaceRow::Space(space.id()), label)
            })
            .chain(snapshot.clearance_volumes().map(|volume| {
                let label = self.catalog.format(
                    "spaces-clearance-row",
                    &BTreeMap::from([
                        ("id", volume.id().0.to_string()),
                        ("severity", self.catalog.text(severity(volume.severity()))),
                        ("name", volume.reason().to_owned()),
                        ("dimensions", dimensions(volume.volume())),
                    ]),
                );
                (SpaceRow::Clearance(volume.id()), label)
            }))
            .collect();
        if rows.is_empty() {
            return;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-spaces"));
        let picked = self.picked_space_row().map(|(row, _)| row);
        for (row, label) in rows {
            if ui.selectable_label(picked == Some(row), label).clicked() {
                self.clear_selection();
                self.picked_space = Some(row);
            }
        }
    }

    /// Deletes the picked row with `delete_space` or `delete_clearance_volume`,
    /// planned and committed as MCP `edit propose` + `commit` do: one Undo step,
    /// or the rejection in the status line and the document unchanged.
    pub(crate) fn delete_picked_space_row(&mut self, row: SpaceRow) -> bool {
        let operation = match row {
            SpaceRow::Space(id) => AssistantCadEditOperation::DeleteSpace { space_id: id.0 },
            SpaceRow::Clearance(id) => AssistantCadEditOperation::DeleteClearanceVolume {
                clearance_volume_id: id.0,
            },
        };
        let program = AssistantCadEditProgram {
            operations: vec![operation],
        };
        let committed = self
            .derive_assistant_cad_edit_proposal(&program)
            .map_err(|diagnostic| Rejection::from(*diagnostic))
            .and_then(|proposal| {
                self.commit_verified_proposal_with_work_recovery(&proposal)
                    .map_err(|error| {
                        Rejection::new("commit_rejected", RejectionPhase::Commit).caused_by(error)
                    })
            });
        match committed {
            Ok(_) => {
                self.picked_space = None;
                true
            }
            Err(rejection) => {
                self.digest = self.catalog.format(
                    "spaces-delete-rejected",
                    &BTreeMap::from([("reason", rejection.to_string())]),
                );
                false
            }
        }
    }

    /// The twelve edges of the picked row's box, over the bodies like a
    /// selection.
    pub(crate) fn paint_picked_space(&self, painter: &egui::Painter, rect: Rect) {
        let Some((_, volume)) = self.picked_space_row() else {
            return;
        };
        let (min, max) = (volume.min(), volume.max());
        let corner = |index: usize| {
            Vec3::new(
                if index & 1 == 0 { min[0] } else { max[0] },
                if index & 2 == 0 { min[1] } else { max[1] },
                if index & 4 == 0 { min[2] } else { max[2] },
            )
        };
        let stroke = Stroke::new(1.8_f32, Color32::from_rgb(240, 78, 35));
        for start in 0..8 {
            for bit in [1, 2, 4] {
                if start & bit == 0 {
                    let points =
                        [start, start | bit].map(|index| self.project(corner(index), rect));
                    paint_dashed_segment(painter, points, stroke);
                }
            }
        }
    }
}
