//! The one path policy for files the window or this server touches on behalf of
//! a client: absolute, bounded and on a local drive. It reads nothing from the
//! file system (on Windows only the drive table, to see a drive letter mapped to
//! a share; on macOS only the mount table, to see a share mounted under
//! `/Volumes`), so a network path is refused before any request reaches its
//! server (and before Windows sends it the user's NTLM hash).

use std::path::{Component, Path, Prefix};

/// Longest accepted path, in bytes.
const PATH_BYTES: usize = 4096;

/// Windows device names, which name a device in any directory and with any
/// extension (`C:\work\nul.pdf`).
const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Whether `path` is an absolute path of a file on a local drive. On Windows
/// that is a drive letter not mapped to a share and a root (`C:\…` or `C:/…`)
/// and nothing else: no share or device however its separators are mixed (`\\server`, `\/server`,
/// `\\?\`, `\\.\`), no alternate data stream (`file.pdf:stream`) and no device
/// name. On macOS it is a path on no network mount and in no directory the
/// automounter serves (`/net`, `/Network`, `/home`).
#[must_use]
pub fn local_absolute_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).is_absolute()
        && (!cfg!(windows) || local_windows_path(path, remote_drive))
        && (!cfg!(target_os = "macos") || mounts().is_ok_and(|m| local_macos_path(path, &m)))
}

/// Top-level directories macOS's automounter serves from the network.
const AUTOMOUNTED: [&str; 3] = ["net", "network", "home"];

/// The data volume's firmlinks make `/System/Volumes/Data/Volumes` the same
/// directory as `/Volumes`, and so for every top-level directory.
const DATA_VOLUME: [&str; 3] = ["system", "volumes", "data"];

/// A mount point as lowercase components (the default macOS file system ignores
/// case), and whether it is a local file system.
pub(crate) struct Mount {
    on: Vec<String>,
    local: bool,
}

impl Mount {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn new(on: &str, local: bool) -> Self {
        Self {
            on: lexical_components(on),
            local,
        }
    }
}

fn local_macos_path(path: &str, mounts: &[Mount]) -> bool {
    let components = lexical_components(path);
    let through_data_volume = components.strip_prefix(&DATA_VOLUME.map(str::to_owned)[..]);
    [Some(&components[..]), through_data_volume]
        .into_iter()
        .flatten()
        .all(|components| {
            !components
                .first()
                .is_some_and(|top| AUTOMOUNTED.contains(&top.as_str()))
                && mounts
                    .iter()
                    .filter(|mount| components.starts_with(&mount.on))
                    .max_by_key(|mount| mount.on.len())
                    .is_none_or(|mount| mount.local)
        })
}

/// The path's directories, lowercase, with `.` dropped and `..` applied.
fn lexical_components(path: &str) -> Vec<String> {
    let mut components = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => components.push(name.to_string_lossy().to_lowercase()),
            Component::ParentDir => {
                components.pop();
            }
            _ => {}
        }
    }
    components
}

#[cfg(target_os = "macos")]
fn mounts() -> std::io::Result<Vec<Mount>> {
    let table = crate::local_auth::mount_table()?;
    Ok(table
        .iter()
        .map(|(on, local)| Mount::new(on, *local))
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn mounts() -> std::io::Result<Vec<Mount>> {
    Ok(Vec::new())
}

fn local_windows_path(path: &str, remote_drive: impl Fn(u8) -> bool) -> bool {
    let mut components = Path::new(path).components();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) => Some(letter),
            _ => None,
        },
        _ => None,
    };
    let on_disk = drive.is_some() && components.next() == Some(Component::RootDir);
    let stream = path.char_indices().any(|(index, c)| c == ':' && index != 1);
    on_disk
        && !stream
        && !drive.is_some_and(&remote_drive)
        && components.all(|component| match component {
            Component::Normal(name) => !reserved_name(&name.to_string_lossy()),
            _ => true,
        })
}

