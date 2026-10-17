//! The Viewer window: toolbar, scenes, component information and 3D view.
//! Shared by the desktop binary and the mobile entry points.

use crate::{
    camera::{Camera, StandardView},
    dimensions,
    gpu::{MeshRenderer, RenderTarget, RenderView},
    model::Model,
    review::{Review, Written},
    text::Language,
};
use eframe::egui;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{
    CommentView, DisplayStyle, InstancePath, LocalDimensions, PathCover, Section,
};
use num_traits::ToPrimitive;
use std::{
    io::{Read, Write},
    path::Path,
};

mod comments_section;
mod session;
pub use session::{SessionKeeper, SessionMark};

/// Screens narrower than this (in points) get the phone layout.
const NARROW_WIDTH: f32 = 650.0;

/// A file action the host performs for the Viewer when it has no native file
/// dialogs, as in a web page: it picks or downloads the file itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRequest {
    OpenPackage,
    OpenComments,
    SaveComments,
    SaveCommentedCopy,
    /// Hand the comments file to the system share sheet (mail, chat), offered
    /// only by hosts set up [`ViewerApp::with_host_sharing`].
    ShareComments,
    /// Save the earlier content of a file whose cut-off save could not be
    /// finished, which the host keeps where the customer cannot reach it;
    /// offered only while the host says so ([`ViewerApp::offer_kept`]).
    SaveKept,
}

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
    /// Axes and ground grid, on by default as in the Kečup editor.
    guides: bool,
    /// The customer's "Show dimensions" switch over the scene's dimensions.
    dimensions: bool,
    /// Comments on the opened package.
    review: Option<Review>,
    error: Option<Rejection>,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    renderer: Option<MeshRenderer>,
    target: Option<(RenderTarget, egui::TextureId)>,
    language: Language,
    /// `Some` when the host handles file actions; the requests wait here.
    host_files: Option<Vec<FileRequest>>,
    /// The host can share the comments file with other apps.
    host_sharing: bool,
    /// The host is still downloading the shared model.
    loading: bool,
    /// The host keeps earlier content the customer can save elsewhere.
    kept: bool,
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
            guides: true,
            dimensions: true,
            review: None,
            error: None,
            render_state,
            renderer: None,
            target: None,
            host_files: None,
            host_sharing: false,
            loading: false,
            kept: false,
        }
    }

    /// Whether to offer saving earlier content the host keeps
    /// ([`FileRequest::SaveKept`]).
    pub fn offer_kept(&mut self, offered: bool) {
        self.kept = offered;
    }

    /// While the host downloads a shared model the empty view says so.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Let the host pick and download files: the file buttons queue
    /// [`FileRequest`]s for [`Self::take_file_requests`] instead of opening dialogs.
    #[must_use]
    pub fn with_host_files(mut self) -> Self {
        self.host_files = Some(vec![]);
        self
    }

    /// Host files plus a button that sends the comments through the system
    /// share sheet, as phones offer.
    #[must_use]
    pub fn with_host_sharing(mut self) -> Self {
        self.host_sharing = true;
        self.with_host_files()
    }

    /// File actions the customer asked for since the last call.
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        self.host_files
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    fn request_file(&mut self, request: FileRequest) {
        if let Some(requests) = &mut self.host_files {
            requests.push(request);
        }
    }

    /// Show a failure the host met while reading or writing files.
    pub fn report(&mut self, error: Rejection) {
        self.error = Some(error);
    }

    /// The failure the Viewer shows now, if any.
    pub fn error(&self) -> Option<&Rejection> {
        self.error.as_ref()
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
    /// Comments saved into a commented copy are opened with it.
    pub fn open(&mut self, reader: impl Read) -> Result<(), Rejection> {
        let (package, comments) =
            ketchup_view_format::IdentifiedPackage::read_with_comments(reader)?;
        self.open_package(package, comments)
    }

    fn open_package(
        &mut self,
        package: ketchup_view_format::IdentifiedPackage,
        comments: Option<ketchup_view_format::Comments>,
    ) -> Result<(), Rejection> {
        let model = Model::from_identified(package)?;
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
        let review = match comments {
            Some(comments) => Review::with(comments),
            None => Review::new(&model)?,
        };
        self.visible = visibility(&model, scene);
        self.style = scene.map_or(DisplayStyle::ShadedEdges, |i| {
            model.manifest().scenes[i].style.clone()
        });
        self.section = scene.and_then(|i| model.manifest().scenes[i].section.clone());
        self.model = Some(model);
        self.review = Some(review);
        self.renderer = renderer;
        self.camera = Some(camera);
        self.scene = scene;
        self.selected = None;
        self.error = None;
        Ok(())
    }

    /// Open a local `.ketchup-view`, or a `.ketchup-comments` with the
    /// package it was written on: the open one, else the matching package in
    /// the same folder. The current model stays when nothing matches.
    pub fn open_path(&mut self, path: &Path) -> Result<(), Rejection> {
        let is_comments = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("ketchup-comments"));
        if !is_comments {
            return self.open(std::io::BufReader::new(open_file(path)?));
        }
        let mut bytes = Vec::new();
        open_file(path)?
            .read_to_end(&mut bytes)
            .map_err(|error| file_error(path, error))?;
        let wanted = ketchup_view_format::Comments::read_unbound(bytes.as_slice())?
            .export
            .package_sha256;
        let open = self
            .model
            .as_ref()
            .is_some_and(|model| model.export().payload_sha256() == wanted);
        if !open {
            let package = sibling_package(path, &wanted)?;
            self.open_package(package, None)?;
        }
        self.open_comments(bytes.as_slice())
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

    /// The desktop Open button with the system file dialog.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn open_with_dialog(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.t("viewer-open")).clicked()
            && self.may_discard_comments()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter(self.t("viewer-file-filter"), &["ketchup-view"])
                .pick_file()
            && let Err(error) = self.open_path(&path)
        {
            self.error = Some(error);
        }
    }

    /// Standard views and the axes/grid and dimensions switches.
    fn view_controls(&mut self, ui: &mut egui::Ui) {
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
        let guides = self.t("viewer-guides");
        ui.checkbox(&mut self.guides, guides);
        if self.model.is_some() {
            let label = self.t("viewer-dimensions");
            ui.checkbox(&mut self.dimensions, label);
        }
    }

    /// Toolbar with scenes on top, part information at the side (bottom on
    /// narrow phone screens) and the 3D view in the middle.
    pub fn show(&mut self, ctx: &egui::Context) {
        let narrow = ctx.screen_rect().width() < NARROW_WIDTH;
        egui::TopBottomPanel::top("viewer_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if !narrow {
                    ui.heading(self.t("viewer-title"));
                }
                if self.host_files.is_some() {
                    if ui.button(self.t("viewer-open")).clicked() && self.may_discard_comments() {
                        self.request_file(FileRequest::OpenPackage);
                    }
                } else {
                    #[cfg(any(target_os = "windows", target_os = "macos"))]
                    self.open_with_dialog(ui);
                }
                if ui.button(self.t("viewer-fit")).clicked()
                    && let Some(model) = &self.model
                {
                    self.camera = Some(Camera::fit(model));
                }
                // On a phone the views and switches fold into one menu, so the
                // toolbar stays one row and the model keeps the screen.
                if narrow {
                    ui.menu_button(self.t("viewer-views"), |ui| self.view_controls(ui));
                } else {
                    self.view_controls(ui);
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
            // With a model on screen the failure was a file action, not the model.
            if let Some(error) = &self.error {
                let headline = if self.model.is_some() {
                    "viewer-error-file"
                } else {
                    "viewer-error"
                };
                ui.colored_label(egui::Color32::LIGHT_RED, self.t(headline));
                ui.small(error.to_string());
                ui.small(error.fix_hint_text());
            }
            if self.kept
                && self.host_files.is_some()
                && ui.button(self.t("viewer-save-kept")).clicked()
            {
                self.request_file(FileRequest::SaveKept);
            }
        });
        if narrow {
            egui::TopBottomPanel::bottom("viewer_info_mobile")
                .max_height(160.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.information(ui);
                        self.comments(ui);
                    })
                });
        } else {
            // A fixed minimum width keeps the side from jumping wider when a comment starts.
            egui::SidePanel::right("viewer_info")
                .default_width(220.0)
                .min_width(220.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.information(ui);
                        self.comments(ui);
                    })
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
        let prompt = self.t(if self.loading {
            "viewer-loading"
        } else {
            "viewer-open-prompt"
        });
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
        if response.double_clicked() {
            *camera = Camera::fit(model);
        }
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
            // As in SketchUp: left or middle button orbits, Shift with either
            // pans; the right button pans too, there is no context menu.
            let (delta, pan) = ui.input(|input| {
                (
                    input.pointer.delta(),
                    input.pointer.secondary_down() || input.modifiers.shift,
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
            if factor != 1.0 {
                // Zoom towards the pointer, or the pinch centre on a phone.
                let anchor = touch
                    .filter(|touch| touch.num_touches > 1)
                    .map(|touch| touch.center_pos)
                    .or_else(|| response.hover_pos());
                match anchor {
                    Some(point) => camera.zoom_at(factor, ndc(rect, point), aspect),
                    None => camera.zoom(factor),
                }
            }
        }
        if response.clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            let at = ndc(rect, point);
            let hit = camera.pick_point(model, at, aspect, &self.visible, self.section.as_ref());
            match &mut self.review {
                // While placing an arrow, clicks place its ends, not a selection.
                Some(review)
                    if review
                        .draft
                        .as_ref()
                        .is_some_and(|draft| draft.placing.is_some()) =>
                {
                    review.place_arrow_end(model, hit, camera.point_on_view_plane(at, aspect));
                }
                _ => self.selected = hit.map(|(index, _)| index),
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
                        guides: self.guides,
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
        if self.dimensions {
            let mut placed =
                dimensions::placed(model, self.scene, &self.visible, self.section.as_ref());
            if let Some(index) = self.selected {
                placed.extend(dimensions::overall(model, index));
            }
            paint_dimensions(&ui.painter_at(rect), rect, camera, &placed);
        }
        if let Some(review) = &self.review {
            let arrows = review
                .visible_arrows()
                .iter()
                .filter_map(|arrow| {
                    Some((
                        dimensions::world_point(model, &arrow.tail, &self.visible)?,
                        dimensions::world_point(model, &arrow.tip, &self.visible)?,
                    ))
                })
                .collect::<Vec<_>>();
            paint_arrows(&ui.painter_at(rect), rect, camera, &arrows);
        }
    }

    /// What is shown now, for a new comment.
    fn current_view(&self) -> Option<(CommentView, Option<InstancePath>)> {
        let (model, camera) = (self.model.as_ref()?, self.camera.as_ref()?);
        let mut view = CommentView {
            camera: camera.to_scene(),
            hidden: vec![],
            style: self.style.clone(),
            section: self.section.clone(),
            scene: None,
            shown: None,
        };
        view.set_visibility(model.manifest(), &self.visible);
        let occurrences = &model.manifest().occurrences;
        Some((
            view,
            self.selected.map(|index| occurrences[index].path.clone()),
        ))
    }

    /// Restore the view comment `id` was written in.
    fn show_comment(&mut self, id: u64) -> Result<(), Rejection> {
        let (Some(model), Some(review)) = (&self.model, &mut self.review) else {
            return Ok(());
        };
        let Some(comment) = review.comments.comments.iter().find(|c| c.id == id) else {
            return Ok(());
        };
        let camera = Camera::from_scene(model, &comment.view.camera)?;
        let occurrences = &model.manifest().occurrences;
        self.visible = comment.view.visibility(model.manifest());
        self.style = comment.view.style.clone();
        self.section = comment.view.section.clone();
        self.selected = comment
            .selected
            .as_ref()
            .and_then(|path| occurrences.iter().position(|o| &o.path == path));
        self.camera = Some(camera);
        self.scene = None;
        review.active = Some(id);
        Ok(())
    }

    /// Comments returned for the opened package; the current ones stay on failure.
    pub fn open_comments(&mut self, reader: impl Read) -> Result<(), Rejection> {
        let Some(model) = &self.model else {
            return Err(
                Rejection::new("viewer.comments.open", RejectionPhase::Request)
                    .reason("Open the Viewer package the comments belong to first.")
                    .fix_hint("Open the .ketchup-view, then its .ketchup-comments."),
            );
        };
        self.review = Some(Review::open(reader, model)?);
        Ok(())
    }

    /// Write the comments for the author; the package file is never touched.
    /// `None` when nothing is open. Pass the result to
    /// [`Self::comments_written`] once the host has kept the file.
    pub fn save_comments(&self, writer: impl Write) -> Result<Option<Written>, Rejection> {
        match (&self.model, &self.review) {
            (Some(model), Some(review)) => review.save(writer, model).map(Some),
            _ => Ok(None),
        }
    }

    /// Write a copy of the package with every comment in it; the opened
    /// file is untouched unless the customer chooses its path. Saved as
    /// [`Self::save_comments`].
    pub fn save_commented_copy(&self, writer: impl Write) -> Result<Option<Written>, Rejection> {
        match (&self.model, &self.review) {
            (Some(model), Some(review)) => review.save_copy(writer, model).map(Some),
            _ => Ok(None),
        }
    }

    /// The host kept the file `written` came from; a cancelled save dialog
    /// or a share sheet, whose outcome the host never learns, keeps nothing.
    pub fn comments_written(&mut self, written: Option<Written>) {
        if let (Some(review), Some(written)) = (&mut self.review, written) {
            review.written(written);
        }
    }

    /// Components, scenes and comments of the opened package, for the
    /// headless `--verify-open` check of an installed Viewer.
    pub fn opened_counts(&self) -> Option<[usize; 3]> {
        let model = self.model.as_ref()?;
        Some([
            model.manifest().occurrences.len(),
            model.manifest().scenes.len(),
            self.review
                .as_ref()
                .map_or(0, |review| review.comments.comments.len()),
        ])
    }

    /// Unsaved comments, or a changed comment being written, would be lost by
    /// opening another package or comments file, or by closing.
    pub fn has_unsaved_comments(&self) -> bool {
        self.review.as_ref().is_some_and(Review::has_unkept_work)
    }

    /// Whether the opened comments and the one being written may be replaced
    /// now; the first attempt with unsaved work only shows a warning.
    pub fn may_discard_comments(&mut self) -> bool {
        let allowed = self.review.as_mut().is_none_or(Review::may_discard);
        if !allowed {
            self.error = Some(
                Rejection::new("viewer.comments.unsaved", RejectionPhase::Request)
                    .reason("The comments, or the comment being written, have not been saved.")
                    .fix_hint("Save them, or repeat the action to discard them."),
            );
        }
        allowed
    }
}

/// Comment arrows from tail to tip, drawn over the 3D image.
fn paint_arrows(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    arrows: &[([f64; 3], [f64; 3])],
) {
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    let stroke = egui::Stroke::new(3.0_f32, egui::Color32::from_rgb(64, 200, 255));
    let screen = |point: [f64; 3]| {
        let p = camera.project(point.map(|v| v.to_f32().unwrap_or(f32::MAX)), aspect);
        ((0.0..=1.0).contains(&p[2]) && p[0].is_finite() && p[1].is_finite()).then(|| {
            egui::pos2(
                rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
                rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
            )
        })
    };
    for &(tail, tip) in arrows {
        let (Some(tail), Some(tip)) = (screen(tail), screen(tip)) else {
            continue;
        };
        painter.line_segment([tail, tip], stroke);
        let back = (tail - tip).normalized();
        if back.is_finite() && back != egui::Vec2::ZERO {
            let base = tip + back * 16.0;
            let across = back.rot90() * 7.0;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, base + across, base - across],
                stroke.color,
                egui::Stroke::NONE,
            ));
        }
        painter.circle_filled(tip, 3.0, stroke.color);
    }
}

