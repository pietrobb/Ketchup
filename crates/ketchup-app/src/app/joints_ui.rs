//! The dock's declared joints: listed with the parts they join and their
//! overlap volume, both parts outlined in the viewport while one is picked,
//! declared where two selected parts overlap, edited by number and deleted,
//! every change by the typed operations MCP uses.

use crate::selection_measures_ui::SelectionMeasures;
use crate::*;
use ketchup_application::model_query::assistant_instance_path;
use ketchup_assistant::sidecar::AssistantCadEditOperation;
use ketchup_geometry::prismatic::{Aabb, JointId};
use ketchup_model::document::JointParticipant;

/// The picked joint's volume as typed, read again whenever the joint or the
/// document revision changes.
#[derive(Default)]
pub(crate) struct JointFields {
    shown: Option<(JointId, u64)>,
    position: [String; 3],
    size: [String; 3],
    /// The joint was just picked and its fields are not yet in view.
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
    /// The picked joint while the part selection is the one it was picked
    /// with, no group is selected and the joint still exists.
    pub(crate) fn picked_joint_id(&self) -> Option<JointId> {
        let (id, with) = self.picked_joint.as_ref()?;
        if *with != self.selected_instance_paths() || self.selection.selected_group.is_some() {
            return None;
        }
        self.document.current().joint(*id).map(|joint| joint.id())
    }

