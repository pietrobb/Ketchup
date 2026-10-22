//! The dock's dimensions: listed with their live value, the parts the picked
//! one measures outlined in the viewport, created from a length parameter of
//! the selected part, renamed and reformatted, and deleted, every change by
//! the typed operations MCP uses.

use crate::*;
use ketchup_assistant::sidecar::{AssistantCadEditOperation, AssistantDimensionUnit};
use ketchup_model::document::{
    DimensionDisplayUnit, DimensionReferenceHealth, FeatureParameterTarget, ParameterValueType,
    PersistentDimensionTarget,
};

const UNITS: [(DimensionDisplayUnit, AssistantDimensionUnit); 3] = [
    (
        DimensionDisplayUnit::Millimetres,
        AssistantDimensionUnit::Mm,
    ),
    (
        DimensionDisplayUnit::Centimetres,
        AssistantDimensionUnit::Cm,
    ),
    (DimensionDisplayUnit::Inches, AssistantDimensionUnit::In),
];

/// The picked dimension's name and format as typed, read again whenever the
/// dimension or the document revision changes.
#[derive(Default)]
pub(crate) struct DimensionFields {
    shown: Option<(PersistentDimensionId, u64)>,
    name: String,
    unit: Option<DimensionDisplayUnit>,
    decimal_places: String,
    /// The dimension was just picked and its fields are not yet in view.
    reveal: bool,
}

/// A length parameter of the selected part a new dimension can show.
struct Candidate {
    feature_id: FeatureId,
    path: String,
    value_mm: f64,
}

impl KetchupApp {
    /// The picked dimension while the part selection is the one it was
    /// picked with, no group is selected and the dimension still exists.
    pub(crate) fn picked_dimension_id(&self) -> Option<PersistentDimensionId> {
        let (id, with) = self.picked_dimension.as_ref()?;
        if *with != self.selected_instance_paths() || self.selection.selected_group.is_some() {
            return None;
        }
        let snapshot = self.document.current();
        snapshot
            .persistent_dimension(*id)
            .map(|dimension| dimension.id)
    }

    /// The parts holding the feature the dimension measures; none for a
    /// derived output, which no part owns.
    pub(crate) fn dimension_parts(&self, id: PersistentDimensionId) -> BTreeSet<InstancePath> {
        let snapshot = self.document.current();
        let definition =
            snapshot
                .persistent_dimension(id)
                .and_then(|dimension| match &dimension.target {
                    PersistentDimensionTarget::FeatureParameter(target) => snapshot
                        .feature(target.feature_id)
                        .map(|feature| feature.definition_id()),
                    PersistentDimensionTarget::ExactFeatureParameter { definition_id, .. } => {
                        Some(*definition_id)
                    }
                    PersistentDimensionTarget::DerivedOutput(_) => None,
                });
        snapshot
            .occurrences()
            .filter(|occurrence| Some(occurrence.definition_id()) == definition)
            .map(|occurrence| InstancePath::root(occurrence.id()))
            .collect()
    }

    fn dimension_row_label(&self, snapshot: &Snapshot, id: PersistentDimensionId) -> String {
        let dimension = snapshot
            .persistent_dimension(id)
            .expect("a listed dimension");
        let projection = snapshot.project_persistent_dimension(id);
        let value = match projection.as_ref().map(|projection| &projection.health) {
            Some(DimensionReferenceHealth::Ambiguous { .. }) => {
                self.catalog.text("measured-dimension-ambiguous")
            }
            _ => projection
                .and_then(|projection| projection.display_text)
                .unwrap_or_else(|| self.catalog.text("measured-dimension-lost")),
        };
        self.catalog.format(
            "measured-dimension-row",
            &BTreeMap::from([("name", dimension.name.clone()), ("value", value)]),
        )
    }

