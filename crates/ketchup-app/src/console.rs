//! The released editor is a window program on Windows (`windows_subsystem =
//! "windows"`), so a terminal that starts it gives it no console and what it prints
//! is lost. A command-line answer such as `--version` attaches to the console of the
//! terminal that started it, so the answer shows there (feedback 2026-11-10).

/// Lets what the program prints reach the console of the terminal that started it.
/// Output already going somewhere (a pipe, a file, an own console) is left alone,
/// and without a parent console nothing happens.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn attach_parent_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: `GetStdHandle` only reads this process's standard output handle.
    let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if !output.is_null() && output != INVALID_HANDLE_VALUE {
        return;
    }
    // SAFETY: `AttachConsole` takes a process selector and touches no memory of
    // ours; when there is no parent console it fails and changes nothing.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Elsewhere a terminal always hands the program its output.
#[cfg(not(windows))]
pub(crate) fn attach_parent_console() {}
