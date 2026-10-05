//! Saved views: a named camera, display switches, hidden tags and section plane kept in
//! the document.
//! Saving or activating a view never touches geometry or the document's program.

use crate::*;
use ketchup_model::document::{SavedCamera, SavedView, SavedViewId};

/// The stable name of a display switch, e.g. "xray" or "wireframe".
fn flag_name(flag: ViewFlag) -> &'static str {
    CommandRegistry::spec(AppCommand::View(flag))
        .label_key
        .trim_start_matches("view-")
}

impl KetchupApp {
    /// Saved views in creation order, as (id, name).
    pub fn saved_views(&self) -> Vec<(SavedViewId, String)> {
        self.document
            .current()
            .saved_views()
            .map(|view| (view.id(), view.name().to_owned()))
            .collect()
    }

    pub fn saved_view_id(&self, name: &str) -> Option<SavedViewId> {
        self.document
            .current()
            .saved_views()
            .find(|view| view.name() == name)
            .map(SavedView::id)
    }

    /// The current camera, display switches and hidden tags as a view named `name`.
    fn capture_view(&self, id: SavedViewId, name: &str) -> SavedView {
        let camera = &self.camera;
        SavedView {
            id,
            name: name.to_owned(),
            camera: SavedCamera {
                parallel: camera.projection_mode == ProjectionMode::Parallel,
                yaw_rad: f64::from(camera.yaw),
                pitch_rad: f64::from(camera.pitch),
                target_z_mm: camera.target_z,
                zoom: f64::from(camera.zoom),
                pan: [f64::from(camera.pan.x), f64::from(camera.pan.y)],
            },
            style: ViewFlag::ALL
                .into_iter()
                .filter(|flag| self.view.contains(*flag))
                .map(|flag| flag_name(flag).to_owned())
                .collect(),
            hidden_tags: self
                .document
                .current()
                .tags()
                .filter(|tag| !tag.visible())
                .map(|tag| tag.id())
                .collect(),
            section: self.section,
        }
    }