    /// The length parameters of the one selected part that no dimension
    /// shows yet, without raw sketch coordinates.
    fn dimension_candidates(&self, snapshot: &Snapshot) -> Vec<Candidate> {
        let paths = self.selected_instance_paths();
        let (1, Some(path)) = (paths.len(), paths.first()) else {
            return Vec::new();
        };
        let Some(definition) = path
            .steps()
            .is_empty()
            .then(|| snapshot.occurrence(path.root_occurrence()))
            .flatten()
            .and_then(|occurrence| snapshot.definition(occurrence.definition_id()))
        else {
            return Vec::new();
        };
        let shown: BTreeSet<_> = snapshot
            .persistent_dimensions()
            .filter_map(|dimension| match &dimension.target {
                PersistentDimensionTarget::FeatureParameter(target) => Some(target.clone()),
                _ => None,
            })
            .collect();
        let mut candidates = Vec::new();
        for feature in definition
            .feature_ids()
            .iter()
            .filter_map(|id| snapshot.feature(*id))
        {
            for descriptor in feature.kind().parameter_descriptors() {
                let path = descriptor.path().as_str();
                if descriptor.value_type() != ParameterValueType::Length
                    || path.starts_with("entities.")
                {
                    continue;
                }
                let Ok(target) =
                    FeatureParameterTarget::new(feature.id(), path, ParameterValueType::Length)
                else {
                    continue;
                };
                if let (false, Some(value_mm)) = (
                    shown.contains(&target),
                    snapshot.feature_parameter_value(&target),
                ) {
                    candidates.push(Candidate {
                        feature_id: feature.id(),
                        path: path.to_owned(),
                        value_mm,
                    });
                }
            }
        }
        candidates
    }

    pub(crate) fn show_dimensions_section(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        let candidates = self.dimension_candidates(&snapshot);
        let rows: Vec<_> = snapshot
            .persistent_dimensions()
            .map(|dimension| {
                let label = self.dimension_row_label(&snapshot, dimension.id);
                (dimension.id, label)
            })
            .collect();
        if rows.is_empty() && candidates.is_empty() {
            return;
        }
        ui.separator();
        section_header(
            ui,
            self.palette(),
            &self.catalog.text("dock-measured-dimensions"),
        );
        for candidate in candidates {
            let label = self.catalog.format(
                "measured-dimension-create",
                &BTreeMap::from([
                    ("parameter", candidate.path.clone()),
                    ("value", format_height(candidate.value_mm)),
                ]),
            );
            if ui.button(label).clicked() {
                self.create_dimension(candidate);
            }
        }
        let picked = self.picked_dimension_id();
        for (id, label) in rows {
            if ui.selectable_label(picked == Some(id), label).clicked() {
                self.pick_dimension(id);
            }
        }
        if let Some(id) = self.picked_dimension_id() {
            self.show_dimension_fields(ui, id);
        }
    }

