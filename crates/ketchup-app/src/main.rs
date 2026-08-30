#![forbid(unsafe_code)]

use ketchup_app::{KetchupApp, inspect_native_document};

fn main() -> eframe::Result {
    let mut arguments = std::env::args_os();
    let _executable = arguments.next();
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
    let document_path = first_argument.map(std::path::PathBuf::from);
    if arguments.next().is_some() {
        eprintln!("ketchup-app accepts at most one document path");
        std::process::exit(2);
    }
    eframe::run_native(
        &KetchupApp::title(),
        KetchupApp::native_options(),
        Box::new(move |creation_context| {
            let mut app = KetchupApp::from_creation_context(creation_context);
            if let Some(path) = document_path.as_deref() {
                app.open_document_path(path);
            }
            Ok(Box::new(app))
        }),
    )
}
