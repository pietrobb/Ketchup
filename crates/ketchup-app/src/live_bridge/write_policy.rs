//! Where a connected AI writes files (owner decision R1 of review 2026-11-19
//! og). Kečup runs for one user on one computer, writes only its own kinds of
//! file, none of which runs by itself, and replaces no file without asking. So a
//! client writes a new document or export into any folder the system lets the
//! user write into, unasked; replacing a file is asked about in the window as
//! each export and Save As does. Refused, with the reason, are only the
//! system's own places nobody keeps their files in: the folders that start
//! programs at login, the Windows and Program Files folders, and the root of
//! the system drive. What the client wrote is listed in the Connect tab
//! ([`super::write_log`]). The folders a user had to allow before (review
//! 2026-11-08 §5.4) are gone with their question: they protected nothing a
//! client could not reach by writing beside the document (R1119og-SEC-800).

use super::*;

/// The path `request` writes a file at as the client named it: a document
/// saved or a file exported, none for a request that writes no file.
pub(super) fn written_path(request: &Request) -> Option<&str> {
    match request {
        Request::SaveAs { path, .. }
        | Request::Export(crate::live_bridge::interchange::ExportRequest { path, .. })
        | Request::DrawingExport { path, .. }
        | Request::ExportProgramVariants { path, .. } => Some(path),
        Request::ExportDrawings(export) => Some(&export.path),
        Request::ApplyAndVerify {
            save: Some(ApplyAndVerifySave::Path { path }),
            ..
        } => Some(path),
        _ => None,
    }
}

/// Whether `request` saves the document rather than exporting a file.
fn saves_document(request: &Request) -> bool {
    matches!(
        request,
        Request::SaveAs { .. }
            | Request::ApplyAndVerify {
                save: Some(ApplyAndVerifySave::Path { .. }),
                ..
            }
    )
}

/// `queued` as the window runs it, or none when it is refused for the file it
/// writes and answered so. What it writes is listed in the Connect tab once
/// it is answered, however it is run.
pub(super) fn admitted(
    app: &KetchupApp,
    mut queued: Queued,
    context: &egui::Context,
) -> Option<Queued> {
    if let Some(refused) = refusal(&queued.request) {
        LiveBridge::reply(app, queued.id, &queued.reply, Err(refused));
        return None;
    }
    // A save to the document's own file names no path (review 2026-11-20 ac P3).
    let saved_in_place = || match queued.request {
        Request::Save { .. } => app.file.path.clone(),
        _ => None,
    };
    if let Some(path) = written_path(&queued.request)
        .map(PathBuf::from)
        .or_else(saved_in_place)
    {
        queued.reply = app
            .live
            .write_log
            .watch(path, queued.reply, context.clone());
    }
    Some(queued)
}

/// The code refusing the file `request` writes, its reason recorded, or none:
/// a document saved as anything but a `.ketchup` file, or as a hidden one, is
/// `invalid_path` (review 2026-11-08 SEC-240), an export whose folder does not
/// resolve is `invalid_path`, and a file in a system place is `system_folder`.
/// A path that is not local and absolute is left to the request, which
/// refuses it as it did.
pub(super) fn refusal(request: &Request) -> Option<&'static str> {
    let path = written_path(request)?;
    if !drawings::local_absolute_path(path) {
        return None;
    }
    take_error_details();
    let path = Path::new(path);
    let document = saves_document(request);
    if document && let Some(refused) = document_name_refusal(path) {
        return Some(refused);
    }
    let folder = match folder_of(path) {
        Ok(Some(folder)) => folder,
        // A document's folder that does not resolve is the save's to refuse.
        _ if document => return None,
        Ok(None) => {
            return Some(failure(
                "invalid_path",
                "The path's folder does not resolve: name a folder that exists, without `..` past it.",
                json!({"path": path}),
            ));
        }
        Err(error) => {
            return Some(failure(
                "invalid_path",
                "A folder of the path is there but does not resolve, such as a link whose target is gone: name the folder it stands for.",
                json!({"path": path, "error_kind": error.kind().to_string()}),
            ));
        }
    };
    let place = system_place(&folder, &system_places())?;
    Some(failure(
        "system_folder",
        format!(
            "{} Write the file into a folder of the user's, such as the document's or Documents.",
            place.reason()
        ),
        json!({"path": path, "folder": folder, "place": place.name()}),
    ))
}

/// The code refusing `path` as the name of a document saved, or none: a
/// `.ketchup` file, never a hidden one.
fn document_name_refusal(path: &Path) -> Option<&'static str> {
    if path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
    {
        return Some(failure(
            "invalid_path",
            "A document is saved as no hidden file: the name begins with a dot. Name the document like plan.ketchup.",
            json!({"path": path}),
        ));
    }
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ketchup"))
    {
        return Some(failure(
            "invalid_path",
            "A document is saved as a Ketchup document, whose name ends in .ketchup; export other formats with file action=export.",
            json!({"path": path}),
        ));
    }
    None
}

/// A system place nobody keeps their files in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Place {
    /// A folder whose programs start when the user logs in.
    Startup,
    /// The operating system's or the installed programs' folder: Windows and
    /// Program Files (R1 names no other), so none off Windows.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    System,
    /// The root of the system drive itself, not the folders on it.
    DriveRoot,
}

