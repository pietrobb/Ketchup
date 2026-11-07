//! Whether a file the user named is there, the one answer Open Recent, the
//! installer of a client's entry and the list of written drawings take
//! (review 2026-11-06 r23 SEC-201, SEC-202): a file whose drive, share or
//! mounted volume does not answer is neither there nor gone. Asking may wait
//! for a server that does not answer, so it is asked on a worker, never on
//! the window's thread.

use std::path::{Path, PathBuf};

/// Whether a file is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Here,
    /// Its folder answers and holds no such file: gone for good.
    Gone,
    /// Its folder does not answer, as on a server, a disk or a volume that is
    /// offline or not mounted: it may be back later.
    Unknown,
}

/// The folder below which a missing folder was deleted, not a volume that is
/// not mounted now, even in a folder volumes are mounted in: the home folder,
/// on Windows none, where a drive or share that is out answers nothing at all.
pub(crate) fn local_folder() -> Option<PathBuf> {
    // Without a home folder no folder above a file's own is trusted: an
    // absolute folder is never below a relative one.
    (!cfg!(windows)).then(|| {
        std::env::home_dir()
            .filter(|home| home.is_absolute())
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

/// Whether the file at `path` is there, a missing folder above it trusted to
/// be deleted below `local` only (`None`: anywhere; see [`local_folder`]).
pub(crate) fn presence(path: &Path, local: Option<&Path>) -> Presence {
    presence_below(path, local.as_ref().map(std::slice::from_ref))
}

/// Whether the file at `path` is there, a missing folder above it trusted to
/// be deleted below one of `trusted` only (`None`: anywhere).
pub(crate) fn presence_below(path: &Path, trusted: Option<&[&Path]>) -> Presence {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Presence::Here,
        Ok(_) => Presence::Gone,
        Err(error) => {
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) && !is_dangling_link(path)
                && folder_answers(path, trusted)
            {
                Presence::Gone
            } else {
                Presence::Unknown
            }
        }
    }
}

/// Whether the folders of a missing `path` answer: its own folder is there,
/// or the nearest one above it that is there is below one of `trusted`
/// (review 2026-11-04 r19 P3: a document whose folder was deleted stayed out
/// of reach for ever). No list of the folders volumes are mounted in is
/// trusted to be whole: gvfs, kio-fuse and autofs mount shares where they
/// will, so a folder missing outside `trusted` is unknown (review 2026-11-07
/// capability finding on its P3 of `mcp_setup.rs`). A drive, share or volume
/// that is out has no folder there at all (Windows says so of `E:\` and
/// `\\server\share` alike, macOS of `/Volumes/NAS`), and an empty folder does
/// not answer: it may be where a share is mounted, as sshfs under the home
/// folder, while it is not (review 2026-11-06 P3).
fn folder_answers(path: &Path, trusted: Option<&[&Path]>) -> bool {
    for folder in path.ancestors().skip(1) {
        match std::fs::metadata(folder) {
            Ok(metadata) => {
                return metadata.is_dir()
                    && std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some())
                    && (Some(folder) == path.parent()
                        || trusted.is_none_or(|trusted| {
                            trusted.iter().any(|root| folder.starts_with(root))
                        }));
            }
            // Only a folder that is not there lets the search go up.
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound || is_dangling_link(folder) {
                    return false;
                }
            }
        }
    }
    false
}

