//! The right dock's outliner and layers: what is in the model, what is visible.

use crate::*;

impl KetchupApp {
    pub(crate) fn show_outliner(&mut self, ui: &mut egui::Ui) {
        self.show_pocket_properties(ui);
        self.show_classification_dimensions(ui);
        if self.panels.outliner_visible {
            let groups = self.outliner_groups();
            let entries = self.outliner_query();
            section_header(ui, self.palette(), &self.catalog.text("dock-outliner"));
            ui.separator();
            // Bounded so the sections below it stay reachable in the dock.
            egui::ScrollArea::vertical()
                .id_salt("outliner-scroll")
                .max_height(280.0)
                .show(ui, |ui| {
                    if self.show_local_component_outliner(ui) {
                        return;
                    }
                    for group in groups {
                        let label = self.catalog.format(
                            "outliner-group",
                            &BTreeMap::from([
                                ("name", group.name),
                                ("count", group.member_count.to_string()),
                            ]),
                        );
                        let response = ui.selectable_label(
                            self.selection.selected_group == Some(group.id),
                            label,
                        );
                        if response.double_clicked() {
                            self.enter_group_context(group.id);
                        } else if response.clicked() {
                            self.select_group(group.id);
                        }
                    }
                    if !entries.is_empty() {
                        ui.separator();
                    }
                    for mut definition in entries {
                        if definition.occurrences.len() == 1 {
                            let occurrence = definition.occurrences.remove(0);
                            let default_name = format!("{} #1", definition.name);
                            let name = if occurrence.name == default_name {
                                definition.name
                            } else {
                                occurrence.name.clone()
                            };
                            let mut arguments = BTreeMap::from([
                                ("name", name),
                                ("dimensions", definition.specification),
                                (
                                    "visibility",
                                    if occurrence.visible { "◉" } else { "○" }.to_owned(),
                                ),
                            ]);
                            let key = if let Some(group_id) = occurrence.parent {
                                arguments.insert("group", group_id.0.to_string());
                                "outliner-object-grouped"
                            } else {
                                "outliner-object"
                            };
                            let response = ui.selectable_label(
                                self.selection.contains(&occurrence.instance_path),
                                self.catalog.format(key, &arguments),
                            );
                            if response.double_clicked() {
                                self.enter_occurrence_context(occurrence.instance_path.clone());
                            } else if response.clicked() {
                                let additive = ui.input(|input| input.modifiers.shift);
                                self.select_from_outliner(occurrence.instance_path, additive);
                            }
                        } else {
                            let count = definition.occurrences.len();
                            let heading = self.catalog.format(
                                "outliner-component",
                                &BTreeMap::from([
                                    ("name", definition.name),
                                    ("count", count.to_string()),
                                    ("dimensions", definition.specification),
                                ]),
                            );
                            let definition_id = definition.id;
                            let response = egui::CollapsingHeader::new(heading)
                                .id_salt(definition_id.0)
                                .show(ui, |ui| {
                                    for occurrence in definition.occurrences {
                                        let mut arguments = BTreeMap::from([
                                            ("name", occurrence.name),
                                            (
                                                "visibility",
                                                if occurrence.visible { "◉" } else { "○" }
                                                    .to_owned(),
                                            ),
                                        ]);
                                        let key = if let Some(group_id) = occurrence.parent {
                                            arguments.insert("group", group_id.0.to_string());
                                            "outliner-instance-grouped"
                                        } else {
                                            "outliner-instance"
                                        };
                                        let row = ui.selectable_label(
                                            self.selection.contains(&occurrence.instance_path),
                                            self.catalog.format(key, &arguments),
                                        );
                                        if row.double_clicked() {
                                            self.enter_occurrence_context(
                                                occurrence.instance_path.clone(),
                                            );
                                        } else if row.clicked() {
                                            let additive = ui.input(|input| input.modifiers.shift);
                                            self.select_from_outliner(
                                                occurrence.instance_path,
                                                additive,
                                            );
                                        }
                                    }
                                });
                            if response.header_response.clicked() {
                                let additive = ui.input(|input| input.modifiers.shift);
                                self.select_definition(definition_id, additive);
                            }
                        }
                        ui.add_space(2.0);
                    }
                });
            self.show_spaces_section(ui);
            self.show_dimensions_section(ui);
            self.show_joints_section(ui);
            self.show_variants_section(ui);
        }
    }

