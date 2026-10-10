//! Viewer sharing preparation: author notes, shared components and scenes.
//! Kept as project companion data next to the CAD model; the CAD model, its
//! history and Undo are never changed by these notes or choices.
use crate::*;
use ketchup_model::document::{DefinitionId, OccurrenceId, SavedViewId};
use serde::{Deserialize, Serialize};

/// Where the settings live among the `.ketchup` companion data.
const VIEWER_NAMESPACE: &str = "org.ketchup.viewer";
const VIEWER_SETTINGS_PATH: &str = "sharing-v1.json";
/// The package limit of one note; longer text would make the export fail.
pub(crate) const MAX_VIEWER_NOTE_BYTES: usize = 65_536;

/// Author choices for the Viewer export, saved with the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ViewerSettings {
    /// Note shown on every occurrence of a component definition.
    #[serde(default)]
    pub(crate) definition_notes: BTreeMap<u64, String>,
    /// Note of exactly one occurrence, root or nested.
    #[serde(default)]
    pub(crate) occurrence_notes: Vec<(InstancePath, String)>,
    /// Root components left out of the package entirely, geometry included.
    #[serde(default)]
    pub(crate) excluded_roots: BTreeSet<u64>,
    /// Shared saved views; `None` shares every saved view.
    #[serde(default)]
    pub(crate) scenes: Option<BTreeSet<u64>>,
    /// The scene the Viewer opens first.
    #[serde(default)]
    pub(crate) start_scene: Option<u64>,
}

impl ViewerSettings {
    /// The note of a definition, empty when none.
    pub(crate) fn definition_note(&self, id: DefinitionId) -> &str {
        self.definition_notes.get(&id.0).map_or("", String::as_str)
    }

    /// The note of one occurrence, empty when none.
    pub(crate) fn occurrence_note(&self, path: &InstancePath) -> &str {
        self.occurrence_notes
            .iter()
            .find(|(candidate, _)| candidate == path)
            .map_or("", |(_, note)| note.as_str())
    }

    /// Whether the saved view goes into the package.
    fn scene_shared(&self, id: SavedViewId) -> bool {
        self.scenes
            .as_ref()
            .is_none_or(|scenes| scenes.contains(&id.0))
    }
}

