#![forbid(unsafe_code)]

use ketchup_app::{
    KetchupApp, inspect_native_document, live_bridge::bootstrap::LiveStdinBootstrap,
    verify_public_assistant_runtime,
};
use ketchup_assistant::sidecar::AssistantDistribution;
use ketchup_model::persistence::{self, LoadOutcome};

/// Reports why the launcher-requested live bridge could not start; never prints input.
fn bootstrap_failed(reason: impl std::fmt::Display) -> ! {
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
        if let Err(error) = ketchup_mcp::serve_stdio(std::env::current_exe().ok()) {
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
    if first_argument.as_deref() == Some(std::ffi::OsStr::new("--verify-public-assistant-runtime"))
    {
        if arguments.next().is_some() {
            eprintln!("--verify-public-assistant-runtime accepts no arguments");
            std::process::exit(2);
        }
        if let Err(error) = verify_public_assistant_runtime() {
            eprintln!("public Assistant runtime verification failed: {error}");
            std::process::exit(2);
        }
        println!("public Assistant runtime verified");
        return Ok(());
    }
    if first_argument.as_deref() == Some(std::ffi::OsStr::new("--verify-manual-alpha")) {
        let persistence_path = arguments.next();
        if arguments.next().is_some() {
            eprintln!("--verify-manual-alpha accepts at most one persistence path");
            std::process::exit(2);
        }
        if !KetchupApp::is_manual_alpha_build() || !cfg!(feature = "private-oauth") {
            eprintln!("this is not an OAuth-enabled Manual Alpha build");
            std::process::exit(2);
        }
        let app = KetchupApp::new();
        let handshake = app.assistant_handshake();
        if handshake.distribution != AssistantDistribution::PrivateOauth
            || handshake.provider != "codex-oauth"
            || handshake.model != "gpt-5.6-sol"
            || handshake.validate().is_err()
        {
            eprintln!("Manual Alpha Assistant defaults are invalid");
            std::process::exit(2);
        }
        if let Some(path) = persistence_path {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() || path.exists() {
                eprintln!("Manual Alpha persistence path must be absolute and absent");
                std::process::exit(2);
            }
            let before = app.document_snapshot();
            if persistence::save_atomic(&path, &before).is_err() {
                eprintln!("Manual Alpha persistence save failed");
                std::process::exit(2);
            }
            let reopened = match persistence::load_file(&path) {
                Ok(LoadOutcome::Editable { document, .. }) => document.current(),
                _ => {
                    eprintln!("Manual Alpha persistence reopen failed");
                    std::process::exit(2);
                }
            };
            if reopened.document_id() != before.document_id()
                || reopened.revision_id() != before.revision_id()
                || reopened.canonical_digest() != before.canonical_digest()
            {
                eprintln!("Manual Alpha persistence identity changed after reopen");
                std::process::exit(2);
            }
        }
        println!(
            "Ketchup Manual Alpha {} verified; private-oauth codex-oauth gpt-5.6-sol",
            KetchupApp::build_version()
        );
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
    let (bootstrap, document_path) = if let Some(live) = live {
        // Fail before creating a native window when input is absent or invalid.
        (
            Some(
                live.read_stdin()
                    .unwrap_or_else(|error| bootstrap_failed(error)),
            ),
            None,
        )
    } else {
        let document_path = first_argument.map(std::path::PathBuf::from);
        if arguments.next().is_some() {
            eprintln!("ketchup-app accepts at most one document path");
            std::process::exit(2);
        }
        (None, document_path)
    };
    let live_requested = bootstrap.is_some();
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
                if let Some(path) = document_path.as_deref() {
                    app.open_document_path(path);
                }
            }
            Ok(Box::new(app))
        }),
    );
    if live_requested && let Err(error) = &result {
        bootstrap_failed(format_args!(
            "live bridge bootstrap failed: window failed: {error}"
        ));
    }
    result
}
