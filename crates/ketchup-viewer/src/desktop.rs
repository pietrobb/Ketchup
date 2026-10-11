//! Desktop start shared by the Windows binary and the macOS app:
//! `ketchup-viewer [file]` opens a window, `--verify-open <file>` only reads
//! the file and prints what it holds (installers use it to prove the
//! installed Viewer reads packages). Files the system hands over while the
//! Viewer runs (macOS Finder, "Open with") arrive through [`OpenRequests`].

use crate::app::ViewerApp;
use eframe::egui;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};

/// Files the operating system asked the Viewer to open, filled by the
/// platform layer and opened by the window on its next frame.
#[derive(Clone, Default)]
pub struct OpenRequests(Arc<Mutex<Inbox>>);

#[derive(Default)]
struct Inbox {
    paths: Vec<PathBuf>,
    /// The window to wake once it exists; files arriving earlier wait.
    context: Option<egui::Context>,
}

impl OpenRequests {
    /// Queue `path` and wake the window.
    pub fn push(&self, path: PathBuf) {
        let mut inbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inbox.paths.push(path);
        if let Some(context) = &inbox.context {
            context.send_viewport_cmd(egui::ViewportCommand::Focus);
            context.request_repaint();
        }
    }

    fn attach(&self, context: &egui::Context) {
        let mut inbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inbox.context = Some(context.clone());
    }

    fn take(&self) -> Vec<PathBuf> {
        let mut inbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut inbox.paths)
    }
}

/// Run the desktop Viewer with the process arguments.
pub fn main(requests: OpenRequests) -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--verify-open")) {
        return verify_open(args.next().map(PathBuf::from));
    }
    if let Some(path) = first {
        requests.push(PathBuf::from(path));
    }
    let result = eframe::run_native(
        "Kečup Viewer",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(move |cc| {
            requests.attach(&cc.egui_ctx);
            let app = ViewerApp::new(cc.wgpu_render_state.clone());
            Ok(Box::new(DesktopViewer { app, requests }))
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

struct DesktopViewer {
    app: ViewerApp,
    requests: OpenRequests,
}

impl eframe::App for DesktopViewer {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Unsaved comments only warn, like the Open button; handing the file
        // over again (a second double-click) discards them.
        let paths = self.requests.take();
        if !paths.is_empty() && self.app.may_discard_comments() {
            for path in paths {
                if let Err(error) = self.app.open_path(&path) {
                    eprintln!("{error}");
                    self.app.report(error);
                }
            }
        }
        self.app.update(ctx, frame);
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
