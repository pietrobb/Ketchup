//! Gives `ketchup-viewer.exe` its icon (`assets/ketchup-viewer.ico`).
#[path = "windows_icon.rs"]
mod windows_icon;

fn main() -> std::io::Result<()> {
    windows_icon::embed("ketchup-viewer.ico")
}
