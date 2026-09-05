//! CAD-only readback. The GUI framebuffer is never an image source.
use super::*;
use crate::{CameraViewState, ProjectedEdge, ProjectedFace, RenderBox};
use egui::{ColorImage, Rect, Shape};
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "image_privacy_tests.rs"]
mod privacy_tests;
#[path = "image_target.rs"]
mod target;

const THUMBNAIL_SIDE: usize = 64;
const MAX_SOURCE_PIXELS: usize = 16_777_216;

#[derive(Default)]
pub(super) struct ImageState {
    pending: Option<ImageRequest>,
    painted: Option<Result<Painted, &'static str>>,
}
impl ImageState {
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
    nonce: CaptureNonce,
    capture: Option<(Painted, target::Readback)>,
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
        if !self
            .live_bridge
            .as_ref()
            .is_some_and(|b| b.image.pending.is_some())
        {
            return;
        }
        let painted = (|| {
            let screen = ui.ctx().screen_rect();
            let ppp = ui.ctx().pixels_per_point();
            if !ui.is_visible()
                || !ui.clip_rect().contains_rect(rect)
                || !screen.contains_rect(rect)
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
            Ok(Painted {
                state: VisualState::read(self)?,
                rect,
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
            let Request::Image { expected } = &queued.request else {
                unreachable!()
            };
            Self::guard(app, expected)?;
            Self::available(app, ctx.wants_keyboard_input() || ctx.is_using_pointer())?;
            if self.image.pending.is_some() {
                return Err("busy");
            }
            let mut nonce = [0; 32];
            getrandom::fill(&mut nonce).map_err(|_| "image_unavailable")?;
            Ok((VisualState::read(app)?, CaptureNonce(nonce)))
        })();
        match result {
            Ok((initial, nonce)) => {
                self.image.pending = Some(ImageRequest {
                    queued,
                    deadline: Instant::now() + Duration::from_millis(1500),
                    initial,
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
            if ctx.viewport_id() != egui::ViewportId::ROOT
                || ctx.input(|i| i.viewport().minimized == Some(true) || !i.focused)
            {
                return Err("hidden_viewport");
            }
            let painted = self.image.painted.take().ok_or("hidden_viewport")??;
            if VisualState::read(app)? != painted.state {
                return Err("stale_image");
            }
            if let Some((capture, readback)) = &request.capture {
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
                    return thumbnail(capture, &pixels.image).map(Some);
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
                let readback = target::schedule(
                    ctx,
                    &painted,
                    request.nonce.clone(),
                    request.queued.cancelled.clone(),
                )?;
                request.capture = Some((painted, readback));
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
                response.stamp = Some(
                    request
                        .capture
                        .as_ref()
                        .map_or_else(|| app.live_bridge_stamp(), |(p, _)| p.state.stamp.clone()),
                );
                if serde_json::to_vec(&response).map_or(true, |v| v.len() > MAX_FRAME_BYTES) {
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
fn thumbnail(capture: &Painted, image: &ColorImage) -> Result<Value, &'static str> {
    let ppp = capture.ppp;
    let expected = target::dimensions(capture.screen, ppp)?.map(|v| v as usize);
    let count = image.size[0]
        .checked_mul(image.size[1])
        .ok_or("response_limit")?;
    if image.size != expected || count > MAX_SOURCE_PIXELS || image.pixels.len() != count {
        return Err("invalid_image_dimensions");
    }
    // This is an isolated CAD texture, never a crop of the GUI framebuffer.
    let rect = capture.rect.shrink(2.0);
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
    let scale = THUMBNAIL_SIDE as f64 / sw.max(sh) as f64;
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
    Ok(
        json!({"mime_type":"image/png","encoding":"base64","data":base64(&png),"width":w,"height":h,
        "scope":"cad_viewport","stamp":capture.state.stamp,"capture_pass":capture.pass,
        "source_size_px":image.size,"crop_px":[x0,y0,sw,sh],"pixels_per_point":ppp,
        "sampling":"nearest_center","thumbnail":true,
        "view":{"projection":format!("{:?}",camera.projection_mode),"yaw":camera.yaw,"pitch":camera.pitch,
            "target_z_mm":camera.target_z,"zoom":camera.zoom,"pan":[camera.pan.x,camera.pan.y],"distance_mm":capture.state.distance},
        "selection":capture.state.selection,
        "render":{"callback_correlated":true,"viewport_unoccluded":true,"geometry_complete":false,
            "source":"isolated_cad_target","gui_overlays_included":false,
            "completeness":"display_only_not_geometry_validation","exact_contents_stamp":capture.state.exact,
            "topology_contents_stamp":capture.state.topology,"exact_evaluation_complete":capture.state.exact_complete,
            "exact_evaluation_pending":capture.state.evaluating,"scene_callbacks":capture.callbacks,
            "paint_shape_count":capture.shapes.len(),"style":format!("{:?}",camera),"theme":capture.state.theme}}),
    )
}
// Fixed-size RGB PNG using one stored DEFLATE block: bounded, lossless, no new dependency.
fn png_rgb(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgb.len() + h);
    for row in rgb.chunks_exact(w * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01, 0x01];
    let n = raw.len() as u16;
    z.extend_from_slice(&n.to_le_bytes());
    z.extend_from_slice(&(!n).to_le_bytes());
    z.extend_from_slice(&raw);
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
