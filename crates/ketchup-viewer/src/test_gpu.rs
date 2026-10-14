//! The one wgpu instance of this crate's tests; every test still gets its own device.
//!
//! A debug-build instance enables the D3D12 debug layer and then releases its controller,
//! which unloads `D3D12SDKLayers.dll` while devices of other tests may still use it. Tests
//! running in parallel, each with its own instance, so corrupted the heap (#103:
//! 0xc0000374 freeing a debug-layer root signature as a render state was dropped).

use std::sync::OnceLock;

fn instance() -> &'static wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| pollster::block_on(egui_kittest::wgpu::default_wgpu_setup().new_instance()))
}

/// A device on the adapter wgpu prefers (the real GPU), for the renderer's own pixels.
pub(crate) fn gpu_device() -> (wgpu::Device, wgpu::Queue) {
    let adapter =
        pollster::block_on(instance().request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter required, never skip rendering proof");
    eprintln!("Viewer GPU probe: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("GPU device")
}

/// A fresh egui render state (own device and renderer) on the adapter `egui_kittest`
/// prefers, for whole-window pictures.
pub(crate) fn render_state() -> eframe::egui_wgpu::RenderState {
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
