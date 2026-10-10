//! Saved views: a named camera, display switches, hidden tags and section plane kept in
//! the document.
//! Saving or activating a view never touches geometry or the document's program.

use crate::*;
use ketchup_model::document::{SavedCamera, SavedView, SavedViewId};

mod viewer;

/// What the saved-view controls remember between frames.
#[derive(Default)]
pub(crate) struct SavedViewsUi {
    /// The name typed for saving or renaming a saved view.
    pub(crate) name: String,
    /// The scene shown last, highlighted in the scene tabs above the viewport.
    pub(crate) active: Option<SavedViewId>,
    /// The scene tab being renamed in place, and its edited name.
    pub(crate) renaming: Option<(SavedViewId, String)>,
    /// A tab clicked once at this input time: it is shown when no second
    /// click (rename) follows within the double-click delay.
    pub(crate) pending_activation: Option<(SavedViewId, f64)>,
}

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
        let saved = self.commit_saved_view(view, "digest-saved-view-saved");
        if saved {
            self.saved_views_ui.active = Some(id);
        }
        saved.then_some(id)
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
        if self.saved_views_ui.active == Some(id) {
            self.saved_views_ui.active = None;
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
        self.saved_views_ui.active = Some(id);
        self.refresh_camera_distance();
        self.remember_camera_change(before);
        self.digest = self.catalog.format(
            "digest-saved-view-activated",
            &BTreeMap::from([("name", view.name)]),
        );
        true
    }

    /// The scene tab stays highlighted only while the layers show what the
    /// scene saved: after Undo or a layer switch no tab claims the display.
    pub(crate) fn forget_scene_no_longer_shown(&mut self) {
        let Some(id) = self.saved_views_ui.active else {
            return;
        };
        let snapshot = self.document.current();
        let shown = snapshot.saved_view(id).is_some_and(|view| {
            snapshot
                .tags()
                .all(|tag| tag.visible() != view.hidden_tags.contains(&tag.id()))
        });
        if !shown {
            self.saved_views_ui.active = None;
        }
    }

    /// "Scene N" with the first N not taken by another view.
    fn next_scene_name(&self) -> String {
        (self.saved_views().len() + 1..)
            .map(|number| {
                self.catalog.format(
                    "scenes-default-name",
                    &BTreeMap::from([("number", number.to_string())]),
                )
            })
            .find(|name| self.saved_view_id(name).is_none())
            .expect("an unused scene number")
    }

    /// The scene tabs above the viewport: one click shows a saved view, "+" saves
    /// what is shown now as a new scene, a right click updates, renames or deletes.
    pub(crate) fn show_scene_tabs(&mut self, ui: &mut egui::Ui) {
        // A fixed whole-point height: text heights round differently at each
        // display scale, and the viewport below must keep the same shape.
        const HEIGHT: f32 = 24.0;
        let now = ui.input(|input| input.time);
        let double_click_delay = ui
            .ctx()
            .options(|options| options.input_options.max_double_click_delay);
        if let Some((id, clicked_at)) = self.saved_views_ui.pending_activation
            && now - clicked_at >= double_click_delay
        {
            self.saved_views_ui.pending_activation = None;
            self.activate_saved_view(id);
        }
        let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), HEIGHT), layout, |ui| {
            ui.set_min_height(HEIGHT);
            ui.label(egui::RichText::new(self.catalog.text("scenes")).weak());
            for (id, name) in self.saved_views() {
                let arguments = BTreeMap::from([("name", name.clone())]);
                if self.scene_rename_field(ui, id, &name) {
                    continue;
                }
                let tab =
                    ui.selectable_label(self.saved_views_ui.active == Some(id), name.as_str());
                name_widget(&tab, true, &self.catalog.format("scenes-tab", &arguments));
                // A double click renames without first showing the scene
                // (camera and an Undo step), so a click waits for the delay.
                if tab.double_clicked() {
                    self.saved_views_ui.pending_activation = None;
                    self.saved_views_ui.renaming = Some((id, name.clone()));
                } else if tab.clicked() {
                    self.saved_views_ui.pending_activation = Some((id, now));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(
                            double_click_delay,
                        ));
                }
                tab.context_menu(|ui| {
                    if ui
                        .button(self.catalog.format("saved-views-update", &arguments))
                        .clicked()
                    {
                        self.update_saved_view(id);
                        ui.close();
                    }
                    if ui.button(self.catalog.text("scenes-rename")).clicked() {
                        self.saved_views_ui.renaming = Some((id, name.clone()));
                        ui.close();
                    }
                    if ui
                        .button(self.catalog.format("saved-views-delete", &arguments))
                        .clicked()
                    {
                        self.delete_saved_view(id);
                        ui.close();
                    }
                });
            }
            let add = icon_button(ui, true, "+", &self.catalog.text("scenes-add"));
            if add.clicked() {
                let name = self.next_scene_name();
                self.save_view(&name);
            }
        });
    }

    /// The in-place name field of a scene tab being renamed, prefilled with its
    /// current name. Enter or clicking away keeps the new name, Escape keeps the
    /// old one. Returns whether `id` is being renamed (and so drew no tab).
    fn scene_rename_field(&mut self, ui: &mut egui::Ui, id: SavedViewId, name: &str) -> bool {
        let Some((_, typed)) = self
            .saved_views_ui
            .renaming
            .as_mut()
            .filter(|(renaming, _)| *renaming == id)
        else {
            return false;
        };
        let field = ui.add(
            egui::TextEdit::singleline(typed)
                .hint_text(name)
                .desired_width(140.0),
        );
        field.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::TextEdit,
                true,
                self.catalog.text("scenes-rename"),
            )
        });
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        // Escape never reaches here: `handle_shortcuts` cancels the rename.
        if field.lost_focus() {
            let typed = typed.trim().to_owned();
            self.saved_views_ui.renaming = None;
            if self.can_rename_saved_view(id, &typed) {
                self.rename_saved_view(id, &typed);
            } else if typed != name && self.saved_view_id(&typed).is_some() {
                self.digest = self.catalog.format(
                    "digest-saved-view-name-taken",
                    &BTreeMap::from([("name", typed)]),
                );
            }
        }
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
                egui::TextEdit::singleline(&mut self.saved_views_ui.name)
                    .hint_text(name_label.as_str())
                    .desired_width(120.0),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name_label)
            });
            let enabled = !self.saved_views_ui.name.trim().is_empty();
            if ui
                .add_enabled(enabled, egui::Button::new(save_label))
                .clicked()
            {
                let name = self.saved_views_ui.name.clone();
                if self.save_view(&name).is_some() {
                    self.saved_views_ui.name.clear();
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
                let update = icon_button(
                    ui,
                    true,
                    "↻",
                    &self.catalog.format("saved-views-update", &arguments),
                );
                if update.clicked() {
                    self.update_saved_view(id);
                }
                let typed = self.saved_views_ui.name.clone();
                let rename_enabled = self.can_rename_saved_view(id, &typed);
                let rename = icon_button(
                    ui,
                    rename_enabled,
                    "✏",
                    &self.catalog.format("saved-views-rename", &arguments),
                );
                if rename.clicked() && self.rename_saved_view(id, &typed) {
                    self.saved_views_ui.name.clear();
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
                self.viewer_scene_controls(ui, id, &name);
            });
        }
    }
}
