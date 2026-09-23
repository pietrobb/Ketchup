//! CAD-only readback. The GUI framebuffer is never an image source.
use super::*;
use crate::{CameraViewState, ProjectedEdge, ProjectedFace, RenderBox};
use egui::{ColorImage, Rect, Shape};
use ketchup_core::{
    document::{InstancePath, OccurrenceId},
    exact_product::ExactBodyPackage,
    topology::TopologicalElementKind,
};
use ketchup_interaction::{Vec3, projection::CanonicalInteractionProjection};
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "image_privacy_tests.rs"]
mod privacy_tests;
#[path = "image_target.rs"]
mod target;

const MAX_SOURCE_PIXELS: usize = 16_777_216;

#[derive(Default)]
pub(super) struct ImageState {
    pending: Option<ImageRequest>,
    painted: Option<Result<Painted, &'static str>>,
}
impl ImageState {
    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn revoke(&mut self) {
        if let Some(request) = self.pending.take() {
            drop(request.capture);
        }
        self.painted = None;
    }
    fn purge_abandoned(&mut self, session: u64) {
        if self.pending.as_ref().is_some_and(|r| {
            r.queued.session != session || r.queued.cancelled.load(Ordering::Acquire)
        }) {
            self.revoke();
        }
    }
}
struct ImageRequest {
    queued: Queued,
    deadline: Instant,
    initial: VisualState,
    mode: CaptureMode,
    max_side_px: u32,
    framing: ImageFraming,
    detail: Option<ResolvedImageDetail>,
    nonce: CaptureNonce,
    capture: Option<(Painted, target::Readback, bool)>,
}
#[derive(Clone)]
struct ResolvedImageDetail {
    request: ImageDetailTarget,
    reference_id: String,
    world_points: Vec<Vec3>,
}
#[derive(Clone, Debug, PartialEq)]
struct CaptureNonce([u8; 32]);
#[derive(Clone, PartialEq)]
struct VisualState {
    stamp: Stamp,
    camera: CameraViewState,
    distance: f64,
    selection: Vec<u64>,
    primary: Option<SelectionId>,
    exact: u64,
    topology: u64,
    exact_complete: bool,
    evaluating: bool,
    theme: String,
    #[cfg(feature = "named-product-fixtures")]
    beam: u64,
}
impl VisualState {
    fn read(app: &KetchupApp) -> Result<Self, &'static str> {
        Ok(Self {
            stamp: app.live_bridge_stamp(),
            camera: app.camera_view_state(),
            distance: app.camera_distance_mm,
            selection: LiveBridge::selection(app)?,
            primary: app.selection.primary.clone(),
            exact: app.exact_results.contents_stamp(),
            topology: app.topology_results.contents_stamp(),
            exact_complete: app.exact_source.as_ref()
                == Some(&ketchup_application::evaluation::exact_source(
                    &app.document.current(),
                )),
            evaluating: app.exact_task.is_some(),
            theme: format!("{:?}", app.theme),
            #[cfg(feature = "named-product-fixtures")]
            beam: app.beam_exact_results.contents_stamp(),
        })
    }
}
struct Painted {
    state: VisualState,
    rect: Rect,
    crop: Rect,
    screen: Rect,
    ppp: f32,
    pass: u64,
    shapes: Vec<egui::epaint::ClippedShape>,
    callbacks: usize,
    jobs: Vec<egui::epaint::ClippedPrimitive>,
    atlas: ColorImage,
}
impl Painted {
    fn same(&self, other: &Self) -> bool {
        self.state == other.state
            && self.rect == other.rect
            && self.crop == other.crop
            && self.screen == other.screen
            && self.ppp == other.ppp
            && self.shapes == other.shapes
            && self.callbacks == other.callbacks
    }
}
// This only compares our private CAD output between frames. It is NOT a privacy
// whitelist. Callback provenance comes from paint_scene_base_layers, never from
// inspecting or downcasting arbitrary shapes in the application's graphics.
fn normalize(shape: &mut Shape, callbacks: &mut usize) {
    match shape {
        Shape::Callback(_) => {
            *callbacks += 1;
            *shape = Shape::Noop;
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                normalize(shape, callbacks);
            }
        }
        _ => {}
    }
}

