//! macOS entry point of the Kečup Viewer, the executable inside
//! `Kečup Viewer.app`. Finder does not pass a double-clicked file on the
//! command line but as an Apple Event, also to a Viewer that already runs.

fn main() -> std::process::ExitCode {
    let requests = ketchup_viewer::desktop::OpenRequests::default();
    #[cfg(target_os = "macos")]
    let _handler = {
        let requests = requests.clone();
        ketchup_macos_open::install(move |path| requests.push(path))
    };
    ketchup_viewer::desktop::main(requests)
}
