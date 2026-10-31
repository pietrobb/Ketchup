//! Desktop start shared by the Windows binary and the macOS app:
//! `ketchup-viewer [file]` opens a window, `--verify-open <file>` only reads
//! the file and prints what it holds (installers use it to prove the
//! installed Viewer reads packages), `--benchmark <file>` prints how fast it
//! opens, draws and picks the package. Files the system hands over while the
//! Viewer runs (macOS Finder, "Open with") arrive through [`OpenRequests`].

use crate::app::ViewerApp;
use crate::preferences::{Preferences, desktop_settings_path};
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
    if first.as_deref() == Some(std::ffi::OsStr::new("--benchmark")) {
        return benchmark(args.next().map(PathBuf::from));
    }
    if let Some(path) = first {
        requests.push(PathBuf::from(path));
    }
    let settings = desktop_settings_path();
    let preferences = settings
        .as_deref()
        .map(Preferences::load)
        .unwrap_or_default();
    let result = eframe::run_native(
        "Kečup Viewer",
        native_options(),
        Box::new(move |cc| {
            requests.attach(&cc.egui_ctx);
            let app = ViewerApp::new(cc.wgpu_render_state.clone()).with_preferences(preferences);
            Ok(Box::new(DesktopViewer {
                app,
                requests,
                settings,
            }))
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

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_icon(window_icon()),
        ..Default::default()
    }
}

/// The Viewer icon on the title bar and taskbar: the editor's ketchup artwork
/// on a green background with a large eye (from `scripts/make_app_icon.py`).
#[must_use]
pub fn window_icon() -> egui::IconData {
    const SIDE: u32 = 128;
    egui::IconData {
        rgba: include_bytes!("../assets/ketchup-viewer-128.rgba").to_vec(),
        width: SIDE,
        height: SIDE,
    }
}

struct DesktopViewer {
    app: ViewerApp,
    requests: OpenRequests,
    /// `None` without a per-user settings folder: settings last until the Viewer closes.
    settings: Option<PathBuf>,
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
        if let (Some(preferences), Some(path)) =
            (self.app.take_changed_preferences(), &self.settings)
            && let Err(error) = preferences.save(path)
        {
            eprintln!("{error}");
            self.app.report(error);
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

/// Frames orbited by `--benchmark`, at a common laptop window size.
const BENCHMARK_FRAMES: usize = 300;
const BENCHMARK_SIZE: [u32; 2] = [1600, 900];

/// `--benchmark <file>`: open, draw and pick on the GPU the window would
/// use, and print the timings (`crate::app::benchmark`).
fn benchmark(path: Option<PathBuf>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("usage: ketchup-viewer --benchmark <file.ketchup-view>");
        return ExitCode::FAILURE;
    };
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    })) else {
        eprintln!("viewer.benchmark.gpu: no GPU adapter");
        return ExitCode::FAILURE;
    };
    let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()));
    let Ok((device, queue)) = device else {
        eprintln!("viewer.benchmark.gpu: the GPU device could not be created");
        return ExitCode::FAILURE;
    };
    let info = adapter.get_info();
    println!(
        "Ketchup Viewer {} on {} ({:?})",
        env!("CARGO_PKG_VERSION"),
        info.name,
        info.backend
    );
    match crate::app::benchmark::run(&device, &queue, &path, BENCHMARK_SIZE, BENCHMARK_FRAMES) {
        Ok(result) => {
            print!("{}", result.report());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_carries_the_viewer_icon() {
        let icon = native_options()
            .viewport
            .icon
            .expect("the Viewer window must carry its icon");
        assert_eq!(*icon, window_icon());
    }

    #[test]
    fn viewer_icon_is_transparent_in_the_corners_and_differs_from_the_editor() {
        let icon = window_icon();
        let side = icon.width as usize;
        assert_eq!((icon.width, icon.height), (128, 128));
        assert_eq!(icon.rgba.len(), side * side * 4);
        let pixel = |x: usize, y: usize| &icon.rgba[(y * side + x) * 4..(y * side + x) * 4 + 4];
        for (x, y) in [(0, 0), (side - 1, 0), (0, side - 1), (side - 1, side - 1)] {
            assert_eq!(pixel(x, y)[3], 0, "corner ({x}, {y}) must be transparent");
        }
        assert_eq!(pixel(side / 2, side / 2)[3], 255);
        // The large eye over the lower half: dark pupil in its middle, white beside it.
        let eye_y = side * 72 / 100;
        assert_eq!(pixel(side / 2, eye_y), [10, 20, 40, 255]);
        assert_eq!(pixel(side * 23 / 100, eye_y), [255, 255, 255, 255]);
        // Green background instead of the editor's blue.
        let &[r, g, b, _] = pixel(side * 8 / 100, side * 31 / 100) else {
            unreachable!("an RGBA pixel has four channels")
        };
        assert!(g > r && g > b, "background must be green, got {r} {g} {b}");
        let editor = include_bytes!("../../ketchup-app/assets/ketchup-128.rgba");
        assert_ne!(icon.rgba.as_slice(), editor.as_slice());
    }
}
