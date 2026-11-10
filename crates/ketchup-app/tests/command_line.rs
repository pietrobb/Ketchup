//! What Kečup answers on its command line without opening a window.
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Runs Kečup with `arguments`, killed after 30 s: a window opened instead of
/// an answer never ends by itself.
fn run(arguments: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ketchup-app"))
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("Kečup {arguments:?} did not end within 30 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

/// Feedback 2026-11-10: `Ketchup.exe --version` took the flag for a document
/// to open, started the window and never ended.
#[test]
fn version_prints_the_release_and_ends() {
    let expected = format!("Ketchup {}\n", ketchup_app::KetchupApp::build_version());
    for flag in ["--version", "-V"] {
        let output = run(&[flag]);
        assert!(output.status.success(), "{flag}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected, "{flag}");
    }
    let refused = run(&["--version", "stolik.ketchup"]);
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
}

/// Feedback 2026-11-10: the executable had no FileVersion or ProductVersion,
/// so Explorer and scripts could not tell which Kečup it is.
#[cfg(all(windows, target_env = "msvc"))]
#[test]
fn the_executable_carries_its_version() {
    let read = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
             $info = (Get-Item $env:KETCHUP_EXE).VersionInfo; \
             $info.ProductVersion; $info.FileVersion; $info.ProductName",
        ])
        .env("KETCHUP_EXE", env!("CARGO_BIN_EXE_ketchup-app"))
        .output()
        .unwrap();
    assert!(read.status.success(), "{read:?}");
    let version = ketchup_app::KetchupApp::build_version();
    let numeric: String = version
        .split(['-', '+'])
        .next()
        .unwrap()
        .split('.')
        .chain(["0"])
        .take(4)
        .collect::<Vec<_>>()
        .join(".");
    let stdout = String::from_utf8_lossy(&read.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, [version, numeric.as_str(), "Kečup"], "{read:?}");
}
