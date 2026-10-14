//! The one wgpu instance of this crate's tests (unit tests and, through `#[path]`, the
//! integration binary); every test still gets its own device and render state.
//!
//! A debug-build instance enables the D3D12 debug layer and then releases its controller,
//! which unloads `D3D12SDKLayers.dll` while devices of other tests may still use it. Tests
//! running in parallel, each with its own instance, so corrupted the heap in the Kečup
//! Viewer tests (#103). An `egui_kittest` harness without an explicit renderer makes its
//! own instance when it renders, so harnesses that render take `renderer()`.

use std::sync::OnceLock;

pub(crate) fn instance() -> &'static wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| pollster::block_on(egui_kittest::wgpu::default_wgpu_setup().new_instance()))
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

/// The harness renderer: a fresh render state on the shared instance, made only when the
/// harness first renders.
pub(crate) fn renderer() -> egui_kittest::LazyRenderer {
    egui_kittest::LazyRenderer::new(|| {
        egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render_state())
    })
}
