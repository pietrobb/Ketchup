//! Desktop entry point: `ketchup-viewer [file.ketchup-view | file.ketchup-comments]`.
//! `--verify-open <file>` opens the file without a window and prints what it holds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    ketchup_viewer::desktop::main(ketchup_viewer::desktop::OpenRequests::default())
}