impl KetchupApp {
    /// Stored settings, the defaults when none are; unreadable stored settings
    /// are an error, never quietly the defaults.
    pub(crate) fn stored_viewer_settings(&self) -> Result<ViewerSettings, serde_json::Error> {
        self.file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == VIEWER_NAMESPACE && entry.path() == VIEWER_SETTINGS_PATH
            })
            .map_or_else(
                || Ok(ViewerSettings::default()),
                |entry| serde_json::from_slice(entry.bytes()),
            )
    }

    /// Store the settings without empty notes; `false` when nothing changed.
    pub(crate) fn store_viewer_settings(&mut self, mut settings: ViewerSettings) -> bool {
        settings
            .definition_notes
            .retain(|_, note| !note.trim().is_empty());
        settings
            .occurrence_notes
            .retain(|(_, note)| !note.trim().is_empty());
        if settings
            .definition_notes
            .values()
            .chain(settings.occurrence_notes.iter().map(|(_, note)| note))
            .any(|note| note.len() > MAX_VIEWER_NOTE_BYTES || note.contains('\0'))
        {
            return false;
        }
        if self.stored_viewer_settings().ok().as_ref() == Some(&settings) {
            return true;
        }
        let Ok(bytes) = serde_json::to_vec(&settings) else {
            return false;
        };
        let Ok(entry) = ketchup_model::persistence::ExtensionEntry::new(
            VIEWER_NAMESPACE,
            VIEWER_SETTINGS_PATH,
            false,
            bytes,
        ) else {
            return false;
        };
        self.file.container_data.set_extension(entry);
        self.viewer_prep.unsaved = true;
        true
    }

    /// Apply `edit` to the stored settings; refused while they are unreadable.
    fn edit_viewer_settings(&mut self, edit: impl FnOnce(&mut ViewerSettings)) -> bool {
        // Unreadable stored settings are kept untouched rather than replaced.
        let Ok(mut settings) = self.stored_viewer_settings() else {
            return false;
        };
        edit(&mut settings);
        self.store_viewer_settings(settings)
    }

    /// Set the note shown on every occurrence of a definition.
    pub fn set_viewer_definition_note(&mut self, id: DefinitionId, note: &str) -> bool {
        self.edit_viewer_settings(|settings| {
            settings.definition_notes.insert(id.0, note.to_owned());
        })
    }

    /// Set the note of exactly one occurrence.
    pub fn set_viewer_occurrence_note(&mut self, path: &InstancePath, note: &str) -> bool {
        self.edit_viewer_settings(|settings| {
            settings
                .occurrence_notes
                .retain(|(candidate, _)| candidate != path);
            settings
                .occurrence_notes
                .push((path.clone(), note.to_owned()));
            settings.occurrence_notes.sort_by(|a, b| a.0.cmp(&b.0));
        })
    }

    /// Include or leave out a root component, geometry included.
    pub fn set_viewer_component_shared(&mut self, root: OccurrenceId, shared: bool) -> bool {
        self.edit_viewer_settings(|settings| {
            if shared {
                settings.excluded_roots.remove(&root.0);
            } else {
                settings.excluded_roots.insert(root.0);
            }
        })
    }

    /// Include or leave out a saved view.
    pub fn set_viewer_scene_shared(&mut self, id: SavedViewId, shared: bool) -> bool {
        let all = self
            .document
            .current()
            .saved_views()
            .map(|view| view.id().0)
            .collect::<BTreeSet<_>>();
        self.edit_viewer_settings(|settings| {
            let scenes = settings.scenes.get_or_insert_with(|| all.clone());
            if shared {
                scenes.insert(id.0);
            } else {
                scenes.remove(&id.0);
            }
            if *scenes == all {
                settings.scenes = None;
            }
        })
    }

    /// Make a saved view the one the Viewer opens first.
    pub fn set_viewer_start_scene(&mut self, id: SavedViewId) -> bool {
        self.edit_viewer_settings(|settings| settings.start_scene = Some(id.0))
    }

    /// Shared saved views in document order, the opening scene first; a start
    /// scene that is no longer shared falls back to the first shared scene.
    pub(crate) fn viewer_export_scenes(&self, settings: &ViewerSettings) -> Vec<SavedViewId> {
        let mut scenes = self
            .document
            .current()
            .saved_views()
            .map(|view| view.id())
            .filter(|&id| settings.scene_shared(id))
            .collect::<Vec<_>>();
        if let Some(start) = settings
            .start_scene
            .and_then(|start| scenes.iter().position(|id| id.0 == start))
        {
            let start = scenes.remove(start);
            scenes.insert(0, start);
        }
        scenes
    }

    /// Per-scene sharing controls inside one saved-view row.
    pub(crate) fn viewer_scene_controls(&mut self, ui: &mut egui::Ui, id: SavedViewId, name: &str) {
        let settings = self.stored_viewer_settings().unwrap_or_default();
        let arguments = BTreeMap::from([("name", name.to_owned())]);
        let mut shared = settings.scene_shared(id);
        let toggle = ui.checkbox(&mut shared, self.catalog.text("viewer-scene-share"));
        toggle.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                shared,
                self.catalog.format("viewer-scene-share-named", &arguments),
            )
        });
        if toggle.changed() {
            self.set_viewer_scene_shared(id, shared);
        }
        let start = self.viewer_export_scenes(&settings).first() == Some(&id);
        let opening = ui.add_enabled(
            shared,
            egui::RadioButton::new(start, self.catalog.text("viewer-scene-start")),
        );
        opening.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                shared,
                start,
                self.catalog.format("viewer-scene-start-named", &arguments),
            )
        });
        if opening.clicked() {
            self.set_viewer_start_scene(id);
        }
    }

    /// Author notes and sharing of the one selected component.
    pub(super) fn show_viewer_notes(&mut self, ui: &mut egui::Ui) {
        let paths = self.selected_instance_paths();
        let mut paths = paths.into_iter();
        let (Some(path), None) = (paths.next(), paths.next()) else {
            return;
        };
        let snapshot = self.document.current();
        let Ok(resolved) = snapshot.resolve_instance_path(&path) else {
            return;
        };
        let Some(definition) = snapshot.definition(resolved.definition_id) else {
            return;
        };
        let definition_name = definition.name().to_owned();
        let Ok(settings) = self.stored_viewer_settings() else {
            ui.label(self.catalog.text("viewer-notes-unreadable"));
            return;
        };
        egui::CollapsingHeader::new(self.catalog.text("viewer-notes-title"))
            .id_salt("viewer-notes")
            .default_open(false)
            .show(ui, |ui| {
                let arguments = BTreeMap::from([("name", definition_name)]);
                let label = self.catalog.format("viewer-note-definition", &arguments);
                ui.label(&label);
                let mut text = settings.definition_note(resolved.definition_id).to_owned();
                let field = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(2)
                        .char_limit(MAX_VIEWER_NOTE_BYTES / 4),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                });
                if field.changed() {
                    self.set_viewer_definition_note(resolved.definition_id, &text);
                }
                let label = self.catalog.text("viewer-note-occurrence");
                ui.label(&label);
                let mut text = settings.occurrence_note(&path).to_owned();
                let field = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(2)
                        .char_limit(MAX_VIEWER_NOTE_BYTES / 4),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                });
                if field.changed() {
                    self.set_viewer_occurrence_note(&path, &text);
                }
                let root = path.root_occurrence();
                let mut shared = !settings.excluded_roots.contains(&root.0);
                if ui
                    .checkbox(&mut shared, self.catalog.text("viewer-component-share"))
                    .changed()
                {
                    self.set_viewer_component_shared(root, shared);
                }
            });
        ui.separator();
    }
}
