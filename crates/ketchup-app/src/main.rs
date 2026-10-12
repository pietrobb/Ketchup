#![forbid(unsafe_code)]
// No console window behind the editor; MCP and launchers still talk over inherited pipes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ketchup_app::{
    KetchupApp, inspect_native_document, live_bridge::bootstrap::LiveStdinBootstrap,
};
use ketchup_model::persistence::{self, LoadOutcome};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

/// Documents the operating system asks a running Ketchup to open: on macOS
/// Finder and "Open with" send them by Apple Event, not on the command line.
#[derive(Clone, Default)]
struct OpenRequests(Arc<Mutex<(Vec<PathBuf>, Option<eframe::egui::Context>)>>);

impl OpenRequests {
    fn push(&self, path: PathBuf) {
        let mut inbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inbox.0.push(path);
        if let Some(context) = &inbox.1 {
            context.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
            context.request_repaint();
        }
    }

    /// The document to open now. Ketchup holds one document, so of several
    /// files handed over at once only the first is opened.
    fn take(&self, context: &eframe::egui::Context) -> Option<PathBuf> {
        let mut inbox = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inbox.1.get_or_insert_with(|| context.clone());
        let mut paths = std::mem::take(&mut inbox.0).into_iter();
        paths.next()
    }
}

struct Desktop {
    app: KetchupApp,
    requests: OpenRequests,
}

impl eframe::App for Desktop {
    fn update(&mut self, context: &eframe::egui::Context, frame: &mut eframe::Frame) {
        // Wait until a running Open or Save finishes; the open itself asks
        // before discarding unsaved changes, like the Open command.
        if self.app.file_activity_idle()
            && let Some(path) = self.requests.take(context)
        {
            self.app.queue_open_document_path(&path);
        }
        self.app.update(context, frame);
    }
}

/// Reports why the launcher-requested live bridge could not start; never prints input.
/// The launcher reads the reason from the readiness line on stdout.
fn bootstrap_failed(reason: impl std::fmt::Display) -> ! {
    let reason = reason.to_string();
    println!("{}", serde_json::json!({"version": 1, "error": reason}));
    eprintln!("{reason}");
    std::process::exit(2);
}

fn main() -> eframe::Result {
    let all_arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if all_arguments.first().map(|argument| argument == "--mcp") == Some(true) {
        if all_arguments.len() > 1 {
            eprintln!("--mcp accepts no arguments");
            std::process::exit(2);
        }
        // Stdout carries only MCP messages from here on.
        if let Err(error) = ketchup_mcp::serve_stdio(ketchup_mcp::stage::window_source()) {
            eprintln!("MCP server stopped: {error}");
            std::process::exit(2);
        }
        return Ok(());
    }
    // Parsing the explicit flag is the only gateway to stdin. No environment,
    // automatic discovery, token argv, or token output fallback is supported.
    let live = LiveStdinBootstrap::from_arguments(all_arguments.clone())
        .unwrap_or_else(|error| bootstrap_failed(error));
    let mut arguments = all_arguments.into_iter();
    let first_argument = arguments.next();
    if first_argument.as_deref() == Some(std::ffi::OsStr::new("--verify-install")) {
        let persistence_path = arguments.next();
        if arguments.next().is_some() {
            eprintln!("--verify-install accepts at most one persistence path");
            std::process::exit(2);
        }
        let app = KetchupApp::new();
        if let Some(path) = persistence_path {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() || path.exists() {
                eprintln!("the persistence path must be absolute and absent");
                std::process::exit(2);
            }
            let before = app.document_snapshot();
            if persistence::save_atomic(&path, &before).is_err() {
                eprintln!("persistence save failed");
                std::process::exit(2);
            }
            let reopened = match persistence::load_file(&path) {
                Ok(LoadOutcome::Editable { document, .. }) => document.current(),
                _ => {
                    eprintln!("persistence reopen failed");
                    std::process::exit(2);
                }
            };
            if reopened.document_id() != before.document_id()
                || reopened.revision_id() != before.revision_id()
                || reopened.canonical_digest() != before.canonical_digest()
            {
                eprintln!("persistence identity changed after reopen");
                std::process::exit(2);
            }
        }
        println!("Ketchup {} verified", KetchupApp::build_version());
        return Ok(());
    }
    if first_argument.as_deref() == Some(std::ffi::OsStr::new("--inspect-native-document")) {
        let Some(path) = arguments.next() else {
            eprintln!("--inspect-native-document requires one path");
            std::process::exit(2);
        };
        if arguments.next().is_some() {
            eprintln!("--inspect-native-document accepts exactly one path");
            std::process::exit(2);
        }
        match inspect_native_document(std::path::Path::new(&path)) {
            Ok(inspection) => println!("{}", inspection.to_json()),
            Err(error) => {
                eprintln!("native document inspection failed: {error}");
                std::process::exit(2);
            }
        }
        return Ok(());
    }
    let (bootstrap, start_arguments) = if let Some(live) = live {
        // Fail before creating a native window when input is absent or invalid.
        (
            Some(
                live.read_stdin()
                    .unwrap_or_else(|error| bootstrap_failed(error)),
            ),
            Vec::new(),
        )
    } else {
        // Checked in the window, which says what it cannot open.
        (None, first_argument.into_iter().chain(arguments).collect())
    };
    let live_requested = bootstrap.is_some();
    let requests = OpenRequests::default();
    #[cfg(target_os = "macos")]
    let _open_events = {
        let requests = requests.clone();
        ketchup_macos_open::install(move |path| requests.push(path))
    };
    let result = eframe::run_native(
        &KetchupApp::title(),
        KetchupApp::native_options(),
        Box::new(move |creation_context| {
            let mut app = KetchupApp::from_creation_context(creation_context);
            if let Some(bootstrap) = bootstrap {
                app.enable_live_consent_broker(&creation_context.egui_ctx)
                    .unwrap_or_else(|error| {
                        bootstrap_failed(format_args!(
                            "live bridge bootstrap failed: consent broker unavailable: {error}"
                        ))
                    });
                bootstrap
                    .enable(&mut app, &creation_context.egui_ctx, std::io::stdout())
                    .unwrap_or_else(|error| bootstrap_failed(error));
            } else {
                if let Err(error) = app.enable_live_consent_broker(&creation_context.egui_ctx) {
                    eprintln!(
                        "optional live consent broker unavailable; manual CAD remains available: {error}"
                    );
                }
                app.queue_startup_arguments(start_arguments);
            }
            Ok(Box::new(Desktop { app, requests }))
        }),
    );
    if live_requested && let Err(error) = &result {
        bootstrap_failed(format_args!(
            "live bridge bootstrap failed: window failed: {error}"
        ));
    }
    result
}
