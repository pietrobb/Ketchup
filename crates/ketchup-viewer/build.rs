//! Gives `ketchup-viewer.exe` its icon (`assets/ketchup-viewer.ico`). Built
//! with `-C target-feature=+crt-static` for MSVC, as the release Viewer is
//! (`scripts/windows/build-installer.ps1`), it carries the Visual C++ runtime
//! and needs no Redistributable on a clean Windows; the universal C runtime
//! stays the DLL that Windows 10 and later have (the hybrid CRT).
#[path = "windows_icon.rs"]
mod windows_icon;

fn main() -> std::io::Result<()> {
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    let static_crt = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .is_ok_and(|features| features.split(',').any(|feature| feature == "crt-static"));
    if msvc && static_crt {
        println!("cargo:rustc-link-arg=/NODEFAULTLIB:libucrt.lib");
        println!("cargo:rustc-link-arg=/DEFAULTLIB:ucrt.lib");
    }
    windows_icon::embed("ketchup-viewer.ico")
}
