#![forbid(unsafe_code)]

use ketchup_app::KetchupApp;

fn main() -> eframe::Result {
    eframe::run_native(
        &KetchupApp::title(),
        KetchupApp::native_options(),
        Box::new(|_creation_context| Ok(Box::new(KetchupApp::new()))),
    )
}
