//! The options of File > Export > Presentation image: background, size and how see-through the
//! parts of each material of the visible model are. Save image… asks where and makes it.

use super::presentation_export::PresentationRequest;
use crate::*;
use ketchup_manufacturing::presentation::{Background, MAX_PRESENTATION_SIDE};
use ketchup_model::drawing::BomItems;

/// What the user chose so far.
pub(crate) struct PresentationDialog {
    pub(crate) request: PresentationRequest,
    /// The background colour, kept while another background is picked.
    pub(crate) color: [u8; 3],
    /// Opacity in percent of each material of the visible parts, 100 = opaque.
    pub(crate) materials: BTreeMap<String, u8>,
}

impl PresentationDialog {
    /// The request with only the materials made see-through.
    pub(crate) fn request(&self) -> PresentationRequest {
        PresentationRequest {
            opacity_percent: self
                .materials
                .iter()
                .filter(|(_, percent)| **percent < 100)
                .map(|(name, percent)| (name.clone(), *percent))
                .collect(),
            ..self.request.clone()
        }
    }
}

impl KetchupApp {
    /// The materials of the visible parts, by name.
    pub(crate) fn visible_material_names(&self) -> Result<BTreeSet<String>, ExportError> {
        let snapshot = self.document.current();
        let items = BomItems::new(&snapshot).map_err(ExportError::failed)?;
        let mut names = BTreeSet::new();
        for body in self.current_visible_mesh_scene(&snapshot)? {
            let item = items
                .item(&body.occurrence.instance_path)
                .map_err(ExportError::failed)?;
            if let Some(name) = items.material_name(&item).map_err(ExportError::failed)? {
                names.insert(name.to_owned());
            }
        }
        Ok(names)
    }

    pub(crate) fn open_presentation_dialog(&mut self) {
        let materials = match self.visible_material_names() {
            Ok(names) => names.into_iter().map(|name| (name, 100)).collect(),
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-export-presentation",
                    &BTreeMap::from([("path", String::new()), ("reason", error.to_string())]),
                );
                return;
            }
        };
        self.file.activity.presentation_dialog = Some(PresentationDialog {
            request: PresentationRequest::default(),
            color: [255, 255, 255],
            materials,
        });
    }

    pub(crate) fn show_presentation_window(&mut self, context: &egui::Context) {
        let Some(mut dialog) = self.file.activity.presentation_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("presentation-title"))
            .id(egui::Id::new("presentation-image"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                self.presentation_options(ui, &mut dialog);
                ui.horizontal(|ui| {
                    save = ui.button(self.catalog.text("presentation-save")).clicked();
                    cancel = ui.button(self.catalog.text("dialog-cancel")).clicked();
                });
            });
        if save {
            if let Some(path) = self.choose_export_path("png") {
                self.start_presentation_export(path, &dialog.request(), context);
            }
            return;
        }
        if open && !cancel {
            self.file.activity.presentation_dialog = Some(dialog);
        }
    }

    fn presentation_options(&self, ui: &mut egui::Ui, dialog: &mut PresentationDialog) {
        let options = &mut dialog.request.options;
        ui.label(self.catalog.text("presentation-background"));
        ui.horizontal(|ui| {
            let background = &mut options.background;
            ui.radio_value(
                background,
                Background::White,
                self.catalog.text("presentation-background-white"),
            );
            ui.radio_value(
                background,
                Background::Transparent,
                self.catalog.text("presentation-background-transparent"),
            );
            let colored = matches!(background, Background::Color(_));
            if ui
                .radio(colored, self.catalog.text("presentation-background-color"))
                .clicked()
            {
                *background = Background::Color(dialog.color);
            }
            if egui::color_picker::color_edit_button_srgb(ui, &mut dialog.color).changed() {
                *background = Background::Color(dialog.color);
            }
        });
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("presentation-size"));
            ui.add(egui::DragValue::new(&mut options.width).range(1..=MAX_PRESENTATION_SIDE));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut options.height).range(1..=MAX_PRESENTATION_SIDE));
            ui.label("px");
        });
        ui.checkbox(
            &mut options.shadows,
            self.catalog.text("presentation-shadows"),
        );
        ui.checkbox(
            &mut options.ambient_occlusion,
            self.catalog.text("presentation-corners"),
        );
        ui.checkbox(
            &mut options.outlines,
            self.catalog.text("presentation-outlines"),
        );
        if !dialog.materials.is_empty() {
            ui.separator();
            ui.label(self.catalog.text("presentation-opacity"));
            for (name, percent) in &mut dialog.materials {
                ui.add(
                    egui::Slider::new(percent, 0..=100)
                        .suffix(" %")
                        .text(name.as_str()),
                );
            }
        }
    }
}
