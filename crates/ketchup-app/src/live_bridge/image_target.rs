//! Independent renderer, resources, attachment and readback. No GUI textures.
use super::{CaptureNonce, MAX_SOURCE_PIXELS, Painted};
use eframe::egui;
use eframe::egui_wgpu::{Callback, CallbackResources, CallbackTrait, Renderer, ScreenDescriptor};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Pixels {
    pub nonce: CaptureNonce,
    pub pass: u64,
    pub image: egui::ColorImage,
}
#[derive(Default)]
struct State {
    device: Option<wgpu::Device>,
    result: Option<Result<Pixels, &'static str>>,
}
struct Shared {
    state: Mutex<State>,
    revoked: AtomicBool,
    started: AtomicBool,
}
pub(super) struct Readback(Arc<Shared>);
impl Drop for Readback {
    fn drop(&mut self) {
        self.0.revoked.store(true, Ordering::Release);
        if let Ok(mut state) = self.0.state.lock() {
            state.result = None;
        }
    }
}
impl Readback {
    pub fn take(&self) -> Result<Option<Pixels>, &'static str> {
        let device = self
            .0
            .state
            .lock()
            .map_err(|_| "image_unavailable")?
            .device
            .clone();
        // Never wait on the UI thread. map_async completion needs device polling.
        if let Some(device) = device {
            device
                .poll(wgpu::PollType::Poll)
                .map_err(|_| "image_unavailable")?;
        }
        self.0
            .state
            .lock()
            .map_err(|_| "image_unavailable")?
            .result
            .take()
            .transpose()
    }
}
pub(super) fn dimensions(screen: egui::Rect, ppp: f32) -> Result<[u32; 2], &'static str> {
    if !ppp.is_finite()
        || ppp <= 0.0
        || screen.min != egui::Pos2::ZERO
        || !screen.is_finite()
        || screen.width() <= 0.0
        || screen.height() <= 0.0
    {
        return Err("invalid_image_dimensions");
    }
    let size = [
        (screen.width() * ppp).round(),
        (screen.height() * ppp).round(),
    ];
    if size
        .iter()
        .any(|v| !v.is_finite() || *v < 1.0 || *v > MAX_SOURCE_PIXELS as f32)
        || f64::from(size[0]) * f64::from(size[1]) > MAX_SOURCE_PIXELS as f64
    {
        return Err("invalid_image_dimensions");
    }
    Ok(size.map(|v| v as u32))
}
fn capture(
    painted: &Painted,
    nonce: CaptureNonce,
    cancelled: Arc<AtomicBool>,
) -> Result<(Readback, IsolatedCapture), &'static str> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        revoked: AtomicBool::new(false),
        started: AtomicBool::new(false),
    });
    let callback = IsolatedCapture {
        shared: shared.clone(),
        cancelled,
        nonce,
        pass: painted.pass,
        size: dimensions(painted.screen, painted.ppp)?,
        ppp: painted.ppp,
        jobs: painted.jobs.clone(),
        atlas: painted.atlas.clone(),
        scene: painted.callbacks != 0,
    };
    Ok((Readback(shared), callback))
}

pub(super) fn schedule(
    ctx: &egui::Context,
    painted: &Painted,
    nonce: CaptureNonce,
    cancelled: Arc<AtomicBool>,
) -> Result<Readback, &'static str> {
    let (readback, callback) = capture(painted, nonce, cancelled)?;
    ctx.layer_painter(egui::LayerId::background())
        .add(Callback::new_paint_callback(painted.rect, callback));
    Ok(readback)
}

