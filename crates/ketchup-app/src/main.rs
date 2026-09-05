#![forbid(unsafe_code)]

use ketchup_app::{
    KetchupApp, inspect_native_document, live_bridge::bootstrap::LiveStdinBootstrap,
};

fn bootstrap_failed() -> ! {
    eprintln!("live bridge bootstrap failed");
    std::process::exit(2);
}

fn main() -> eframe::Result {
    let all_arguments: Vec<_> = std::env::args_os().skip(1).collect();
    // Parsing the explicit flag is the only gateway to stdin. No environment,
    // automatic discovery, token argv, or token output fallback is supported.
    let live = LiveStdinBootstrap::from_arguments(all_arguments.clone())
        .unwrap_or_else(|_| bootstrap_failed());
    let mut arguments = all_arguments.into_iter();
    let first_argument = arguments.next();
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
            Some(live.read_stdin().unwrap_or_else(|_| bootstrap_failed())),
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
                bootstrap
                    .enable(&mut app, &creation_context.egui_ctx, std::io::stdout())
                    .unwrap_or_else(|_| bootstrap_failed());
            } else if let Some(path) = document_path.as_deref() {
                app.open_document_path(path);
            }
            Ok(Box::new(app))
        }),
    );
    if live_requested && result.is_err() {
        bootstrap_failed();
    }
    result
}
