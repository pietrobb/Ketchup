//! Definition-local groups are addressed by owner and instance path, never root group IDs.
use crate::*;
use ketchup_model::document::InstancePathStep;

impl KetchupApp {
    pub(crate) fn show_local_component_outliner(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(EditContext::Definition {
            definition_id,
            instance_path,
        }) = self.selection.edit_context.last().cloned()
        else {
            return false;
        };
        let snapshot = self.document.current();
        let Some(definition) = snapshot.definition(definition_id) else {
            return false;
        };
        if definition.local_group_ids().is_empty() && definition.local_occurrence_ids().is_empty() {
            return false;
        }
        let scene = self.active_scene_query();
        self.show_local_group_level(ui, &snapshot, &instance_path, &scene);
        true
    }

    fn show_local_group_level(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        prefix: &InstancePath,
        scene: &[SceneOccurrence],
    ) {
        let Ok(resolved) = snapshot.resolve_instance_path(prefix) else {
            return;
        };
        let parent = match prefix.steps().last() {
            Some(InstancePathStep::Group(id)) => Some(*id),
            _ => None,
        };
        for group in snapshot.local_groups().filter(|group| {
            group.key().definition_id == resolved.definition_id && group.parent() == parent
        }) {
            let path = prefix.with_step(InstancePathStep::Group(group.key().local_id));
            let members = scene
                .iter()
                .filter(|item| path.is_prefix_of(&item.instance_path))
                .map(|item| item.instance_path.clone())
                .collect::<BTreeSet<_>>();
            let label = self.catalog.format(
                "outliner-group",
                &BTreeMap::from([
                    ("name", group.name().to_owned()),
                    ("count", members.len().to_string()),
                ]),
            );
            let selected = !members.is_empty() && members == self.selected_instance_paths();
            let (_, header, _) = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id(("local-group", &path)),
                true,
            )
            .show_header(ui, |ui| ui.selectable_label(selected, label))
            .body(|ui| self.show_local_group_level(ui, snapshot, &path, scene));
            if header.inner.clicked() {
                self.end_transform_correction();
                self.selection.clear();
                self.selection.occurrences = members;
                self.digest = self.catalog.format(
                    "digest-selected-group",
                    &BTreeMap::from([
                        ("name", group.name().to_owned()),
                        ("count", self.selection_count().to_string()),
                    ]),
                );
            }
        }
        for item in scene.iter().filter(|item| {
            prefix.is_prefix_of(&item.instance_path)
                && item.instance_path.steps().len() == prefix.steps().len() + 1
                && matches!(
                    item.instance_path.steps().last(),
                    Some(InstancePathStep::Occurrence(_))
                )
        }) {
            let label = self.catalog.format(
                "outliner-instance",
                &BTreeMap::from([
                    ("name", item.occurrence_name.clone()),
                    (
                        "visibility",
                        if item.visible { "◉" } else { "○" }.to_owned(),
                    ),
                ]),
            );
            let response = ui.selectable_label(self.selection.contains(&item.instance_path), label);
            if response.double_clicked() {
                self.enter_occurrence_context(item.instance_path.clone());
            } else if response.clicked() {
                let additive = ui.input(|input| input.modifiers.shift);
                self.select_from_outliner(item.instance_path.clone(), additive);
            }
        }
    }

    pub(crate) fn program_local_groups(
        snapshot: &Snapshot,
        owner: DefinitionId,
    ) -> Vec<serde_json::Value> {
        snapshot
            .local_groups()
            .filter(|group| group.key().definition_id == owner)
            .map(|group| {
                serde_json::json!({
                    "name": group.name(), "local_group_id": group.key().local_id.0,
                    "parent_local_group_id": group.parent().map(|id| id.0),
                    "transform": group.transform(),
                })
            })
            .collect()
    }

    pub(crate) fn program_local_members(
        snapshot: &Snapshot,
        owner: DefinitionId,
    ) -> Vec<serde_json::Value> {
        snapshot
            .local_occurrences()
            .filter(|part| part.key().definition_id == owner)
            .map(|part| {
                serde_json::json!({
                    "name": part.name(), "local_occurrence_id": part.key().local_id.0,
                    "definition_id": part.definition_id().0,
                    "parent_local_group_id": part.parent().map(|id| id.0),
                    "transform": part.transform(), "visible": part.visible(),
                })
            })
            .collect()
    }
}
