//! The Viewer window: toolbar, scenes, component information and 3D view.
//! Shared by the desktop binary and the mobile entry points.

use crate::{
    camera::{Camera, StandardView},
    gpu::{MeshRenderer, RenderTarget, RenderView},
    model::Model,
    text::Language,
};
use eframe::egui;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{DisplayStyle, LocalDimensions, Section};
use num_traits::ToPrimitive;
use std::{io::Read, path::Path};

/// The read-only Viewer window: one opened package, its active scene and the
/// viewer-only state (selection, visibility, camera) that never edits the package.
pub struct ViewerApp {
    model: Option<Model>,
    camera: Option<Camera>,
    selected: Option<usize>,
    scene: Option<usize>,
    visible: Vec<bool>,
    style: DisplayStyle,
    section: Option<Section>,
    error: Option<Rejection>,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    renderer: Option<MeshRenderer>,
    target: Option<(RenderTarget, egui::TextureId)>,
    language: Language,
}

impl ViewerApp {
    /// An empty Viewer in the system language; `render_state` is `None` in tests.
    pub fn new(render_state: Option<eframe::egui_wgpu::RenderState>) -> Self {
        Self {
            language: Language::detect(),
            model: None,
            camera: None,
            selected: None,
            scene: None,
            visible: vec![],
            style: DisplayStyle::ShadedEdges,
            section: None,
            error: None,
            render_state,
            renderer: None,
            target: None,
        }
    }

