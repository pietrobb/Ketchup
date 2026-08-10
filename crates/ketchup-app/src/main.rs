#![forbid(unsafe_code)]

use ketchup_app::{KetchupApp, inspect_native_document};

fn main() -> eframe::Result {
    let mut arguments = std::env::args_os();
    let _executable = arguments.next();
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--inspect-native-document")) {
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
    eframe::run_native(
        &KetchupApp::title(),
        KetchupApp::native_options(),
        Box::new(|creation_context| {
            Ok(Box::new(KetchupApp::from_creation_context(
                creation_context,
            )))
        }),
    )
}