/// Extension lines, the dimension line with arrows and the value label,
/// drawn over the 3D image; points behind the camera are left out.
fn paint_dimensions(
    painter: &egui::Painter,
    rect: egui::Rect,
    camera: &Camera,
    placed: &[dimensions::Placed],
) {
    let aspect = rect.width().max(1.0) / rect.height().max(1.0);
    let line = egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(255, 214, 102));
    let screen = |point: [f64; 3]| {
        let p = camera.project(point.map(|v| v.to_f32().unwrap_or(f32::MAX)), aspect);
        ((0.0..=1.0).contains(&p[2]) && p[0].is_finite() && p[1].is_finite()).then(|| {
            egui::pos2(
                rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
                rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
            )
        })
    };
    for dimension in placed {
        let shifted = |p: [f64; 3]| std::array::from_fn(|i| p[i] + dimension.offset[i]);
        let (Some(from), Some(to), Some(a), Some(b)) = (
            screen(dimension.from),
            screen(dimension.to),
            screen(shifted(dimension.from)),
            screen(shifted(dimension.to)),
        ) else {
            continue;
        };
        painter.line_segment([from, a], line);
        painter.line_segment([to, b], line);
        painter.line_segment([a, b], line);
        let along = (b - a).normalized();
        if along.is_finite() && along != egui::Vec2::ZERO {
            let across = along.rot90() * 4.0;
            for (tip, back) in [(a, along), (b, -along)] {
                let base = tip + back * 10.0;
                painter.line_segment([tip, base + across], line);
                painter.line_segment([tip, base - across], line);
            }
        }
        let galley = painter.layout_no_wrap(
            dimension.text.clone(),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        let label = egui::Rect::from_center_size(a.lerp(b, 0.5), galley.size()).expand(3.0);
        painter.rect_filled(label, 3.0, egui::Color32::from_black_alpha(190));
        painter.galley(
            label.min + egui::vec2(3.0, 3.0),
            galley,
            egui::Color32::WHITE,
        );
    }
}

