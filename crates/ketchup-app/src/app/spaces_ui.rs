//! The dock's spaces and clearance volumes: listed, outlined in the viewport
//! while picked in the dock or by a validator finding, created from the
//! selection, edited by number and deleted, every change by the typed
//! operations MCP uses.

use crate::*;
use ketchup_application::model_query::assistant_instance_path;
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantClearanceOwner, AssistantClearanceSeverity,
};
use ketchup_geometry::prismatic::Aabb;
use ketchup_model::space::{ClearanceOwner, ClearanceSeverity, ClearanceVolumeId, SpaceId};
use ketchup_rejection::{Rejection, RejectionPhase};

/// Depth of a new clearance volume in front of a part, toward -Y.
const FRONT_CLEARANCE_MM: f64 = 600.0;

/// A row of the spaces section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpaceRow {
    Space(SpaceId),
    Clearance(ClearanceVolumeId),
}

/// The picked row's values as typed, read again whenever the row or the
/// document revision changes.
#[derive(Default)]
pub(crate) struct SpaceFields {
    shown: Option<(SpaceRow, u64)>,
    name: String,
    position: [String; 3],
    size: [String; 3],
    required: bool,
    /// The row was just picked and its fields are not yet in view.
    reveal: bool,
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
    /// The picked row and its box, while the part selection is the one it was
    /// picked with, no group is selected and the row still exists: a part
    /// picked later takes over the outline.
    pub(crate) fn picked_space_row(&self) -> Option<(SpaceRow, Aabb)> {
        let (row, with) = self.picked_space.as_ref()?;
        if *with != self.selected_instance_paths() || self.selection.selected_group.is_some() {
            return None;
        }
        let row = *row;
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
        let selected = self.selected_instance_paths();
        if rows.is_empty() && selected.is_empty() {
            return;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-spaces"));
        ui.horizontal_wrapped(|ui| {
            let from_selection = self.catalog.text("spaces-create-from-selection");
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new(from_selection))
                .clicked()
            {
                self.create_space_from_selection();
            }
            let in_front = self.catalog.text("spaces-create-clearance-in-front");
            if ui
                .add_enabled(selected.len() == 1, egui::Button::new(in_front))
                .clicked()
            {
                self.create_clearance_in_front();
            }
        });
        let picked = self.picked_space_row().map(|(row, _)| row);
        for (row, label) in rows {
            if ui.selectable_label(picked == Some(row), label).clicked() {
                self.pick_space_row(row);
            }
        }
        if let Some(row) = self.picked_space_row().map(|(row, _)| row) {
            self.show_space_fields(ui, row);
        }
    }