    /// Whether the layers lead the dock: once the document has layers,
    /// switching what is visible must not need scrolling past other sections.
    pub(crate) fn layers_lead_dock(&self) -> bool {
        self.document.current().tags().next().is_some()
    }

    /// Layers, saved views and the section plane, separated from the sections
    /// below (`first`) or above it.
    pub(crate) fn show_layers_section(&mut self, ui: &mut egui::Ui, first: bool) {
        if self.panels.tags_visible {
            if !first {
                ui.separator();
            }
            section_header(ui, self.palette(), &self.catalog.text("dock-tags"));
            ui.horizontal_wrapped(|ui| {
                let create_enabled = self.can_begin_tag_creation(None);
                if ui
                    .add_enabled(
                        create_enabled,
                        egui::Button::new(self.catalog.text("tags-create")),
                    )
                    .clicked()
                {
                    self.begin_tag_creation(None);
                }
                let selected_occurrence_ids = self.selected_occurrence_ids();
                let enabled = self.can_begin_tag_creation(Some(&selected_occurrence_ids));
                let create_from_selection = icon_button(
                    ui,
                    enabled,
                    "+",
                    &self.catalog.text("tags-create-from-selection"),
                );
                if create_from_selection.clicked() {
                    self.begin_tag_creation(Some(selected_occurrence_ids));
                }
                let show_enabled = self.can_set_all_tag_visibility(true);
                let show_all =
                    icon_button(ui, show_enabled, "👁", &self.catalog.text("tags-show-all"));
                if show_all.clicked() {
                    self.set_all_tag_visibility(true);
                }
                let hide_enabled = self.can_set_all_tag_visibility(false);
                let hide_all =
                    icon_button(ui, hide_enabled, "○", &self.catalog.text("tags-hide-all"));
                if hide_all.clicked() {
                    self.set_all_tag_visibility(false);
                }
                let invert_enabled = self.can_invert_tag_visibility();
                let invert = icon_button(
                    ui,
                    invert_enabled,
                    "🔃",
                    &self.catalog.text("tags-invert-visibility"),
                );
                if invert.clicked() {
                    self.invert_tag_visibility();
                }
                let isolate_selection_enabled = self.can_isolate_selected_tags();
                let isolate_selection = icon_button(
                    ui,
                    isolate_selection_enabled,
                    "🎯",
                    &self.catalog.text("tags-isolate-selection"),
                );
                if isolate_selection.clicked() {
                    self.isolate_selected_tags();
                }
                let hide_selection_enabled = self.can_hide_selected_tags();
                let hide_selection = icon_button(
                    ui,
                    hide_selection_enabled,
                    "⊟",
                    &self.catalog.text("tags-hide-selection"),
                );
                if hide_selection.clicked() {
                    self.hide_selected_tags();
                }
                let show_selection_enabled = self.can_show_selected_tags();
                let show_selection = icon_button(
                    ui,
                    show_selection_enabled,
                    "⊞",
                    &self.catalog.text("tags-show-selection"),
                );
                if show_selection.clicked() {
                    self.show_selected_tags();
                }
                let invert_selection_enabled = self.can_invert_selected_tags();
                let invert_selection = icon_button(
                    ui,
                    invert_selection_enabled,
                    "◑",
                    &self.catalog.text("tags-invert-selection"),
                );
                if invert_selection.clicked() {
                    self.invert_selected_tags();
                }
                let select_matching_enabled = self.can_select_matching_tags();
                let select_matching = icon_button(
                    ui,
                    select_matching_enabled,
                    "🔍",
                    &self.catalog.text("tags-select-matching"),
                );
                if select_matching.clicked() {
                    self.select_matching_tags();
                }
            });
            ui.horizontal_wrapped(|ui| {
                let select_all_tagged_enabled = self.can_select_all_tagged_occurrences();
                let select_all_tagged = icon_button(
                    ui,
                    select_all_tagged_enabled,
                    "☑",
                    &self.catalog.text("tags-select-all-tagged"),
                );
                if select_all_tagged.clicked() {
                    self.select_all_tagged_occurrences();
                }
                let select_untagged_enabled = self.can_select_untagged_occurrences();
                let select_untagged = icon_button(
                    ui,
                    select_untagged_enabled,
                    "☐",
                    &self.catalog.text("tags-select-untagged"),
                );
                if select_untagged.clicked() {
                    self.select_untagged_occurrences();
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.command_enabled(AppCommand::MakeUnique),
                        egui::Button::new(self.catalog.text("model-make-unique")),
                    )
                    .clicked()
                {
                    self.dispatch_command(AppCommand::MakeUnique);
                }
            });
            // One scene query a frame for the counts and the layer rows.
            let snapshot = self.document.current();
            let scene = snapshot.scene_query();
            let hidden = scene.iter().filter(|item| !item.visible).count();
            ui.label(self.catalog.format(
                "tags-visibility",
                &BTreeMap::from([
                    ("hidden", hidden.to_string()),
                    ("total", scene.len().to_string()),
                ]),
            ));
            let active_scene = self.active_scene_from(&snapshot, Some(&scene));
            for (id, name, mut visible, label) in self.tag_rows() {
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut visible, label).changed() {
                        self.set_tag_visibility(id, visible);
                    }
                    let isolate_label = self
                        .catalog
                        .format("tags-isolate", &BTreeMap::from([("name", name.clone())]));
                    let isolate_enabled = self.can_isolate_tag(id);
                    let isolate = icon_button(ui, isolate_enabled, "◎", &isolate_label);
                    if isolate.clicked() {
                        self.isolate_tag(id);
                    }
                    let select_label = self
                        .catalog
                        .format("tags-select", &BTreeMap::from([("name", name.clone())]));
                    let select_enabled = self.can_select_tag_occurrences(id, &active_scene);
                    let select = icon_button(ui, select_enabled, "▣", &select_label);
                    if select.clicked() {
                        self.select_tag_occurrences(id);
                    }
                    let clear_label = self
                        .catalog
                        .format("tags-clear", &BTreeMap::from([("name", name.clone())]));
                    let clear_enabled = self.can_clear_tag(id);
                    let clear = icon_button(ui, clear_enabled, "×", &clear_label);
                    if clear.clicked() {
                        self.begin_tag_clear(id);
                    }
                    let rename_label = self
                        .catalog
                        .format("tags-rename", &BTreeMap::from([("name", name.clone())]));
                    let rename_enabled = self.tag_rename_source_plan(id).is_some();
                    let rename = icon_button(ui, rename_enabled, "✏", &rename_label);
                    if rename.clicked() {
                        self.begin_tag_rename(id);
                    }
                    let delete_label = self
                        .catalog
                        .format("tags-delete", &BTreeMap::from([("name", name.clone())]));
                    let delete_enabled = self.can_delete_tag(id);
                    let delete = icon_button(ui, delete_enabled, "🗑", &delete_label);
                    if delete.clicked() {
                        self.begin_tag_deletion(id);
                    }
                    let remove_label = self.catalog.format(
                        "tags-remove-selection",
                        &BTreeMap::from([("name", name.clone())]),
                    );
                    let remove_enabled = self.can_remove_selection_from_tag(id);
                    let remove = icon_button(ui, remove_enabled, "−", &remove_label);
                    if remove.clicked() {
                        self.remove_selection_from_tag(id);
                    }
                    let assign_label = self
                        .catalog
                        .format("tags-assign-selection", &BTreeMap::from([("name", name)]));
                    let assign_enabled = self.can_assign_selection_to_tag(id);
                    let assign = icon_button(ui, assign_enabled, "+", &assign_label);
                    if assign.clicked() {
                        self.assign_selection_to_tag(id);
                    }
                });
            }
            ui.horizontal(|ui| {
                self.command_button(ui, AppCommand::Hide);
                self.command_button(ui, AppCommand::Unhide);
            });
            self.saved_views_ui(ui);
            self.section_ui(ui);
            if first {
                ui.separator();
            }
        }
    }
}