fn selection_crop(
    app: &KetchupApp,
    viewport: Rect,
    selection: &[u64],
    primary: Option<&SelectionId>,
    faces: &[ProjectedFace],
    edges: &[ProjectedEdge],
) -> Result<Rect, &'static str> {
    let mut target = primary.and_then(|primary| {
        faces
            .iter()
            .filter(|face| &face.selection == primary)
            .flat_map(|face| face.polygon.points())
            .chain(
                edges
                    .iter()
                    .filter(|edge| &edge.selection == primary)
                    .flat_map(|edge| edge.points.iter()),
            )
            .map(|point| Rect::from_min_max(*point, *point))
            .reduce(|left, right| left.union(right))
    });
    if target.is_none() {
        target = selection
            .iter()
            .filter_map(|id| app.occurrence_box_geometry(*id))
            .flat_map(|(origin, size)| {
                crate::box_corners(size.x, size.y, size.z)
                    .map(|point| app.project_to_screen(point + origin, viewport))
            })
            .map(|point| Rect::from_min_max(point, point))
            .reduce(|left, right| left.union(right));
    }
    let target = target.ok_or("invalid_image_framing")?;
    let padding = target.width().max(target.height()).mul_add(0.12, 8.0);
    let crop = target.expand(padding).intersect(viewport.shrink(2.0));
    if crop.width() < 2.0 || crop.height() < 2.0 {
        return Err("invalid_image_framing");
    }
    Ok(crop)
}

fn detail_crop(
    viewport: Rect,
    app: &KetchupApp,
    detail: &ResolvedImageDetail,
) -> Result<Rect, &'static str> {
    let target = detail
        .world_points
        .iter()
        .map(|point| app.project_to_screen(*point, viewport))
        .map(|point| Rect::from_min_max(point, point))
        .reduce(|left, right| left.union(right))
        .ok_or("invalid_image_framing")?;
    let padding = target.width().max(target.height()).mul_add(0.12, 8.0);
    let crop = target.expand(padding).intersect(viewport.shrink(2.0));
    if crop.width() < 2.0 || crop.height() < 2.0 {
        return Err("invalid_image_framing");
    }
    Ok(crop)
}