    /// Use `language` instead of the system language.
    #[must_use]
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    /// The interface text for `key` in the active language.
    fn t(&self, key: &'static str) -> &'static str {
        self.language.text(key)
    }

    /// Back to the active scene's visibility; viewer-only, the package is unchanged.
    fn restore_scene_visibility(&mut self) {
        if let Some(model) = &self.model {
            self.visible = visibility(model, self.scene);
        }
    }

    /// Hide the picked component and drop the selection.
    fn hide_selected(&mut self) {
        if let Some(index) = self.selected.take() {
            self.visible[index] = false;
        }
    }

    /// Show only the picked component.
    fn isolate_selected(&mut self) {
        if let Some(index) = self.selected {
            self.visible.iter_mut().for_each(|visible| *visible = false);
            self.visible[index] = true;
        }
    }

    // Mobile adapters can supply bytes from their sandbox or document picker.
    pub fn open(&mut self, reader: impl Read) -> Result<(), Rejection> {
        let model = Model::read(reader)?;
        // Validate every scene before replacing the current model or its GPU resources.
        let cameras = model
            .manifest()
            .scenes
            .iter()
            .map(|s| Camera::from_scene(&model, &s.camera))
            .collect::<Result<Vec<_>, _>>()?;
        let scene = model
            .manifest()
            .start_scene
            .and_then(|id| model.manifest().scenes.iter().position(|s| s.id == id));
        let camera = scene.map_or_else(|| Camera::fit(&model), |i| cameras[i].clone());
        let renderer = self
            .render_state
            .as_ref()
            .map(|state| MeshRenderer::new(&state.device, &model))
            .transpose()?;
        self.visible = visibility(&model, scene);
        self.style = scene.map_or(DisplayStyle::ShadedEdges, |i| {
            model.manifest().scenes[i].style.clone()
        });
        self.section = scene.and_then(|i| model.manifest().scenes[i].section.clone());
        self.model = Some(model);
        self.renderer = renderer;
        self.camera = Some(camera);
        self.scene = scene;
        self.selected = None;
        self.error = None;
        Ok(())
    }

    /// Open a local `.ketchup-view` file; the current model stays on failure.
    pub fn open_path(&mut self, path: &Path) -> Result<(), Rejection> {
        let file = std::fs::File::open(path).map_err(|error| {
            Rejection::new("viewer.file.open", RejectionPhase::Io)
                .target(path.display().to_string())
                .reason("The local file could not be opened.")
                .fix_hint("Choose an accessible .ketchup-view exported from Kečup.")
                .caused_by(error)
        })?;
        self.open(file)
    }

    /// Switch to a saved scene: its camera, visibility, style and section.
    fn activate_scene(&mut self, index: usize) -> Result<(), Rejection> {
        let Some(model) = &self.model else {
            return Ok(());
        };
        let scene = &model.manifest().scenes[index];
        let camera = Camera::from_scene(model, &scene.camera)?;
        self.camera = Some(camera);
        self.visible = visibility(model, Some(index));
        self.style = scene.style.clone();
        self.section = scene.section.clone();
        self.scene = Some(index);
        if self.selected.is_some_and(|i| !self.visible[i]) {
            self.selected = None;
        }
        Ok(())
    }

    /// Toolbar with scenes on top, information panel at the side (bottom on
    /// narrow phone screens) and the 3D view in the middle.
    pub fn show(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("viewer_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading(self.t("viewer-title"));
                #[cfg(any(target_os = "windows", target_os = "macos"))]
                if ui.button(self.t("viewer-open")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter(self.t("viewer-file-filter"), &["ketchup-view"])
                        .pick_file()
                    && let Err(error) = self.open_path(&path)
                {
                    self.error = Some(error);
                }
                if ui.button(self.t("viewer-fit")).clicked()
                    && let Some(model) = &self.model
                {
                    self.camera = Some(Camera::fit(model));
                }
                for view in StandardView::ALL {
                    let label = self.t(match view {
                        StandardView::Iso => "viewer-view-iso",
                        StandardView::Front => "viewer-view-front",
                        StandardView::Back => "viewer-view-back",
                        StandardView::Left => "viewer-view-left",
                        StandardView::Right => "viewer-view-right",
                        StandardView::Top => "viewer-view-top",
                    });
                    if ui.small_button(label).clicked()
                        && let (Some(model), Some(camera)) = (&self.model, &mut self.camera)
                    {
                        camera.standard(model, view);
                    }
                }
            });
            let mut requested = None;
            if let Some(model) = &self.model {
                ui.horizontal_wrapped(|ui| {
                    for (i, scene) in model.manifest().scenes.iter().enumerate() {
                        if ui
                            .selectable_label(self.scene == Some(i), &scene.name)
                            .clicked()
                        {
                            requested = Some(i);
                        }
                    }
                });
            }
            if let Some(index) = requested
                && let Err(error) = self.activate_scene(index)
            {
                self.error = Some(error);
            }
            // A translated headline; the rejection itself is the technical detail.
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, self.t("viewer-error"));
                ui.small(error.to_string());
                ui.small(error.fix_hint_text());
            }
        });
        if ctx.screen_rect().width() < 650.0 {
            egui::TopBottomPanel::bottom("viewer_info_mobile")
                .max_height(160.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.information(ui))
                });
        } else {
            egui::SidePanel::right("viewer_info")
                .default_width(220.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.information(ui))
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| self.viewport(ui));
    }

    /// The author's notes come first; name, material and own size follow.
    fn information(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t("viewer-component"));
        let (Some(index), Some(model)) = (self.selected, &self.model) else {
            ui.label(self.t("viewer-pick-prompt"));
            if self.visible.iter().any(|visible| !visible)
                && ui.button(self.t("viewer-show-scene")).clicked()
            {
                self.restore_scene_visibility();
            }
            return;
        };
        let occurrence = &model.manifest().occurrences[index];
        let definition = model
            .manifest()
            .definitions
            .iter()
            .find(|d| d.id == occurrence.definition_id)
            .expect("validated definition");
        ui.label(egui::RichText::new(&occurrence.name).strong().size(18.0));
        for note in [&occurrence.note, &definition.note].into_iter().flatten() {
            ui.label(egui::RichText::new(note).size(16.0));
        }
        if let Some(material) = occurrence
            .material
            .as_ref()
            .or(definition.material.as_ref())
        {
            ui.label(
                self.language
                    .format("viewer-material", &[("material", material)]),
            );
        }
        if let Some(dimensions) = &definition.local_dimensions {
            let (label, size) = match dimensions {
                LocalDimensions::AuthoredAxes { size_mm } => ("viewer-size", *size_mm),
                LocalDimensions::LocalBounds { min_mm, max_mm } => (
                    "viewer-own-size",
                    std::array::from_fn(|i| max_mm[i] - min_mm[i]),
                ),
            };
            let [x, y, z] = size.map(millimetres);
            ui.label(
                self.language
                    .format(label, &[("x", &x), ("y", &y), ("z", &z)]),
            );
        }
        if !occurrence.attributes.is_empty() {
            egui::CollapsingHeader::new(self.t("viewer-attributes")).show(ui, |ui| {
                for (name, value) in &occurrence.attributes {
                    ui.label(format!("{name}: {value}"));
                }
            });
        }
        let path = occurrence.path.glb_key();
        ui.small(
            self.language
                .format("viewer-source-path", &[("path", &path)]),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.t("viewer-hide")).clicked() {
                self.hide_selected();
            }
            if ui.button(self.t("viewer-isolate")).clicked() {
                self.isolate_selected();
            }
            if ui.button(self.t("viewer-show-scene")).clicked() {
                self.restore_scene_visibility();
            }
        });
    }

    /// The 3D view: orbit, pan, zoom and pick, then draw the GPU frame.
    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let viewport_label = self.t("viewer-viewport");
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Image, true, viewport_label)
        });
        let prompt = self.t("viewer-open-prompt");
        let (Some(model), Some(camera)) = (&self.model, &mut self.camera) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                prompt,
                egui::FontId::proportional(20.0),
                egui::Color32::WHITE,
            );
            return;
        };
        let aspect = rect.width().max(1.0) / rect.height().max(1.0);
        let touch = ui.input(egui::InputState::multi_touch);
        let short_px = rect.width().min(rect.height()).max(1.0);
        if let Some(touch) = touch.filter(|touch| touch.num_touches > 1) {
            // Two fingers move and pinch the model; only one finger orbits.
            camera.pan(
                [touch.translation_delta.x, touch.translation_delta.y],
                short_px,
            );
        } else if response.dragged() {
            let (delta, pan) = ui.input(|input| {
                (
                    input.pointer.delta(),
                    input.pointer.secondary_down()
                        || input.pointer.middle_down()
                        || input.modifiers.shift,
                )
            });
            if pan {
                camera.pan([delta.x, delta.y], short_px);
            } else {
                camera.orbit([delta.x, delta.y]);
            }
        }
        let multitouch = touch.is_some_and(|touch| touch.num_touches > 1);
        if response.hovered() || multitouch {
            let factor =
                ui.input(|input| input.zoom_delta() * (input.smooth_scroll_delta.y * 0.002).exp());
            camera.zoom(factor);
        }
        if response.clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            let ndc = [
                (point.x - rect.left()) / rect.width() * 2.0 - 1.0,
                1.0 - (point.y - rect.top()) / rect.height() * 2.0,
            ];
            match camera.pick(model, ndc, aspect, &self.visible, self.section.as_ref()) {
                Ok(selected) => self.selected = selected,
                Err(error) => self.error = Some(error),
            }
        }
        if let (Some(state), Some(renderer)) = (&self.render_state, &self.renderer) {
            let pixels = ui.ctx().pixels_per_point();
            let limit = state.device.limits().max_texture_dimension_2d;
            let size = [
                (rect.width() * pixels).to_u32().unwrap_or(1),
                (rect.height() * pixels).to_u32().unwrap_or(1),
            ]
            .map(|value| value.clamp(1, limit));
            if self
                .target
                .as_ref()
                .is_none_or(|(target, _)| target.size != size)
            {
                let mut egui_renderer = state.renderer.write();
                if let Some((_, id)) = self.target.take() {
                    egui_renderer.free_texture(&id);
                }
                let target = RenderTarget::new(&state.device, size);
                let id = egui_renderer.register_native_texture(
                    &state.device,
                    &target.view,
                    wgpu::FilterMode::Linear,
                );
                self.target = Some((target, id));
            }
            if let Some((target, id)) = &self.target {
                renderer.render(
                    &state.device,
                    &state.queue,
                    target,
                    camera,
                    &RenderView {
                        selected: self.selected,
                        visible: &self.visible,
                        style: &self.style,
                        section: self.section.as_ref(),
                    },
                );
                ui.painter().image(
                    *id,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
    }
}

/// Millimetres with at most two decimals and no trailing zeros: `600`, `18.5`.
fn millimetres(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Per occurrence: visible unless the scene hides it or one of its ancestors.
fn visibility(model: &Model, scene: Option<usize>) -> Vec<bool> {
    model
        .manifest()
        .occurrences
        .iter()
        .map(|occurrence| {
            scene.is_none_or(|i| {
                !model.manifest().scenes[i].hidden.iter().any(|hidden| {
                    hidden.root_occurrence_id == occurrence.path.root_occurrence_id
                        && occurrence.path.steps.starts_with(&hidden.steps)
                })
            })
        })
        .collect()
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::tests::package;
    use egui_kittest::kittest::Queryable as _;
    use ketchup_view_format::{Camera as SceneCamera, Projection, Scene};

    pub(crate) fn scene_package() -> ketchup_view_format::Package {
        let mut package = package(0.1);
        let camera = SceneCamera {
            eye_mm: [98.0, 210.0, 500.0],
            target_mm: [98.0, 210.0, 309.0],
            up: [0.0, 1.0, 0.0],
            projection: Projection::Orthographic {
                short_span_mm: 30.0,
            },
        };
        package.manifest.scenes = vec![
            Scene {
                id: 1,
                name: "First scene".into(),
                camera: camera.clone(),
                hidden: vec![package.manifest.occurrences[1].path.clone()],
                style: DisplayStyle::ShadedEdges,
                section: None,
                visible_dimensions: vec![],
            },
            Scene {
                id: 2,
                name: "Second scene".into(),
                camera: SceneCamera {
                    target_mm: [508.0, 210.0, 310.0],
                    eye_mm: [508.0, 210.0, 500.0],
                    ..camera
                },
                hidden: vec![package.manifest.occurrences[0].path.clone()],
                style: DisplayStyle::Wireframe,
                section: Some(Section {
                    normal: [1.0, 0.0, 0.0],
                    offset_mm: 510.0,
                }),
                visible_dimensions: vec![],
            },
        ];
        package.manifest.start_scene = Some(1);
        package
    }

    #[test]
    fn package_open_selection_and_scenes_preserve_source_data_and_failed_open_state() {
        let package = scene_package();
        let mut bytes = Vec::new();
        package.write(&mut bytes).expect("fixture");
        let mut app = ViewerApp::new(None).with_language(Language::ENGLISH);
        app.open(bytes.as_slice()).expect("package open");
        assert_eq!(app.visible, [true, false]);
        assert_eq!(
            app.model.as_ref().expect("model").manifest(),
            &package.manifest
        );
        assert!(app.open(&b"bad data"[..]).is_err());
        assert_eq!(app.scene, Some(0));
        let model = app.model.as_ref().expect("model");
        let camera = app.camera.as_ref().expect("camera");
        let point = camera.project([88.0, 208.0, 312.0], 1.0);
        assert_eq!(
            camera
                .pick(model, [point[0], point[1]], 1.0, &app.visible, None)
                .expect("pick"),
            Some(0)
        );
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
        harness.run();
        let rect = harness.get_by_label("Model view").rect();
        let projected = harness
            .state()
            .camera
            .as_ref()
            .expect("camera")
            .project([88.0, 208.0, 314.0], rect.width() / rect.height());
        let point = egui::pos2(
            rect.left() + (projected[0] + 1.0) * rect.width() * 0.5,
            rect.top() + (1.0 - projected[1]) * rect.height() * 0.5,
        );
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(point));
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        assert_eq!(harness.state().selected, Some(0));
        harness.get_by_label("Source path: occurrence:10");
        harness.get_by_label("Material: Authored material");
        harness.get_by_label("Definition note");
        harness.get_by_label("Size: 10 × 10 × 10 mm");
        harness.get_by_label("Second scene").click();
        harness.run();
        assert_eq!(harness.state().scene, Some(1));
        assert_eq!(harness.state().visible, [false, true]);
        assert!(matches!(harness.state().style, DisplayStyle::Wireframe));
        assert_eq!(
            harness.state().section.as_ref().expect("section").offset_mm,
            510.0
        );
        assert_eq!(
            harness.state().camera.as_ref().expect("camera").center,
            [508.0, 210.0, 310.0]
        );
        harness.set_size(egui::vec2(390.0, 700.0));
        harness.run();
        let scale = harness.state().camera.as_ref().expect("camera").scale;
        let yaw = harness.state().camera.as_ref().expect("camera").yaw;
        let touch = |id, phase, x| egui::Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId(id),
            phase,
            pos: egui::pos2(x, 250.0),
            force: None,
        };
        harness.input_mut().events.extend([
            touch(1, egui::TouchPhase::Start, 150.0),
            touch(2, egui::TouchPhase::Start, 250.0),
        ]);
        harness.run();
        harness.input_mut().events.extend([
            touch(1, egui::TouchPhase::Move, 100.0),
            touch(2, egui::TouchPhase::Move, 300.0),
        ]);
        harness.run();
        let camera = harness.state().camera.as_ref().expect("camera");
        assert!(
            camera.scale < scale * 0.75,
            "pinch must zoom mobile viewport"
        );
        assert_eq!(camera.yaw, yaw, "pinch must not also orbit");
    }

    #[test]
    fn hide_isolate_views_and_pan_change_only_the_view_in_slovak() {
        let mut bytes = Vec::new();
        scene_package().write(&mut bytes).expect("fixture");
        let mut app =
            ViewerApp::new(None).with_language(Language::from_tag("sk").expect("Slovak catalog"));
        app.open(bytes.as_slice()).expect("open");
        let manifest = app.model.as_ref().expect("model").manifest().clone();
        app.selected = Some(0);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
        harness.run();
        harness.get_by_label("Rozmer: 10 × 10 × 10 mm");
        harness.get_by_label("Definition note");
        harness.get_by_label("Izolovať").click();
        harness.run();
        assert_eq!(harness.state().visible, [true, false]);
        harness.get_by_label("Skryť").click();
        harness.run();
        assert_eq!(harness.state().visible, [false, false]);
        assert_eq!(harness.state().selected, None);
        harness.get_by_label("Obnoviť scénu").click();
        harness.run();
        assert_eq!(harness.state().visible, [true, false], "scene visibility");
        harness.get_by_label("Zhora").click();
        harness.run();
        let camera = harness.state().camera.clone().expect("camera");
        assert!((camera.pitch - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let top = camera.project(camera.center, 1.0);
        assert!(
            top[0].abs() < 1e-4 && top[1].abs() < 1e-4,
            "framed on model"
        );
        let mut panned = camera.clone();
        panned.pan([100.0, 0.0], 500.0);
        let moved = panned.project(camera.center, 1.0);
        assert!(
            (moved[0] - 0.4).abs() < 1e-3 && moved[1].abs() < 1e-4,
            "a 100 px drag on a 500 px view moves the model 1:1 to the right"
        );
        assert_eq!(
            harness.state().model.as_ref().expect("model").manifest(),
            &manifest,
            "the package is never edited"
        );
    }
}
