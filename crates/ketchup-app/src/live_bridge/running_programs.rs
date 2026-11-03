//! The folders of running programs, by a word in their file name. Supervisor
//! keeps no list of its instances, and each instance reads its MCP servers in
//! its own folder, so the running ones are how Kečup finds them. The standard
//! library lists no processes, so this module asks the system
//! (`CreateToolhelp32Snapshot`, `QueryFullProcessImageNameW`), which needs
//! `unsafe`.
#![allow(unsafe_code)]

use std::path::PathBuf;

/// An open system handle, closed once when dropped, also on an early return.
#[cfg(windows)]
struct Handle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by the system, is valid and is closed
        // only here, once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// The folders of the running programs whose file name contains `word` (in
/// any case), each once, in the order the system lists them.
#[cfg(windows)]
pub fn folders_of_running(word: &str) -> Vec<PathBuf> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    /// `dwSize` of a `PROCESSENTRY32W` on 64-bit Windows, the only Windows
    /// Kečup builds for; the build fails where the system's size differs.
    const ENTRY_SIZE: u32 = 568;
    const _: () = assert!(std::mem::size_of::<PROCESSENTRY32W>() == 568);

    let word = word.to_lowercase();
    let mut folders = Vec::new();
    // SAFETY: takes no pointers; an invalid handle is checked below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return folders;
    }
    let snapshot = Handle(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: ENTRY_SIZE,
        ..PROCESSENTRY32W::default()
    };
    // SAFETY: the snapshot is open and `entry` is owned here with its size set.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        let name = &entry.szExeFile;
        let length = name
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(name.len());
        if String::from_utf16_lossy(&name[..length])
            .to_lowercase()
            .contains(&word)
            && let Some(folder) = program_folder(entry.th32ProcessID)
            && !folders.contains(&folder)
        {
            folders.push(folder);
        }
        // SAFETY: as for Process32FirstW.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    folders
}

/// The folder of the program process `id` runs, if the system tells it.
#[cfg(windows)]
fn program_folder(id: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    // SAFETY: takes no pointers; a null handle (no access) is checked.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id) };
    if process.is_null() {
        return None;
    }
    let process = Handle(process);
    let mut length: u32 = 32_768;
    let mut path = vec![0u16; 32_768];
    // SAFETY: `path` holds `length` units and outlives the call, which writes at
    // most that many and sets `length` to the units written.
    let found = unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            path.as_mut_ptr(),
            &mut length,
        )
    } != 0;
    drop(process);
    if !found {
        return None;
    }
    // The call ends the path with a zero, and `path` was all zeros before.
    let written = path.split(|&unit| unit == 0).next().unwrap_or_default();
    let program = PathBuf::from(std::ffi::OsString::from_wide(written));
    program.parent().map(PathBuf::from)
}

/// Supervisor runs on Windows only.
#[cfg(not(windows))]
pub fn folders_of_running(_word: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn this_test_program_is_found_running_in_its_folder() {
        let this = std::env::current_exe().unwrap();
        let name = this.file_stem().unwrap().to_string_lossy().to_uppercase();
        let folders = super::folders_of_running(&name);
        assert!(
            folders
                .iter()
                .any(|folder| folder == this.parent().unwrap()),
            "{folders:?}"
        );
    }
}