/// A screen point as normalized device coordinates of the 3D view `rect`.
fn ndc(rect: egui::Rect, point: egui::Pos2) -> [f32; 2] {
    [
        (point.x - rect.left()) / rect.width() * 2.0 - 1.0,
        1.0 - (point.y - rect.top()) / rect.height() * 2.0,
    ]
}

/// Millimetres with at most two decimals and no trailing zeros: `600`, `18.5`.
fn millimetres(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Per occurrence: visible unless the scene hides it or one of its ancestors.
fn visibility(model: &Model, scene: Option<usize>) -> Vec<bool> {
    let hidden = scene.map(|i| PathCover::new(&model.manifest().scenes[i].hidden));
    model
        .manifest()
        .occurrences
        .iter()
        .map(|occurrence| hidden.as_ref().is_none_or(|h| !h.covers(&occurrence.path)))
        .collect()
}

fn file_error(path: &Path, error: std::io::Error) -> Rejection {
    Rejection::new("viewer.file.open", RejectionPhase::Io)
        .target(path.display().to_string())
        .reason("The local file could not be opened.")
        .fix_hint("Choose an accessible .ketchup-view or .ketchup-comments file.")
        .caused_by(error)
}

fn open_file(path: &Path) -> Result<std::fs::File, Rejection> {
    std::fs::File::open(path).map_err(|error| file_error(path, error))
}

/// The `.ketchup-view` next to `comments` whose payload has `sha256`; other
/// packages and unreadable files are skipped, never matched by name.
fn sibling_package(
    comments: &Path,
    sha256: &str,
) -> Result<ketchup_view_format::IdentifiedPackage, Rejection> {
    let folder = comments.parent().unwrap_or_else(|| Path::new("."));
    let entries = std::fs::read_dir(folder).map_err(|error| file_error(folder, error))?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("ketchup-view"))
        })
        .filter_map(|path| {
            let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
            Some(ketchup_view_format::IdentifiedPackage::read_with_comments(file).ok()?.0)
        })
        .find(|package| package.payload_sha256() == sha256)
        .ok_or_else(|| {
            Rejection::new("viewer.comments.package", RejectionPhase::Request)
                .target(comments.display().to_string())
                .reason("The Viewer package these comments were written on is not in this folder.")
                .fix_hint("Put the .ketchup-comments next to its .ketchup-view, or open that package first.")
        })
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Closing with unsaved comments warns once; closing again discards them.
        if ctx.input(|input| input.viewport().close_requested()) && !self.may_discard_comments() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.show(ctx);
    }
}

#[cfg(test)]
pub(crate) mod tests;