impl Place {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::System => "system",
            Self::DriveRoot => "drive_root",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Startup => {
                "The folder starts the programs in it when the user logs in, so Kečup writes no file there."
            }
            Self::System => {
                "The folder is the operating system's or the installed programs', where no user keeps a file, so Kečup writes none there."
            }
            Self::DriveRoot => {
                "The root of the system drive is no place for a user's file, so Kečup writes none there."
            }
        }
    }
}

/// The system places of this computer, as named by its environment; a place
/// the environment does not name is left out.
pub(super) fn system_places() -> Vec<(PathBuf, Place)> {
    let variable = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut places = Vec::new();
    #[cfg(windows)]
    {
        let startup = Path::new(r"Microsoft\Windows\Start Menu\Programs\Startup");
        for (base, below, place) in [
            ("APPDATA", Some(startup), Place::Startup),
            ("ProgramData", Some(startup), Place::Startup),
            ("SystemRoot", None, Place::System),
            ("windir", None, Place::System),
            ("ProgramFiles", None, Place::System),
            ("ProgramFiles(x86)", None, Place::System),
            ("ProgramW6432", None, Place::System),
        ] {
            if let Some(base) = variable(base) {
                places.push((
                    below.map_or_else(|| base.clone(), |below| base.join(below)),
                    place,
                ));
            }
        }
        // A Startup folder moved by "User Shell Folders" is not where the
        // environment says (review 2026-11-20 ac P3); the shell says where.
        places.extend(
            known_startup_folders()
                .into_iter()
                .map(|folder| (folder, Place::Startup)),
        );
        // `C:` and `\` make the root `C:\`, not the current folder of the drive.
        let mut root = variable("SystemDrive")
            .unwrap_or_else(|| PathBuf::from("C:"))
            .into_os_string();
        root.push(r"\");
        places.push((PathBuf::from(root), Place::DriveRoot));
    }
    #[cfg(unix)]
    {
        let home = variable("HOME");
        let config =
            variable("XDG_CONFIG_HOME").or_else(|| home.as_ref().map(|home| home.join(".config")));
        places.extend(config.map(|config| (config.join("autostart"), Place::Startup)));
        places.push((PathBuf::from("/etc/xdg/autostart"), Place::Startup));
        if cfg!(target_os = "macos") {
            places.extend(home.map(|home| (home.join("Library/LaunchAgents"), Place::Startup)));
            places.push((PathBuf::from("/Library/LaunchAgents"), Place::Startup));
            places.push((PathBuf::from("/Library/LaunchDaemons"), Place::Startup));
        }
        places.push((PathBuf::from("/"), Place::DriveRoot));
    }
    places
}

/// The user's and all users' Startup folders where the shell keeps them now,
/// a folder moved included; a folder the shell does not name is left out.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn known_startup_folders() -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_CommonStartup, FOLDERID_Startup, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
    };
    [FOLDERID_Startup, FOLDERID_CommonStartup]
        .iter()
        .filter_map(|id| {
            let mut path: windows_sys::core::PWSTR = std::ptr::null_mut();
            let flags = KF_FLAG_DONT_VERIFY.cast_unsigned();
            // SAFETY: `id` is a known folder's id and `path` receives a string
            // the shell allocates; on success it is NUL-terminated, read before
            // it is freed, and freed whatever the result (null is allowed).
            unsafe {
                let result = SHGetKnownFolderPath(id, flags, std::ptr::null_mut(), &mut path);
                let found = (result >= 0 && !path.is_null()).then(|| {
                    let length = (0..).take_while(|&index| *path.add(index) != 0).count();
                    let wide = std::slice::from_raw_parts(path, length);
                    PathBuf::from(std::ffi::OsString::from_wide(wide))
                });
                CoTaskMemFree(path.cast());
                found
            }
        })
        .collect()
}

/// The system place among `places` the canonical `folder` is: the drive's
/// root only as itself, any other place with the folders under it.
pub(super) fn system_place(folder: &Path, places: &[(PathBuf, Place)]) -> Option<Place> {
    places.iter().find_map(|(place, kind)| {
        let place = resolved(place).ok().flatten()?;
        let inside = if *kind == Place::DriveRoot {
            same_path(folder, &place)
        } else {
            folder.ancestors().any(|folder| same_path(folder, &place))
        };
        inside.then_some(*kind)
    })
}

/// Whether two canonical paths name one folder; on Windows whatever the case,
/// as the disk takes them.
fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    } else {
        left == right
    }
}

/// The folder `path` is written into as the disk names it; see [`resolved`].
pub(super) fn folder_of(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match path.parent() {
        Some(folder) => resolved(folder),
        None => Ok(None),
    }
}

/// `folder` as the disk names it: its nearest folder that exists made
/// canonical, the folders still to be made under it added. None when a part
/// past it is `..` or the root does not resolve; the error of a folder that is
/// there but does not resolve, such as a link whose target is gone or cannot
/// be read, which would be written through.
fn resolved(folder: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut missing = Vec::new();
    let mut existing = folder;
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut folder) => {
                folder.extend(missing.iter().rev());
                return Ok(Some(folder));
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(existing).is_err() =>
            {
                let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
                    return Ok(None);
                };
                missing.push(name);
                existing = parent;
            }
            Err(error) => return Err(error),
        }
    }
}
