//! The one path policy for files the window or this server touches on behalf of
//! a client: absolute, bounded and on a local drive. It reads nothing from the
//! file system, so a network path is refused before any request reaches its
//! server (and before Windows sends it the user's NTLM hash).

use std::path::{Component, Path, Prefix};

/// Longest accepted path, in bytes.
const PATH_BYTES: usize = 4096;

/// Windows device names, which name a device in any directory and with any
/// extension (`C:\work\nul.pdf`).
const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Whether `path` is an absolute path of a file on a local drive. On Windows
/// that is a drive letter and a root (`C:\…` or `C:/…`) and nothing else: no
/// share or device however its separators are mixed (`\\server`, `\/server`,
/// `\\?\`, `\\.\`), no alternate data stream (`file.pdf:stream`) and no device
/// name.
#[must_use]
pub fn local_absolute_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).is_absolute()
        && (!cfg!(windows) || local_windows_path(path))
}

fn local_windows_path(path: &str) -> bool {
    let mut components = Path::new(path).components();
    let on_disk = matches!(
        components.next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ) && components.next() == Some(Component::RootDir);
    let stream = path.char_indices().any(|(index, c)| c == ':' && index != 1);
    on_disk
        && !stream
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

#[cfg(all(test, windows))]
mod tests {
    use super::local_absolute_path;

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
