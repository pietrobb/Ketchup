//! Desktop entry point: `ketchup-viewer [file.ketchup-view | file.ketchup-comments]`.
//! `--verify-open <file>` opens the file without a window and prints what it
//! holds; installers use it to prove the installed Viewer reads packages.

use ketchup_viewer::app::ViewerApp;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--verify-open")) {
        return verify_open(args.next().map(PathBuf::from));
    }
    let path = first.map(PathBuf::from);
    let result = eframe::run_native(
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
                eprintln!("{error}");
            }
            Ok(Box::new(app))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn verify_open(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("usage: ketchup-viewer --verify-open <file.ketchup-view|file.ketchup-comments>");
        return ExitCode::FAILURE;
    };
    let mut app = ViewerApp::new(None);
    if let Err(error) = app.open_path(&path) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let Some([components, scenes, comments]) = app.opened_counts() else {
        return ExitCode::FAILURE;
    };
    println!(
        "Ketchup Viewer {} opened {components} components, {scenes} scenes, {comments} comments",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::SUCCESS
}