fn reserved_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    RESERVED.contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|port| {
            stem.strip_prefix(port).is_some_and(|digit| {
                (digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit())
                    || matches!(digit, "\u{b9}" | "\u{b2}" | "\u{b3}")
            })
        })
}

#[cfg(windows)]
use crate::local_auth::remote_drive;

#[cfg(not(windows))]
fn remote_drive(_letter: u8) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::{local_absolute_path, local_windows_path};

    // Review 2026-10-19 P3: a drive letter mapped to a share passed as local.
    #[test]
    fn a_drive_mapped_to_a_network_share_is_not_local() {
        let z_is_a_share = |letter: u8| letter.eq_ignore_ascii_case(&b'Z');
        assert!(!local_windows_path(r"Z:\backup\x.pdf", z_is_a_share));
        assert!(!local_windows_path("z:/backup/x.pdf", z_is_a_share));
        assert!(local_windows_path(r"C:\work\x.pdf", z_is_a_share));
    }

    #[test]
    fn only_a_drive_path_is_local_however_the_separators_are_mixed() {
        for local in [r"C:\work\desk.pdf", "C:/work/desk.pdf", r"d:\a/b\c.ketchup"] {
            assert!(local_absolute_path(local), "{local}");
        }
        for remote in [
            r"\\server\share\x.pdf",
            "//server/share/x.pdf",
            r"\/server\share\x.pdf",
            r"/\server\share\x.pdf",
            r"\/./pipe/x.pdf",
            r"\\.\pipe\x",
            r"\\?\C:\work\x.pdf",
            r"\\?\UNC\server\share\x.pdf",
            r" //server/share/x.pdf",
            r"\work\x.pdf",
            r"C:work\x.pdf",
            r"C:\work\x.pdf:stream",
            r"C:\work\nul.pdf",
            r"C:\work\COM1",
            r"C:\con\x.pdf",
            r"C:\work\lpt9 .txt",
            "",
        ] {
            assert!(!local_absolute_path(remote), "{remote:?}");
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::{Mount, local_absolute_path, local_macos_path};

    // Review 2026-11-01 SEC-101: the automounter reaches the host named in the path.
    #[test]
    fn automounted_network_directories_are_not_local() {
        for remote in [
            "/net/server/share/x.pdf",
            "/NET/server/x.pdf",
            "//net/server/x.pdf",
            "/Network/Servers/server/x.pdf",
            "/home/user/x.pdf",
            "/Users/../net/server/x.pdf",
        ] {
            assert!(!local_absolute_path(remote), "{remote:?}");
        }
        assert!(local_absolute_path("/Users/me/desk.pdf"));
    }

    #[test]
    fn no_path_on_a_mount_of_this_computer_that_is_not_local_is_local() {
        let table = crate::local_auth::mount_table().expect("the mount table of this computer");
        assert!(table.iter().any(|(on, local)| on == "/" && *local));
        for (on, _) in table.iter().filter(|(_, local)| !local) {
            assert!(!local_absolute_path(&format!("{on}/x.pdf")), "{on}");
        }
    }

    // Review 2026-11-01 SEC-101: a share the Finder mounted under /Volumes.
    #[test]
    fn a_path_on_a_network_mount_is_not_local() {
        let mounts = [
            Mount::new("/", true),
            Mount::new("/Volumes/backup", false),
            Mount::new("/Volumes/USB", true),
        ];
        for local in [
            "/Users/me/desk.pdf",
            "/Volumes/USB/x.pdf",
            "/Volumes/backup2/x.pdf",
            "/Volumes/USB/../USB/x.pdf",
        ] {
            assert!(local_macos_path(local, &mounts), "{local:?}");
        }
        for remote in [
            "/Volumes/backup",
            "/Volumes/backup/x.pdf",
            "/volumes/BACKUP/x.pdf",
            "/Volumes/./backup/x.pdf",
            "/Volumes/USB/../backup/x.pdf",
            "/System/Volumes/Data/Volumes/backup/x.pdf",
        ] {
            assert!(!local_macos_path(remote, &mounts), "{remote:?}");
        }
    }
}
