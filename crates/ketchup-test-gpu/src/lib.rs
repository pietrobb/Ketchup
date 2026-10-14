//! The one wgpu instance of a Ketchup test process; every test still gets its own device
//! and render state. The Kečup app and Viewer tests both take it from here.
//!
//! A debug-build instance enables the D3D12 debug layer and then releases its controller,
//! which unloads `D3D12SDKLayers.dll` while devices of other tests may still use it. Tests
//! running in parallel, each with its own instance, so corrupted the heap (#103:
//! 0xc0000374 freeing a debug-layer root signature as a render state was dropped). An
//! `egui_kittest` harness without an explicit renderer makes its own instance when it
//! renders, so harnesses that render take `renderer()`.

use std::sync::OnceLock;

pub fn instance() -> &'static wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| pollster::block_on(egui_kittest::wgpu::default_wgpu_setup().new_instance()))
}

/// A device on the adapter wgpu prefers (the real GPU), for a renderer's own pixels.
pub fn gpu_device() -> (wgpu::Device, wgpu::Queue) {
    let adapter =
        pollster::block_on(instance().request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter required, never skip rendering proof");
    eprintln!("GPU probe: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("GPU device")
}

/// A fresh egui render state (own device and renderer) on the adapter `egui_kittest`
/// prefers, for whole-window pictures.
pub fn render_state() -> eframe::egui_wgpu::RenderState {
    let config = eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_kittest::wgpu::default_wgpu_setup(),
        ..Default::default()
    };
    pollster::block_on(eframe::egui_wgpu::RenderState::create(
        &config,
        instance(),
        None,
        None,
        1,
        false,
    ))
    .expect("render state")
}

/// The harness renderer: a fresh render state on the shared instance, made only when the
/// harness first renders.
pub fn renderer() -> egui_kittest::LazyRenderer {
    egui_kittest::LazyRenderer::new(|| {
        egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render_state())
    })
}