    fn show_space_fields(&mut self, ui: &mut egui::Ui, row: SpaceRow) {
        let snapshot = self.document.current();
        let shown = Some((row, snapshot.revision_id()));
        let newly_picked = self.space_fields.shown.map(|(shown, _)| shown) != Some(row);
        if self.space_fields.shown != shown {
            let (name, volume, required) = match row {
                SpaceRow::Space(id) => {
                    let space = snapshot.spaces().find(|space| space.id() == id);
                    let space = space.expect("the picked row exists");
                    (space.purpose().to_owned(), space.volume(), false)
                }
                SpaceRow::Clearance(id) => {
                    let volume = snapshot
                        .clearance_volumes()
                        .find(|volume| volume.id() == id);
                    let volume = volume.expect("the picked row exists");
                    let required = volume.severity() == ClearanceSeverity::Required;
                    (volume.reason().to_owned(), volume.volume(), required)
                }
            };
            self.space_fields = SpaceFields {
                shown,
                name,
                position: volume.min().map(format_height),
                size: volume.extents().map(format_height),
                required,
                reveal: newly_picked,
            };
        }
        let fields = &mut self.space_fields;
        let field = |ui: &mut egui::Ui, label: String, value: &mut String| {
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.add(egui::TextEdit::singleline(value).desired_width(90.0))
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                    });
            });
        };
        let name_key = match row {
            SpaceRow::Space(_) => "spaces-field-purpose",
            SpaceRow::Clearance(_) => "spaces-field-reason",
        };
        field(ui, self.catalog.text(name_key), &mut fields.name);
        for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let arguments = BTreeMap::from([("axis", axis.to_owned())]);
            let position = self.catalog.format("spaces-field-position", &arguments);
            field(ui, position, &mut fields.position[index]);
            let size = self.catalog.format("spaces-field-size", &arguments);
            field(ui, size, &mut fields.size[index]);
        }
        if let SpaceRow::Clearance(_) = row {
            let required = self.catalog.text("spaces-field-required");
            ui.checkbox(&mut fields.required, required);
        }
        let parsed = |values: &[String; 3]| {
            let [x, y, z] = values
                .each_ref()
                .map(|value| parse_distance_mm(value, self.value_box.decimal));
            Some([x?, y?, z?])
        };
        let boxed = parsed(&fields.position)
            .zip(parsed(&fields.size))
            .map(|(min, size)| (min, std::array::from_fn(|axis| min[axis] + size[axis])));
        let apply = ui.add_enabled(
            boxed.is_some(),
            egui::Button::new(self.catalog.text("spaces-apply")),
        );
        // A row picked from a finding or the list may have its fields below
        // the fold of the dock, where a click on Apply never lands. The dock
        // settles over a few frames, so it scrolls until Apply is in view.
        if fields.reveal {
            if ui.clip_rect().contains_rect(apply.rect) {
                fields.reveal = false;
            } else {
                let instant = egui::style::ScrollAnimation::none();
                ui.scroll_to_rect_animation(apply.rect, None, instant);
            }
        }
        if let (true, Some((min_mm, max_mm))) = (apply.clicked(), boxed) {
            let name = fields.name.clone();
            let severity = if fields.required {
                AssistantClearanceSeverity::Required
            } else {
                AssistantClearanceSeverity::Advisory
            };
            let operation = match row {
                SpaceRow::Space(id) => {
                    let space = snapshot.spaces().find(|space| space.id() == id);
                    let space = space.expect("the picked row exists");
                    let ids = |ids: &[SpaceId]| ids.iter().map(|id| id.0).collect();
                    AssistantCadEditOperation::UpsertSpace {
                        space_id: id.0,
                        purpose: name,
                        min_mm,
                        max_mm,
                        adjacent_to: ids(space.adjacent_to()),
                        accessible_to: ids(space.accessible_to()),
                    }
                }
                SpaceRow::Clearance(id) => {
                    let volume = snapshot
                        .clearance_volumes()
                        .find(|volume| volume.id() == id);
                    let volume = volume.expect("the picked row exists");
                    let owner = match volume.owner() {
                        ClearanceOwner::Space(space) => {
                            AssistantClearanceOwner::Space { space_id: space.0 }
                        }
                        ClearanceOwner::Occurrence(path) => AssistantClearanceOwner::Occurrence {
                            instance_path: assistant_instance_path(&snapshot, path)
                                .expect("a stored owner resolves"),
                        },
                    };
                    AssistantCadEditOperation::UpsertClearanceVolume {
                        clearance_volume_id: id.0,
                        reason: name,
                        owner,
                        min_mm,
                        max_mm,
                        severity,
                        tolerance_mm: Some(volume.tolerance().linear_mm()),
                    }
                }
            };
            self.commit_dock_operation(operation);
        }
    }

    /// A new space around the selected parts' world box.
    fn create_space_from_selection(&mut self) {
        let Some(measures) = self.selection_measures() else {
            return;
        };
        let snapshot = self.document.current();
        let id = snapshot
            .spaces()
            .map(|space| space.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let min_mm = measures.world_min_mm;
        let operation = AssistantCadEditOperation::UpsertSpace {
            space_id: id,
            purpose: self.catalog.text("spaces-new-space"),
            min_mm,
            max_mm: std::array::from_fn(|axis| min_mm[axis] + measures.world_size_mm[axis]),
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        };
        if self.commit_dock_operation(operation) {
            self.pick_space_row(SpaceRow::Space(SpaceId(id)));
        }
    }

    /// A required clearance volume owned by the one selected part, as wide
    /// and high as the part and [`FRONT_CLEARANCE_MM`] deep in front of it.
    fn create_clearance_in_front(&mut self) {
        let paths = self.selected_instance_paths();
        let (1, Some(path)) = (paths.len(), paths.first()) else {
            return;
        };
        let snapshot = self.document.current();
        let (Some(measures), Some(instance_path)) = (
            self.selection_measures(),
            assistant_instance_path(&snapshot, path),
        ) else {
            return;
        };
        let id = snapshot
            .clearance_volumes()
            .map(|volume| volume.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let [x, y, z] = measures.world_min_mm;
        let [width, _, height] = measures.world_size_mm;
        let operation = AssistantCadEditOperation::UpsertClearanceVolume {
            clearance_volume_id: id,
            reason: self.catalog.text("spaces-new-clearance"),
            owner: AssistantClearanceOwner::Occurrence { instance_path },
            min_mm: [x, y - FRONT_CLEARANCE_MM, z],
            max_mm: [x + width, y, z + height],
            severity: AssistantClearanceSeverity::Required,
            tolerance_mm: None,
        };
        if self.commit_dock_operation(operation) {
            self.pick_space_row(SpaceRow::Clearance(ClearanceVolumeId(id)));
        }
    }

    fn pick_space_row(&mut self, row: SpaceRow) {
        self.clear_selection();
        self.picked_dimension = None;
        self.picked_joint = None;
        self.picked_space = Some((row, BTreeSet::new()));
    }

    /// Selects the parts a validator finding names and outlines the
    /// clearance volume it is about, side by side while that selection stands.
    pub(crate) fn show_validator_finding(&mut self, finding: &ValidatorPanelFinding) {
        self.clear_selection();
        self.selection.occurrences = finding
            .occurrences
            .iter()
            .map(|id| InstancePath::root(*id))
            .collect();
        self.picked_space = finding
            .clearance_volume
            .map(|id| (SpaceRow::Clearance(id), self.selected_instance_paths()));
    }

    /// Deletes the picked row with `delete_space` or `delete_clearance_volume`.
    pub(crate) fn delete_picked_space_row(&mut self, row: SpaceRow) -> bool {
        let operation = match row {
            SpaceRow::Space(id) => AssistantCadEditOperation::DeleteSpace { space_id: id.0 },
            SpaceRow::Clearance(id) => AssistantCadEditOperation::DeleteClearanceVolume {
                clearance_volume_id: id.0,
            },
        };
        let deleted = self.commit_dock_operation(operation);
        if deleted {
            self.picked_space = None;
        }
        deleted
    }

    /// Plans and commits one operation as MCP `edit propose` + `commit` do:
    /// one Undo step, or the rejection in the status line and the document
    /// unchanged.
    pub(crate) fn commit_dock_operation(&mut self, operation: AssistantCadEditOperation) -> bool {
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
        if let Err(rejection) = &committed {
            self.digest = self.catalog.format(
                "spaces-rejected",
                &BTreeMap::from([("reason", rejection.to_string())]),
            );
        }
        committed.is_ok()
    }

    /// The twelve edges of the picked row's box, over the bodies like a
    /// selection.
    pub(crate) fn paint_picked_space(&self, painter: &egui::Painter, rect: Rect) {
        let Some((_, volume)) = self.picked_space_row() else {
            return;
        };
        let stroke = Stroke::new(1.8_f32, Color32::from_rgb(240, 78, 35));
        self.paint_dashed_box(painter, rect, volume.min(), volume.max(), stroke);
    }

    pub(crate) fn paint_dashed_box(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        min: [f64; 3],
        max: [f64; 3],
        stroke: Stroke,
    ) {
        let corner = |index: usize| {
            Vec3::new(
                if index & 1 == 0 { min[0] } else { max[0] },
                if index & 2 == 0 { min[1] } else { max[1] },
                if index & 4 == 0 { min[2] } else { max[2] },
            )
        };
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
