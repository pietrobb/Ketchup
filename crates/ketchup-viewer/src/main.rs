//! Desktop entry point: `ketchup-viewer [file.ketchup-view]`.

use ketchup_viewer::app::ViewerApp;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    eframe::run_native(
        "Kečup Viewer",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = ViewerApp::new(cc.wgpu_render_state.clone());
            if let Some(path) = path
                && let Err(error) = app.open_path(&path)
            {
                eprintln!("{error}; {}", error.fix_hint_text());
            }
            Ok(Box::new(app))
        }),
    )
}