    fn commit_saved_view(&mut self, view: SavedView, digest_key: &str) -> bool {
        let name = view.name().to_owned();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::UpsertSavedView(view),
            ]))
            .is_err()
        {
            return false;
        }
        self.digest = self
            .catalog
            .format(digest_key, &BTreeMap::from([("name", name)]));
        true
    }

    /// Save what is shown now under `name`; a view of the same name is replaced.
    pub fn save_view(&mut self, name: &str) -> Option<SavedViewId> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let id = match self.saved_view_id(name) {
            Some(id) => id,
            None => self
                .document
                .current()
                .saved_views()
                .map(|view| view.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .map(SavedViewId)?,
        };
        let view = self.capture_view(id, name);
        self.commit_saved_view(view, "digest-saved-view-saved")
            .then_some(id)
    }

    /// Replace a view's camera, display switches and hidden tags with what is shown now.
    pub fn update_saved_view(&mut self, id: SavedViewId) -> bool {
        let Some(name) = self
            .document
            .current()
            .saved_view(id)
            .map(|view| view.name().to_owned())
        else {
            return false;
        };
        let view = self.capture_view(id, &name);
        self.commit_saved_view(view, "digest-saved-view-saved")
    }

    pub(crate) fn can_rename_saved_view(&self, id: SavedViewId, name: &str) -> bool {
        let name = name.trim();
        !name.is_empty()
            && self
                .document
                .current()
                .saved_view(id)
                .is_some_and(|view| view.name() != name)
            && self.saved_view_id(name).is_none()
    }

    pub fn rename_saved_view(&mut self, id: SavedViewId, name: &str) -> bool {
        if !self.can_rename_saved_view(id, name) {
            return false;
        }
        let Some(mut view) = self.document.current().saved_view(id).cloned() else {
            return false;
        };
        view.name = name.trim().to_owned();
        self.commit_saved_view(view, "digest-saved-view-renamed")
    }

    pub fn delete_saved_view(&mut self, id: SavedViewId) -> bool {
        let Some(name) = self
            .document
            .current()
            .saved_view(id)
            .map(|view| view.name().to_owned())
        else {
            return false;
        };
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::DeleteSavedView { id },
            ]))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-saved-view-deleted",
            &BTreeMap::from([("name", name)]),
        );
        true
    }

    /// Show the model as the view was saved: tag visibility (one Undo step when it
    /// changes), camera and display switches. Geometry and the program stay untouched.
    pub fn activate_saved_view(&mut self, id: SavedViewId) -> bool {
        let snapshot = self.document.current();
        let Some(view) = snapshot.saved_view(id).cloned() else {
            return false;
        };
        let commands = snapshot
            .tags()
            .filter_map(|tag| {
                let visible = !view.hidden_tags.contains(&tag.id());
                (tag.visible() != visible).then_some(CanonicalCommand::SetTagVisibility {
                    id: tag.id(),
                    visible,
                })
            })
            .collect::<Vec<_>>();
        drop(snapshot);
        if !commands.is_empty()
            && self
                .apply_batch_with_work_recovery(&CommandBatch::new(commands))
                .is_err()
        {
            return false;
        }
        let before = self.camera_view_state();
        let camera = view.camera;
        self.camera.projection_mode = if camera.parallel {
            ProjectionMode::Parallel
        } else {
            ProjectionMode::Perspective
        };
        self.camera.yaw = camera.yaw_rad as f32;
        self.camera.pitch = camera.pitch_rad as f32;
        self.camera.target_z = camera.target_z_mm;
        self.camera.zoom = camera.zoom as f32;
        self.camera.pan = Vec2::new(camera.pan[0] as f32, camera.pan[1] as f32);
        for flag in ViewFlag::ALL {
            self.view.set(flag, view.style.contains(flag_name(flag)));
        }
        self.section = view.section;
        self.refresh_camera_distance();
        self.remember_camera_change(before);
        self.digest = self.catalog.format(
            "digest-saved-view-activated",
            &BTreeMap::from([("name", view.name)]),
        );
        true
    }

    /// The saved-views section under the tag list.
    pub(crate) fn saved_views_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("saved-views"));
        let name_label = self.catalog.text("saved-views-name");
        let save_label = self.catalog.text("saved-views-save");
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.saved_view_name)
                    .hint_text(name_label.as_str())
                    .desired_width(120.0),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name_label)
            });
            let enabled = !self.saved_view_name.trim().is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new(save_label))
                .clicked()
            {
                let name = self.saved_view_name.clone();
                if self.save_view(&name).is_some() {
                    self.saved_view_name.clear();
                }
            }
        });
        for (id, name) in self.saved_views() {
            ui.horizontal(|ui| {
                let arguments = BTreeMap::from([("name", name.clone())]);
                let activate = ui.button(name.as_str());
                name_widget(
                    &activate,
                    true,
                    &self.catalog.format("saved-views-activate", &arguments),
                );
                if activate.clicked() {
                    self.activate_saved_view(id);
                }
                let update = ui.button("↻");
                name_widget(
                    &update,
                    true,
                    &self.catalog.format("saved-views-update", &arguments),
                );
                if update.clicked() {
                    self.update_saved_view(id);
                }
                let typed = self.saved_view_name.clone();
                let rename_enabled = self.can_rename_saved_view(id, &typed);
                let rename = icon_button(
                    ui,
                    rename_enabled,
                    "✏",
                    &self.catalog.format("saved-views-rename", &arguments),
                );
                if rename.clicked() && self.rename_saved_view(id, &typed) {
                    self.saved_view_name.clear();
                }
                let delete = icon_button(
                    ui,
                    true,
                    "🗑",
                    &self.catalog.format("saved-views-delete", &arguments),
                );
                if delete.clicked() {
                    self.delete_saved_view(id);
                }
            });
        }
    }
}