fn resolve_detail(
    app: &KetchupApp,
    query: &ModelQuery,
    target: &ImageDetailTarget,
) -> Result<ResolvedImageDetail, &'static str> {
    if target.occurrence_id == 0
        || target.entity_id == 0
        || !matches!(target.kind, EntityKind::Edges | EntityKind::Faces)
    {
        return Err("invalid_image_framing");
    }
    let snapshot = app.document.current();
    let detail = query
        .detail_with_topology(
            &snapshot,
            &app.topology_results,
            target.kind,
            target.entity_id,
        )
        .map_err(|_| "invalid_image_framing")?;
    let item = detail.get("item").ok_or("invalid_image_framing")?;
    let definition_id = item["definition_id"]
        .as_u64()
        .ok_or("invalid_image_framing")?;
    let ordinal = item["ordinal"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("invalid_image_framing")?;
    let reference_id = item["reference_id"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("invalid_image_framing")?
        .to_owned();
    let occurrence_path = InstancePath::root(OccurrenceId(target.occurrence_id));
    let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
    let occurrence = projection
        .occurrences()
        .iter()
        .find(|occurrence| occurrence.instance_path == occurrence_path)
        .filter(|occurrence| occurrence.body.definition_id.0 == definition_id)
        .ok_or("invalid_image_framing")?;
    let kind = match target.kind {
        EntityKind::Edges => TopologicalElementKind::Edge,
        EntityKind::Faces => TopologicalElementKind::Face,
        _ => return Err("invalid_image_framing"),
    };
    let mut local_bounds = None;
    for (key, package) in app
        .topology_results
        .body_values(&snapshot)
        .map_err(|_| "invalid_image_framing")?
    {
        let ExactBodyPackage::Graph(package) = package.as_ref() else {
            continue;
        };
        if key.definition_id.0 != definition_id {
            continue;
        }
        let Some(reference) = package
            .topological_references
            .iter()
            .filter(|reference| reference.kind == kind)
            .nth(ordinal as usize)
        else {
            continue;
        };
        if reference.lineage_digest != reference_id {
            continue;
        }
        if local_bounds.is_some() {
            return Err("invalid_image_framing");
        }
        local_bounds = Some(match target.kind {
            EntityKind::Edges => package
                .edge_evidence
                .iter()
                .find(|evidence| evidence.edge_ordinal == ordinal)
                .map(|evidence| evidence.bounds_mm)
                .ok_or("invalid_image_framing")?,
            EntityKind::Faces => {
                let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
                let mut found = false;
                for triangle in package
                    .triangles
                    .iter()
                    .zip(&package.triangle_face_ordinals)
                    .filter(|(_, face_ordinal)| **face_ordinal == ordinal)
                    .map(|(triangle, _)| triangle)
                {
                    for index in triangle.vertex_indices {
                        let point = package
                            .vertices
                            .get(index as usize)
                            .ok_or("invalid_image_framing")?
                            .position_mm;
                        found = true;
                        for axis in 0..3 {
                            bounds[0][axis] = bounds[0][axis].min(point[axis]);
                            bounds[1][axis] = bounds[1][axis].max(point[axis]);
                        }
                    }
                }
                found.then_some(bounds).ok_or("invalid_image_framing")?
            }
            _ => return Err("invalid_image_framing"),
        });
    }
    let bounds = local_bounds.ok_or("invalid_image_framing")?;
    let matrix = occurrence.canonical_world_transform.matrix();
    let mut world_points = Vec::with_capacity(8);
    for x in [bounds[0][0], bounds[1][0]] {
        for y in [bounds[0][1], bounds[1][1]] {
            for z in [bounds[0][2], bounds[1][2]] {
                world_points.push(Vec3::new(
                    matrix[0] * x + matrix[1] * y + matrix[2] * z + matrix[3],
                    matrix[4] * x + matrix[5] * y + matrix[6] * z + matrix[7],
                    matrix[8] * x + matrix[9] * y + matrix[10] * z + matrix[11],
                ));
            }
        }
    }
    Ok(ResolvedImageDetail {
        request: target.clone(),
        reference_id,
        world_points,
    })
}

impl KetchupApp {
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn headless_live_image_command(&mut self, command: AppCommand) {
        self.dispatch_command(command);
    }
    pub(crate) fn begin_live_image_frame(&mut self) {
        if let Some(bridge) = self.live_bridge.as_mut() {
            bridge.image.painted = None;
        }
    }
    pub(crate) fn live_offscreen_image_pending(&self) -> bool {
        self.live_bridge
            .as_ref()
            .and_then(|bridge| bridge.image.pending.as_ref())
            .is_some_and(|request| request.mode == CaptureMode::Offscreen)
    }
    /// Deliberately accepts CAD data, NOT a layer/range from the GUI context.
    /// No menus, chat, viewport readouts, debug plugins or externally supplied
    /// shapes can paint into this private context or its eventual GPU target.
    pub(crate) fn record_live_image_scene(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        boxes: &[RenderBox],
        faces: &[ProjectedFace],
        edges: &[ProjectedEdge],
        plan: Option<Arc<crate::InstancedRenderPlan>>,
    ) {
        let Some((mode, framing, detail)) = self
            .live_bridge
            .as_ref()
            .and_then(|bridge| bridge.image.pending.as_ref())
            .map(|request| (request.mode, request.framing, request.detail.clone()))
        else {
            return;
        };
        let painted = (|| {
            let screen = ui.ctx().screen_rect();
            let ppp = ui.ctx().pixels_per_point();
            if mode == CaptureMode::VisibleViewport
                && (!ui.is_visible()
                    || !ui.clip_rect().contains_rect(rect)
                    || !screen.contains_rect(rect))
            {
                return Err("hidden_viewport");
            }
            target::dimensions(screen, ppp)?;
            let cad = egui::Context::default();
            cad.set_pixels_per_point(ppp);
            let output = cad.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ctx| {
                    let painter =
                        egui::Painter::new(ctx.clone(), egui::LayerId::background(), rect);
                    let palette = self.palette();
                    let (inner, outer) = if self.white_background_visible {
                        (egui::Color32::WHITE, egui::Color32::WHITE)
                    } else {
                        (palette.viewport_inner, palette.viewport_outer)
                    };
                    crate::theme::paint_vignette(&painter, rect, inner, outer);
                    self.paint_projected_shadows(&painter, rect, boxes);
                    self.paint_scene_base_layers(&painter, rect, plan.clone());
                    self.paint_projected_faces(&painter, faces);
                    self.paint_projected_edges(&painter, edges);
                    self.paint_viewport_fog(&painter, rect);
                    self.paint_projected_selection(&painter, edges);
                },
            );
            if output.shapes.is_empty() || output.shapes.len() > 100_000 {
                return Err("response_limit");
            }
            let jobs = cad.tessellate(output.shapes.clone(), ppp);
            let atlas = cad.fonts(|f| f.image());
            let mut shapes = output.shapes;
            let mut callbacks = 0;
            for entry in &mut shapes {
                normalize(&mut entry.shape, &mut callbacks);
            }
            if callbacks > 1 {
                return Err("unsupported_image_renderer");
            }
            let state = VisualState::read(self)?;
            let crop = match framing {
                ImageFraming::Viewport => rect.shrink(2.0),
                ImageFraming::Selection => selection_crop(
                    self,
                    rect,
                    &state.selection,
                    state.primary.as_ref(),
                    faces,
                    edges,
                )?,
                ImageFraming::DetailSelection => {
                    detail_crop(rect, self, detail.as_ref().ok_or("invalid_image_framing")?)?
                }
            };
            Ok(Painted {
                state,
                rect,
                crop,
                screen,
                ppp,
                pass: ui.ctx().cumulative_pass_nr(),
                shapes,
                callbacks,
                jobs,
                atlas,
            })
        })();
        if let Some(bridge) = self.live_bridge.as_mut() {
            bridge.image.painted = Some(painted);
        }
    }
    pub(crate) fn finish_live_image_frame(&mut self, ctx: &egui::Context) {
        let Some(mut bridge) = self.live_bridge.take() else {
            return;
        };
        bridge.finish_image(self, ctx);
        self.live_bridge = Some(bridge);
    }
}
impl LiveBridge {
    pub(super) fn request_image(&mut self, app: &KetchupApp, ctx: &egui::Context, queued: Queued) {
        self.image.purge_abandoned(self.session);
        let result = (|| {
            let Request::Image {
                expected,
                image_protocol_version,
                capture_mode,
                max_side_px,
                framing,
                detail_target,
            } = &queued.request
            else {
                unreachable!()
            };
            if *image_protocol_version != IMAGE_PROTOCOL_VERSION {
                return Err("unsupported_image_protocol");
            }
            if !(MIN_IMAGE_SIDE_PX..=MAX_IMAGE_SIDE_PX).contains(max_side_px) {
                return Err("invalid_image_dimensions");
            }
            Self::guard(app, expected)?;
            Self::available(app, ctx.wants_keyboard_input() || ctx.is_using_pointer())?;
            if self.image.pending.is_some() {
                return Err("busy");
            }
            let mut nonce = [0; 32];
            getrandom::fill(&mut nonce).map_err(|_| "image_unavailable")?;
            let initial = VisualState::read(app)?;
            let detail = match (*framing, detail_target.as_ref()) {
                (ImageFraming::Viewport | ImageFraming::Selection, None) => None,
                (ImageFraming::DetailSelection, Some(target)) => {
                    Some(resolve_detail(app, &self.query, target)?)
                }
                _ => return Err("invalid_image_framing"),
            };
            if *framing == ImageFraming::Selection && initial.selection.is_empty() {
                return Err("invalid_image_framing");
            }
            Ok((
                initial,
                *capture_mode,
                *max_side_px,
                *framing,
                detail,
                CaptureNonce(nonce),
            ))
        })();
        match result {
            Ok((initial, mode, max_side_px, framing, detail, nonce)) => {
                self.image.pending = Some(ImageRequest {
                    queued,
                    deadline: Instant::now() + Duration::from_secs(5),
                    initial,
                    mode,
                    max_side_px,
                    framing,
                    detail,
                    nonce,
                    capture: None,
                })
            }
            Err(code) => {
                let _ = queued.reply.try_send(Response::error(queued.id, code));
            }
        }
        ctx.request_repaint();
    }
    fn finish_image(&mut self, app: &KetchupApp, ctx: &egui::Context) {
        let Some(mut request) = self.image.pending.take() else {
            return;
        };
        if request.queued.cancelled.load(Ordering::Acquire)
            || request.queued.session != self.session
        {
            return;
        }
        let result = (|| {
            if Instant::now() >= request.deadline {
                return Err("image_timeout");
            }
            Self::guard(app, &request.initial.stamp)?;
            Self::available(app, ctx.wants_keyboard_input() || ctx.is_using_pointer())?;
            if request.mode == CaptureMode::VisibleViewport
                && (ctx.viewport_id() != egui::ViewportId::ROOT
                    || ctx.input(|i| i.viewport().minimized == Some(true) || !i.focused))
            {
                return Err("hidden_viewport");
            }
            let painted = self.image.painted.take().ok_or(
                if request.mode == CaptureMode::VisibleViewport {
                    "hidden_viewport"
                } else {
                    "image_unavailable"
                },
            )??;
            if VisualState::read(app)? != painted.state {
                return Err("stale_image");
            }
            if let Some((capture, readback, direct)) = &request.capture {
                if !capture.same(&painted) {
                    return Err("stale_image");
                }
                if let Some(pixels) = readback.take()? {
                    if pixels.nonce != request.nonce
                        || pixels.pass != capture.pass
                        || painted.pass <= capture.pass
                    {
                        return Err("invalid_image_callback");
                    }
                    return thumbnail(
                        capture,
                        &pixels.image,
                        request.mode,
                        request.max_side_px,
                        request.framing,
                        request.detail.as_ref(),
                        *direct,
                    )
                    .map(Some);
                }
                // A discarded pass never executes its GPU callback. Do not attach
                // an old stamp to a later pass; fail closed rather than reuse it.
            } else {
                if painted.state.camera != request.initial.camera
                    || painted.state.selection != request.initial.selection
                    || painted.state.primary != request.initial.primary
                {
                    return Err("stale_image");
                }
                let direct = request.mode == CaptureMode::Offscreen
                    && app.wgpu_device.is_some()
                    && app.wgpu_queue.is_some();
                let readback = if direct {
                    target::submit(
                        app.wgpu_device.as_ref().unwrap(),
                        app.wgpu_queue.as_ref().unwrap(),
                        &painted,
                        request.nonce.clone(),
                        request.queued.cancelled.clone(),
                    )?
                } else {
                    target::schedule(
                        ctx,
                        &painted,
                        request.nonce.clone(),
                        request.queued.cancelled.clone(),
                    )?
                };
                request.capture = Some((painted, readback, direct));
            }
            Ok(None)
        })();
        match result {
            Ok(None) => {
                self.image.pending = Some(request);
                ctx.request_repaint_after(Duration::from_millis(10));
            }
            result => {
                // Revoke retained callbacks, including ones from discarded passes.
                let mut response = match result {
                    Ok(Some(value)) => Response {
                        version: 1,
                        id: request.queued.id,
                        ok: true,
                        stamp: None,
                        result: Some(value),
                        error: None,
                    },
                    Err(code) => Response::error(request.queued.id, code),
                    Ok(None) => unreachable!(),
                };
                response.stamp = Some(request.capture.as_ref().map_or_else(
                    || app.live_bridge_stamp(),
                    |(p, _, _)| p.state.stamp.clone(),
                ));
                if serde_json::to_vec(&response).map_or(true, |v| v.len() > MAX_IMAGE_FRAME_BYTES) {
                    response = Response::error(request.queued.id, "response_limit");
                }
                if !request.queued.cancelled.load(Ordering::Acquire) {
                    let _ = request.queued.reply.try_send(response);
                }
                drop(request.capture);
            }
        }
    }
}
fn thumbnail(
    capture: &Painted,
    image: &ColorImage,
    mode: CaptureMode,
    max_side_px: u32,
    framing: ImageFraming,
    detail: Option<&ResolvedImageDetail>,
    direct: bool,
) -> Result<Value, &'static str> {
    let ppp = capture.ppp;
    let expected = target::dimensions(capture.screen, ppp)?.map(|v| v as usize);
    let count = image.size[0]
        .checked_mul(image.size[1])
        .ok_or("response_limit")?;
    if image.size != expected || count > MAX_SOURCE_PIXELS || image.pixels.len() != count {
        return Err("invalid_image_dimensions");
    }
    // This is an isolated CAD texture, never a crop of the GUI framebuffer.
    let rect = capture.crop;
    let [x0, y0, x1, y1] = [
        (rect.min.x * ppp).ceil() as usize,
        (rect.min.y * ppp).ceil() as usize,
        (rect.max.x * ppp).floor() as usize,
        (rect.max.y * ppp).floor() as usize,
    ];
    if x1 <= x0 || y1 <= y0 || x1 > image.size[0] || y1 > image.size[1] {
        return Err("invalid_image_dimensions");
    }
    let (sw, sh) = (x1 - x0, y1 - y0);
    let scale = f64::from(max_side_px) / sw.max(sh) as f64;
    let w = ((sw as f64 * scale.min(1.0)).floor() as usize).max(1);
    let h = ((sh as f64 * scale.min(1.0)).floor() as usize).max(1);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let pixel = image.pixels[(y0 + (2 * y + 1) * sh / (2 * h)) * image.size[0]
                + x0
                + (2 * x + 1) * sw / (2 * w)];
            if pixel.a() != 255 {
                return Err("incomplete_image");
            }
            rgb.extend_from_slice(&pixel.to_array()[..3]);
        }
    }
    let png = png_rgb(w, h, &rgb);
    let camera = &capture.state.camera;
    let framing_occurrence_ids = match framing {
        ImageFraming::Viewport => Vec::new(),
        ImageFraming::Selection => capture.state.selection.clone(),
        ImageFraming::DetailSelection => detail
            .map(|target| vec![target.request.occurrence_id])
            .ok_or("invalid_image_framing")?,
    };
    let framing_detail = detail.map(|target| {
        json!({"kind":match target.request.kind { EntityKind::Edges => "edge", EntityKind::Faces => "face", _ => unreachable!() },
            "entity_id":target.request.entity_id,"reference_id":target.reference_id})
    });
    Ok(
        json!({"mime_type":"image/png","encoding":"base64","data":base64(&png),"width":w,"height":h,
        "scope":"cad_viewport","image_protocol_version":IMAGE_PROTOCOL_VERSION,"capture_mode":mode.as_str(),"stamp":capture.state.stamp,"capture_pass":capture.pass,
        "source_size_px":image.size,"crop_px":[x0,y0,sw,sh],"pixels_per_point":ppp,
        "sampling":"nearest_center","thumbnail":framing == ImageFraming::Viewport,"requested_max_side_px":max_side_px,
        "framing":{"mode":framing.as_str(),"occurrence_ids":framing_occurrence_ids,"detail":framing_detail},
        "view":{"projection":format!("{:?}",camera.projection_mode),"yaw":camera.yaw,"pitch":camera.pitch,
            "target_z_mm":camera.target_z,"zoom":camera.zoom,"pan":[camera.pan.x,camera.pan.y],"distance_mm":capture.state.distance},
        "selection":capture.state.selection,
        "render":{"render_correlated":true,"callback_correlated":!direct,
            "viewport_visibility_required":mode == CaptureMode::VisibleViewport,
            "viewport_unoccluded":mode == CaptureMode::VisibleViewport,"geometry_complete":false,
            "source":"isolated_cad_target","gui_overlays_included":false,
            "completeness":"display_only_not_geometry_validation","exact_contents_stamp":capture.state.exact,
            "topology_contents_stamp":capture.state.topology,"exact_evaluation_complete":capture.state.exact_complete,
            "exact_evaluation_pending":capture.state.evaluating,"scene_callbacks":capture.callbacks,
            "paint_shape_count":capture.shapes.len(),"style":format!("{:?}",camera),"theme":capture.state.theme}}),
    )
}
// Bounded RGB PNG using stored DEFLATE blocks: lossless and dependency-free.
fn png_rgb(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgb.len() + h);
    for row in rgb.chunks_exact(w * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let block_count = raw.len().div_ceil(u16::MAX as usize);
    for (index, block) in raw.chunks(u16::MAX as usize).enumerate() {
        z.push(u8::from(index + 1 == block_count));
        let n = block.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::from((w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    for (kind, data) in [
        (b"IHDR", ihdr.as_slice()),
        (b"IDAT", z.as_slice()),
        (b"IEND", &[][..]),
    ] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let mut crc = !0u32;
        for byte in &png[start..] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        png.extend_from_slice(&(!crc).to_be_bytes());
    }
    png
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
