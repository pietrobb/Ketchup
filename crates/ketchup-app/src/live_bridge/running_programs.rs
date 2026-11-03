//! The folders of running programs, by a word in their file name. Supervisor
//! keeps no list of its instances, and each instance reads its MCP servers in
//! its own folder, so the running ones are how Kečup finds them. The standard
//! library lists no processes, so this module asks the system
//! (`CreateToolhelp32Snapshot`, `QueryFullProcessImageNameW`), which needs
//! `unsafe`.
#![allow(unsafe_code)]

use std::path::PathBuf;

/// The folders of the running programs whose file name contains `word` (in
/// any case), each once, in the order the system lists them.
#[cfg(windows)]
pub fn folders_of_running(word: &str) -> Vec<PathBuf> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    let word = word.to_lowercase();
    let mut folders = Vec::new();
    let Ok(entry_size) = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()) else {
        return folders;
    };
    // SAFETY: takes no pointers; the handle is checked and closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return folders;
    }
    // SAFETY: PROCESSENTRY32W is plain data, valid zeroed.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = entry_size;
    // SAFETY: the snapshot is open and `entry` is owned here with its size set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
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
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot is open and closed once.
    unsafe { CloseHandle(snapshot) };
    folders
}

/// The folder of the program process `id` runs, if the system tells it.
#[cfg(windows)]
fn program_folder(id: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    // SAFETY: takes no pointers; a null handle (no access) is checked.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, id) };
    if process.is_null() {
        return None;
    }
    let mut length: u32 = 32_768;
    let mut path = vec![0u16; usize::try_from(length).ok()?];
    // SAFETY: `path` holds `length` units and outlives the call, which writes at
    // most that many and sets `length` to the units written.
    let found = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
    } != 0;
    // SAFETY: the process handle is open and closed once.
    unsafe { CloseHandle(process) };
    if !found {
        return None;
    }
    let written = path.get(..usize::try_from(length).ok()?)?;
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
