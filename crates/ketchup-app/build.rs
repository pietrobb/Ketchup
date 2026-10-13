//! Gives the Windows executable its icon (`assets/ketchup.ico`).
#[path = "../ketchup-viewer/windows_icon.rs"]
mod windows_icon;

fn main() -> std::io::Result<()> {
    windows_icon::embed("ketchup.ico")
}