    fn show_dimension_fields(&mut self, ui: &mut egui::Ui, id: PersistentDimensionId) {
        let snapshot = self.document.current();
        let dimension = snapshot
            .persistent_dimension(id)
            .expect("the picked dimension");
        let shown = Some((id, snapshot.revision_id()));
        let newly_picked = self.dimension_fields.shown.map(|(shown, _)| shown) != Some(id);
        if self.dimension_fields.shown != shown {
            self.dimension_fields = DimensionFields {
                shown,
                name: dimension.name.clone(),
                unit: Some(dimension.presentation.unit),
                decimal_places: dimension.presentation.decimal_places.to_string(),
                reveal: newly_picked,
            };
        }
        let fields = &mut self.dimension_fields;
        let field = |ui: &mut egui::Ui, label: String, value: &mut String| {
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.add(egui::TextEdit::singleline(value).desired_width(140.0))
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                    });
            });
        };
        field(
            ui,
            self.catalog.text("measured-dimension-field-name"),
            &mut fields.name,
        );
        ui.horizontal(|ui| {
            for (unit, _) in UNITS {
                ui.radio_value(&mut fields.unit, Some(unit), unit.label());
            }
        });
        let decimals = self.catalog.text("measured-dimension-field-decimals");
        field(ui, decimals, &mut fields.decimal_places);
        // Only a feature parameter dimension can be declared again by number.
        let target = match &dimension.target {
            PersistentDimensionTarget::FeatureParameter(target) => Some(target),
            _ => None,
        };
        let decimal_places =
            crate::length_input::read_whole::<u8>(&fields.decimal_places, self.value_box.decimal)
                .ok();
        let apply = ui.add_enabled(
            target.is_some() && decimal_places.is_some(),
            egui::Button::new(self.catalog.text("measured-dimension-apply")),
        );
        if fields.reveal {
            if ui.clip_rect().contains_rect(apply.rect) {
                fields.reveal = false;
            } else {
                let instant = egui::style::ScrollAnimation::none();
                ui.scroll_to_rect_animation(apply.rect, None, instant);
            }
        }
        if let (true, Some(target)) = (apply.clicked(), target) {
            let unit = UNITS
                .into_iter()
                .find(|(unit, _)| Some(*unit) == fields.unit)
                .map(|(_, unit)| unit);
            let operation = AssistantCadEditOperation::UpsertDimension {
                dimension_id: id.0,
                name: fields.name.clone(),
                feature_id: target.feature_id.0,
                parameter_path: target.path.as_str().to_owned(),
                unit,
                decimal_places,
            };
            self.commit_dock_operation(operation);
        }
    }

    /// A dimension of the candidate parameter named after the part, picked
    /// once it exists.
    fn create_dimension(&mut self, candidate: Candidate) {
        let snapshot = self.document.current();
        let id = snapshot
            .persistent_dimensions()
            .map(|dimension| dimension.id.0)
            .max()
            .unwrap_or(0)
            + 1;
        let part = self
            .selected_instance_paths()
            .first()
            .and_then(|path| snapshot.occurrence(path.root_occurrence()))
            .map(|occurrence| occurrence.name().to_owned())
            .unwrap_or_default();
        let name = self.catalog.format(
            "measured-dimension-new-name",
            &BTreeMap::from([("part", part), ("parameter", candidate.path.clone())]),
        );
        let operation = AssistantCadEditOperation::UpsertDimension {
            dimension_id: id,
            name: name.trim().to_owned(),
            feature_id: candidate.feature_id.0,
            parameter_path: candidate.path,
            unit: None,
            decimal_places: None,
        };
        if self.commit_dock_operation(operation) {
            self.pick_dimension(PersistentDimensionId(id));
        }
    }

    fn pick_dimension(&mut self, id: PersistentDimensionId) {
        self.clear_selection();
        self.picked_space = None;
        self.picked_joint = None;
        self.picked_annotation = None;
        self.picked_dimension = Some((id, BTreeSet::new()));
    }

    /// Deletes the picked dimension with `delete_dimension`.
    pub(crate) fn delete_picked_dimension(&mut self, id: PersistentDimensionId) -> bool {
        let deleted = self.commit_dock_operation(AssistantCadEditOperation::DeleteDimension {
            dimension_id: id.0,
        });
        if deleted {
            self.picked_dimension = None;
        }
        deleted
    }

    /// The box of the parts the picked dimension measures with its value
    /// above it.
    pub(crate) fn paint_picked_dimension(&self, painter: &egui::Painter, rect: Rect) {
        let Some(id) = self.picked_dimension_id() else {
            return;
        };
        let Some(measures) = self.measures_of(&self.dimension_parts(id)) else {
            return;
        };
        let min = measures.world_min_mm;
        let max: [f64; 3] = std::array::from_fn(|axis| min[axis] + measures.world_size_mm[axis]);
        let color = Color32::from_rgb(52, 140, 235);
        self.paint_dashed_box(painter, rect, min, max, Stroke::new(1.8_f32, color));
        let top = Vec3::new((min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, max[2]);
        painter.text(
            self.project(top, rect),
            egui::Align2::CENTER_BOTTOM,
            self.dimension_row_label(&self.document.current(), id),
            egui::FontId::proportional(13.0),
            color,
        );
    }
}