    /// The placed parts the joint joins; a value a rule derives is none.
    pub(crate) fn joint_parts(&self, id: JointId) -> BTreeSet<InstancePath> {
        let snapshot = self.document.current();
        snapshot
            .joint(id)
            .map(|joint| {
                [joint.participant_a(), joint.participant_b()]
                    .into_iter()
                    .filter_map(JointParticipant::part)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn joint_participant_label(
        &self,
        snapshot: &Snapshot,
        participant: &JointParticipant,
    ) -> String {
        match participant {
            JointParticipant::Part(path) => match snapshot.occurrence(path.root_occurrence()) {
                Some(occurrence) if path.steps().is_empty() => occurrence.name().to_owned(),
                _ => Self::instance_path_label(path),
            },
            JointParticipant::Derived(identity) => self.catalog.format(
                "joints-rule-participant",
                &BTreeMap::from([("node", identity.root_rule_node_id.0.to_string())]),
            ),
        }
    }

    pub(crate) fn show_joints_section(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        let rows: Vec<_> = snapshot
            .joints()
            .map(|joint| {
                let label = self.catalog.format(
                    "joints-row",
                    &BTreeMap::from([
                        ("id", joint.id().0.to_string()),
                        (
                            "a",
                            self.joint_participant_label(&snapshot, joint.participant_a()),
                        ),
                        (
                            "b",
                            self.joint_participant_label(&snapshot, joint.participant_b()),
                        ),
                        ("dimensions", dimensions(joint.volume())),
                    ]),
                );
                (joint.id(), label)
            })
            .collect();
        let two_selected = self.selected_instance_paths().len() == 2;
        if rows.is_empty() && !two_selected {
            return;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-joints"));
        let create = self.catalog.text("joints-create");
        if ui
            .add_enabled(two_selected, egui::Button::new(create))
            .clicked()
        {
            self.create_joint_from_selection();
        }
        let picked = self.picked_joint_id();
        for (id, label) in rows {
            if ui.selectable_label(picked == Some(id), label).clicked() {
                self.pick_joint(id);
            }
        }
        if let Some(id) = self.picked_joint_id() {
            self.show_joint_fields(ui, id);
        }
    }

    fn show_joint_fields(&mut self, ui: &mut egui::Ui, id: JointId) {
        let snapshot = self.document.current();
        let joint = snapshot.joint(id).expect("the picked joint");
        let shown = Some((id, snapshot.revision_id()));
        let newly_picked = self.joint_fields.shown.map(|(shown, _)| shown) != Some(id);
        if self.joint_fields.shown != shown {
            self.joint_fields = JointFields {
                shown,
                position: joint.volume().min().map(format_height),
                size: joint.volume().extents().map(format_height),
                reveal: newly_picked,
            };
        }
        let fields = &mut self.joint_fields;
        let field = |ui: &mut egui::Ui, label: String, value: &mut String| {
            ui.horizontal(|ui| {
                ui.label(&label);
                ui.add(egui::TextEdit::singleline(value).desired_width(90.0))
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                    });
            });
        };
        for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let arguments = BTreeMap::from([("axis", axis.to_owned())]);
            let position = self.catalog.format("spaces-field-position", &arguments);
            field(ui, position, &mut fields.position[index]);
            let size = self.catalog.format("spaces-field-size", &arguments);
            field(ui, size, &mut fields.size[index]);
        }
        let parsed = |values: &[String; 3]| {
            let [x, y, z] = values.each_ref().map(|value| parse_distance_mm(value));
            Some([x?, y?, z?])
        };
        let boxed = parsed(&fields.position)
            .zip(parsed(&fields.size))
            .map(|(min, size)| (min, std::array::from_fn(|axis| min[axis] + size[axis])));
        // Only a joint of two placed parts can be declared again by number.
        let parts = [joint.participant_a(), joint.participant_b()].map(|participant| {
            participant
                .part()
                .and_then(|path| assistant_instance_path(&snapshot, path))
        });
        let apply = ui.add_enabled(
            boxed.is_some() && parts.iter().all(Option::is_some),
            egui::Button::new(self.catalog.text("spaces-apply")),
        );
        if fields.reveal {
            if ui.clip_rect().contains_rect(apply.rect) {
                fields.reveal = false;
            } else {
                let instant = egui::style::ScrollAnimation::none();
                ui.scroll_to_rect_animation(apply.rect, None, instant);
            }
        }
        if let (true, Some((min_mm, max_mm)), [Some(part_a), Some(part_b)]) =
            (apply.clicked(), boxed, parts)
        {
            self.commit_dock_operation(AssistantCadEditOperation::UpsertJoint {
                joint_id: id.0,
                part_a,
                part_b,
                min_mm,
                max_mm,
            });
        }
    }

    /// A joint of the two selected parts over the overlap of their world
    /// boxes; parts that do not overlap leave nothing to declare, and the
    /// planner says so.
    fn create_joint_from_selection(&mut self) {
        let paths: Vec<_> = self.selected_instance_paths().into_iter().collect();
        let [a, b] = paths.as_slice() else {
            return;
        };
        let snapshot = self.document.current();
        let (Some(part_a), Some(part_b), Some(box_a), Some(box_b)) = (
            assistant_instance_path(&snapshot, a),
            assistant_instance_path(&snapshot, b),
            self.measures_of(&BTreeSet::from([a.clone()])),
            self.measures_of(&BTreeSet::from([b.clone()])),
        ) else {
            return;
        };
        let max = |measures: &SelectionMeasures, axis: usize| {
            measures.world_min_mm[axis] + measures.world_size_mm[axis]
        };
        let min_mm =
            std::array::from_fn(|axis| box_a.world_min_mm[axis].max(box_b.world_min_mm[axis]));
        let max_mm = std::array::from_fn(|axis| max(&box_a, axis).min(max(&box_b, axis)));
        let id = snapshot
            .joints()
            .map(|joint| joint.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let operation = AssistantCadEditOperation::UpsertJoint {
            joint_id: id,
            part_a,
            part_b,
            min_mm,
            max_mm,
        };
        if self.commit_dock_operation(operation) {
            self.pick_joint(JointId(id));
        }
    }

    fn pick_joint(&mut self, id: JointId) {
        self.clear_selection();
        self.picked_space = None;
        self.picked_dimension = None;
        self.picked_joint = Some((id, BTreeSet::new()));
    }

    /// Deletes the picked joint with `delete_joint`.
    pub(crate) fn delete_picked_joint(&mut self, id: JointId) -> bool {
        let deleted =
            self.commit_dock_operation(AssistantCadEditOperation::DeleteJoint { joint_id: id.0 });
        if deleted {
            self.picked_joint = None;
        }
        deleted
    }

    /// Each part the picked joint joins outlined on its own, and the overlap
    /// volume between them.
    pub(crate) fn paint_picked_joint(&self, painter: &egui::Painter, rect: Rect) {
        let Some(id) = self.picked_joint_id() else {
            return;
        };
        let color = Color32::from_rgb(46, 170, 90);
        for path in self.joint_parts(id) {
            if let Some(measures) = self.measures_of(&BTreeSet::from([path])) {
                let min = measures.world_min_mm;
                let max = std::array::from_fn(|axis| min[axis] + measures.world_size_mm[axis]);
                self.paint_dashed_box(painter, rect, min, max, Stroke::new(1.8_f32, color));
            }
        }
        if let Some(joint) = self.document.current().joint(id) {
            let volume = joint.volume();
            self.paint_dashed_box(
                painter,
                rect,
                volume.min(),
                volume.max(),
                Stroke::new(1.2_f32, Color32::from_rgb(240, 78, 35)),
            );
        }
    }
}
