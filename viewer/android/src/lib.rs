//! Android entry point of the Kečup Viewer: opens the package placed in the
//! app's external data directory.

/// Called by android-activity when the app starts.
#[cfg(target_os = "android")]
#[allow(unsafe_code)] // android-activity resolves this unique Rust-ABI entry point.
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {
    let path = android_app
        .external_data_path()
        .map(|path| path.join("model.ketchup-view"));
    let options = eframe::NativeOptions {
        android_app: Some(android_app),
        renderer: eframe::Renderer::Wgpu,
        run_and_return: false,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Kečup Viewer",
        options,
        Box::new(move |cc| {
            let mut app = ketchup_viewer::app::ViewerApp::new(cc.wgpu_render_state.clone());
            if let Some(path) = path
                && path.exists()
                && let Err(error) = app.open_path(&path)
            {
                eprintln!("{error}; {}", error.fix_hint_text());
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(error) = result {
        eprintln!("Kečup Viewer Android startup failed: {error}");
    }
}