pub(super) fn submit(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    painted: &Painted,
    nonce: CaptureNonce,
    cancelled: Arc<AtomicBool>,
) -> Result<Readback, &'static str> {
    let (readback, callback) = capture(painted, nonce, cancelled)?;
    callback.shared.started.store(true, Ordering::Release);
    callback.render(device, queue)?;
    Ok(readback)
}
struct IsolatedCapture {
    shared: Arc<Shared>,
    cancelled: Arc<AtomicBool>,
    nonce: CaptureNonce,
    pass: u64,
    size: [u32; 2],
    ppp: f32,
    jobs: Vec<egui::epaint::ClippedPrimitive>,
    atlas: egui::ColorImage,
    scene: bool,
}
impl IsolatedCapture {
    fn stopped(&self) -> bool {
        self.shared.revoked.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire)
    }
    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), &'static str> {
        if self
            .size
            .iter()
            .any(|v| *v > device.limits().max_texture_dimension_2d)
            || self
                .atlas
                .size
                .iter()
                .any(|v| *v > device.limits().max_texture_dimension_2d as usize)
        {
            return Err("unsupported_image_renderer");
        }
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut renderer = Renderer::new(device, format, None, 1, false);
        if self.scene {
            renderer
                .callback_resources
                .insert(crate::GpuInstancedRenderer::new(device, format));
        }
        renderer.update_texture(
            device,
            queue,
            egui::TextureId::default(),
            &egui::epaint::ImageDelta::full(self.atlas.clone(), egui::TextureOptions::LINEAR),
        );
        let descriptor = ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: self.ppp,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Live bridge isolated CAD attachment"),
            size: wgpu::Extent3d {
                width: self.size[0],
                height: self.size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row_bytes = self.size[0] * 4;
        let stride = row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live bridge isolated CAD readback"),
            size: u64::from(stride) * u64::from(self.size[1]),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Live bridge isolated CAD encoder"),
        });
        let mut commands =
            renderer.update_buffers(device, queue, &mut encoder, &self.jobs, &descriptor);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Live bridge isolated CAD pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            renderer.render(&mut pass, &self.jobs, &descriptor);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        if self.stopped() {
            return Ok(());
        }
        commands.push(encoder.finish());
        queue.submit(commands);
        self.shared
            .state
            .lock()
            .map_err(|_| "image_unavailable")?
            .device = Some(device.clone());
        let shared = self.shared.clone();
        let cancelled = self.cancelled.clone();
        let nonce = self.nonce.clone();
        let pass = self.pass;
        let size = self.size;
        let mapped = buffer.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if shared.revoked.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) {
                    if result.is_ok() {
                        mapped.unmap();
                    }
                    return;
                }
                let result = result.map_err(|_| "image_unavailable").map(|()| {
                    let bytes = mapped.slice(..).get_mapped_range();
                    let mut rgba = Vec::with_capacity(size[0] as usize * size[1] as usize * 4);
                    for row in bytes.chunks_exact(stride as usize) {
                        rgba.extend_from_slice(&row[..row_bytes as usize]);
                    }
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied(size.map(|v| v as usize), &rgba);
                    drop(bytes);
                    mapped.unmap();
                    Pixels { nonce, pass, image }
                });
                if let Ok(mut state) = shared.state.lock()
                    && !shared.revoked.load(Ordering::Acquire)
                    && !cancelled.load(Ordering::Acquire)
                {
                    state.result = Some(result);
                }
            });
        Ok(())
    }
}
impl CallbackTrait for IsolatedCapture {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &ScreenDescriptor,
        _: &mut wgpu::CommandEncoder,
        _: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if self.stopped() || self.shared.started.swap(true, Ordering::AcqRel) {
            return Vec::new();
        }
        if let Err(error) = self.render(device, queue)
            && let Ok(mut state) = self.shared.state.lock()
        {
            state.result = Some(Err(error));
        }
        Vec::new()
    }
    // The enclosing GUI pass cannot contribute pixels to the private attachment.
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &CallbackResources,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_dimensions_are_bounded_and_finite() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        assert_eq!(dimensions(rect, 2.0), Ok([1600, 1200]));
        for ppp in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e20] {
            assert!(dimensions(rect, ppp).is_err());
        }
        assert!(dimensions(rect.translate(egui::vec2(1.0, 0.0)), 1.0).is_err());
    }
    #[test]
    fn dropping_readback_revokes_retained_callback_authority() {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            revoked: AtomicBool::new(false),
            started: AtomicBool::new(false),
        });
        let readback = Readback(shared.clone());
        drop(readback);
        assert!(shared.revoked.load(Ordering::Acquire));
        assert!(shared.state.lock().unwrap().result.is_none());
    }
}