/// Whether `path` is a link (a symbolic link or a junction) whose target is not
/// there, as `~/NAS` to a volume not mounted now: the way to a place that may be
/// back, not a folder deleted (review 2026-11-07 UI-220).
fn is_dangling_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The root of a drive letter no drive has, as of a USB disk or a share
/// taken out: the system says its path is not found.
#[cfg(all(test, windows))]
pub(crate) fn drive_not_in_use() -> PathBuf {
    (b'D'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .find(|root| {
            std::fs::metadata(root).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        })
        .expect("a drive letter not in use")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_in_a_folder_deleted_locally_is_gone() {
        let directory = tempfile::tempdir().unwrap();
        let document = directory
            .path()
            .join("Projects")
            .join("Phase 2")
            .join("a.ketchup");
        std::fs::write(directory.path().join("notes.txt"), "kept").unwrap();
        assert_eq!(presence(&document, Some(directory.path())), Presence::Gone);
        assert_eq!(presence(&document, None), Presence::Gone);
        // Outside the home folder a missing folder may be a share not mounted
        // now; below another folder trusted, as `/opt` for a program
        // deleted (review 2026-11-07 P3), it is gone.
        let home = directory.path().join("home");
        assert_eq!(presence(&document, Some(&home)), Presence::Unknown);
        assert_eq!(
            presence_below(&document, Some(&[home.as_path(), directory.path()])),
            Presence::Gone
        );
        // Its own folder there still says it is gone anywhere.
        let beside = directory.path().join("a.ketchup");
        assert_eq!(presence(&beside, Some(&home)), Presence::Gone);
        std::fs::write(&beside, "here").unwrap();
        assert_eq!(presence(&beside, Some(&home)), Presence::Here);
    }

    /// Review 2026-11-06 P3: an sshfs share under the home folder that is not
    /// mounted now is an empty folder, which answered.
    #[test]
    fn a_file_on_a_share_not_mounted_is_unknown() {
        let home = tempfile::tempdir().unwrap();
        let mount = home.path().join("nas");
        std::fs::create_dir(&mount).unwrap();
        let deep = mount.join("Domy").join("a.ketchup");
        assert_eq!(presence(&deep, Some(home.path())), Presence::Unknown);
        assert_eq!(
            presence(&mount.join("a.ketchup"), Some(home.path())),
            Presence::Unknown
        );
        std::fs::write(mount.join("b.ketchup"), "mounted").unwrap();
        assert_eq!(presence(&deep, Some(home.path())), Presence::Gone);
        assert_eq!(
            presence(&mount.join("a.ketchup"), Some(home.path())),
            Presence::Gone
        );
    }

    /// Review 2026-11-07 capability finding on the P3 of `mcp_setup.rs`: a
    /// share outside the home folder and every folder volumes are known to be
    /// mounted in, as gvfs below `/run/user/1000`, not mounted now, its folder
    /// gone and the one above it answering, was gone, so Open Recent dropped
    /// it and the rows of drawings.txt were deleted.
    #[test]
    fn a_file_on_a_share_not_mounted_outside_the_home_folder_is_unknown() {
        let run_user = tempfile::tempdir().unwrap();
        std::fs::write(run_user.path().join("bus"), "").unwrap();
        let home = tempfile::tempdir().unwrap();
        let document = run_user
            .path()
            .join("gvfs")
            // As `smb-share:server=nas,share=p`, without the colon Windows
            // refuses in a name.
            .join("smb-share-server=nas,share=p")
            .join("Domy")
            .join("a.ketchup");
        assert_eq!(presence(&document, Some(home.path())), Presence::Unknown);
    }

    /// A link to `target`: a symbolic link, on Windows a junction, which needs
    /// no privilege.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        assert!(
            std::process::Command::new("cmd")
                .arg("/c")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    /// Review 2026-11-07 UI-220: `~/NAS` linked to a volume not mounted now
    /// made a file below it gone, so Open Recent dropped it.
    #[test]
    fn a_file_below_a_dangling_link_is_unknown() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("notes.txt"), "kept").unwrap();
        let volume = home.path().join("volume");
        std::fs::create_dir(&volume).unwrap();
        let nas = home.path().join("NAS");
        link_folder(&volume, &nas);
        std::fs::remove_dir(&volume).unwrap();
        for document in [nas.join("a.ketchup"), nas.join("Domy").join("a.ketchup")] {
            assert_eq!(presence(&document, Some(home.path())), Presence::Unknown);
            assert_eq!(presence(&document, None), Presence::Unknown);
        }
        // A folder deleted beside the link is still gone.
        let deleted = home.path().join("Old").join("a.ketchup");
        assert_eq!(presence(&deleted, Some(home.path())), Presence::Gone);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_to_a_file_is_unknown() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("notes.txt"), "kept").unwrap();
        let document = home.path().join("a.ketchup");
        std::os::unix::fs::symlink(home.path().join("NAS").join("a.ketchup"), &document).unwrap();
        assert_eq!(presence(&document, Some(home.path())), Presence::Unknown);
    }

    /// Review 2026-11-06 r23 SEC-201: a program on a drive that is out, or a
    /// share that does not answer, was gone, so the installer replaced its
    /// client's entry.
    #[cfg(windows)]
    #[test]
    fn a_file_on_a_drive_or_share_that_is_out_is_unknown() {
        let free = drive_not_in_use();
        let program = free.join("tools").join("ketchup.exe");
        assert_eq!(presence(&program, None), Presence::Unknown);
        assert_eq!(presence(&free.join("ketchup.exe"), None), Presence::Unknown);
    }
}
