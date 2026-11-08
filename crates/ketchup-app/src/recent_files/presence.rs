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
    presence_within(path, trusted, LINK_HOPS)
}

/// How many links one after another a file's path is followed through, as
/// many as Linux follows.
const LINK_HOPS: usize = 40;

fn presence_within(path: &Path, trusted: Option<&[&Path]>, hops: usize) -> Presence {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Presence::Here,
        Ok(_) => Presence::Gone,
        Err(error) => {
            if !matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) {
                return Presence::Unknown;
            }
            // A link to a file is gone where the file it names is (review
            // 2026-11-07 r26 P3: one deleted in the home folder was unknown
            // for ever), followed so far only.
            if let Ok(target) = std::fs::read_link(path) {
                let target = path.parent().unwrap_or(Path::new("")).join(target);
                return match hops.checked_sub(1) {
                    Some(hops) => presence_within(&target, trusted, hops),
                    None => Presence::Unknown,
                };
            }
            if !is_dangling_link(path) && folder_answers(path, trusted) {
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
/// folder, while it is not (review 2026-11-06 P3). The folder and `trusted`
/// are compared as the system resolves them, so `~/NAS` linked to gvfs is not
/// below the home folder (review 2026-11-07 r26 P3), nor is a folder mounted
/// on demand or through FUSE, as `/opt` from autofs ([`on_remote_mount`]).
fn folder_answers(path: &Path, trusted: Option<&[&Path]>) -> bool {
    // A folder the system does not resolve is compared as it is written.
    let real = |folder: &Path| {
        std::fs::canonicalize(folder)
            .ok()
            .unwrap_or_else(|| folder.into())
    };
    for folder in path.ancestors().skip(1) {
        match std::fs::metadata(folder) {
            Ok(metadata) => {
                return metadata.is_dir()
                    && std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some())
                    && (Some(folder) == path.parent()
                        || trusted.is_none_or(|trusted| {
                            let folder = real(folder);
                            trusted.iter().any(|root| folder.starts_with(real(root)))
                                && !mounted_remotely(&folder)
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

/// Whether `folder`, resolved, is on a file system the system mounts on
/// demand or through FUSE, which says a folder is not there while its server
/// does not answer (review 2026-11-07 r26 P3: `/opt` or `/usr` so on a cluster
/// was gone, and `--mcp-setup` replaced the client's entry). Only Linux says.
#[cfg(target_os = "linux")]
fn mounted_remotely(folder: &Path) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|mountinfo| on_remote_mount(&mountinfo, folder))
}

#[cfg(not(target_os = "linux"))]
const fn mounted_remotely(_: &Path) -> bool {
    false
}

/// Whether the mount of `folder` in `mountinfo` (as `/proc/self/mountinfo`),
/// the last of those mounted deepest above it, is a mount on demand not made
/// (autofs) or a FUSE daemon's, which says a share it lost is not there. NFS,
/// CIFS and the like are not: a server that says a folder is not there has
/// answered, and one that is down hangs or fails otherwise (review 2026-11-07
/// r26 capability finding: a folder deleted in a home folder on NFS was never
/// gone, as review 2026-11-04 r19 P3 had it).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn on_remote_mount(mountinfo: &str, folder: &Path) -> bool {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (fields, rest) = line.split_once(" - ")?;
            let point = mount_point(fields.split(' ').nth(4)?);
            Some((point, rest.split(' ').next()?))
        })
        .filter(|(point, _)| folder.starts_with(point))
        .max_by_key(|(point, _)| point.components().count())
        .is_some_and(|(_, kind)| kind == "autofs" || kind.starts_with("fuse."))
}

/// A mount point as `mountinfo` writes it, a space, tab, line feed or
/// backslash as three octal digits after a backslash.
fn mount_point(field: &str) -> PathBuf {
    let bytes = field.as_bytes();
    let mut point = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let octal = bytes.get(at + 1..at + 4).filter(|digits| {
            bytes[at] == b'\\'
                && digits[0] <= b'3'
                && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        });
        match octal {
            Some(digits) => {
                point.push(
                    digits
                        .iter()
                        .fold(0u8, |byte, digit| byte * 8 + (digit - b'0')),
                );
                at += 4;
            }
            None => {
                point.push(bytes[at]);
                at += 1;
            }
        }
    }
    PathBuf::from(String::from_utf8_lossy(&point).into_owned())
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

    /// A symbolic link to a file at `link`, false where the system lets none
    /// be made (Windows without the privilege).
    fn link_file(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, link);
        made.is_ok()
    }

    /// Review 2026-11-07 r26 P3: a link to a file deleted in the home folder
    /// was unknown for ever. Its target is asked: gone when deleted there,
    /// unknown on a volume not mounted now.
    #[test]
    fn a_dangling_link_to_a_file_is_as_its_target() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("notes.txt"), "kept").unwrap();
        let media = tempfile::tempdir().unwrap();
        std::fs::write(media.path().join("other"), "").unwrap();
        let document = home.path().join("a.ketchup");
        if !link_file(&media.path().join("NAS").join("a.ketchup"), &document) {
            return;
        }
        assert_eq!(presence(&document, Some(home.path())), Presence::Unknown);
        let deleted = home.path().join("b.ketchup");
        assert!(link_file(
            &Path::new("Projects").join("b.ketchup"),
            &deleted
        ));
        assert_eq!(presence(&deleted, Some(home.path())), Presence::Gone);
        let to_link = home.path().join("c.ketchup");
        assert!(link_file(&deleted, &to_link));
        assert_eq!(presence(&to_link, Some(home.path())), Presence::Gone);
    }

    /// Review 2026-11-07 r26 P3: `~/NAS` linked to a share below gvfs that is
    /// not mounted now was below the home folder by its text, so a file on it
    /// was gone. And a trusted folder by a link to it, as `/tmp` on macOS, did
    /// not hold what is in it.
    #[test]
    fn folders_are_compared_as_the_system_resolves_them() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("notes.txt"), "kept").unwrap();
        let gvfs = tempfile::tempdir().unwrap();
        std::fs::write(gvfs.path().join("other-share"), "").unwrap();
        let nas = home.path().join("NAS");
        link_folder(gvfs.path(), &nas);
        let document = nas.join("smb-share").join("Domy").join("a.ketchup");
        assert_eq!(presence(&document, Some(home.path())), Presence::Unknown);

        let elsewhere = tempfile::tempdir().unwrap();
        let by_link = elsewhere.path().join("home");
        link_folder(home.path(), &by_link);
        let deleted = home.path().join("Old").join("a.ketchup");
        assert_eq!(presence(&deleted, Some(&by_link)), Presence::Gone);
    }

    /// Review 2026-11-07 r26 P3: `/opt` from autofs, or a share a FUSE daemon
    /// mounts, says a folder is not there while its server does not answer.
    #[test]
    fn a_folder_on_a_server_or_mounted_on_demand_is_remote() {
        let mountinfo = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
30 22 0:40 / /opt rw,relatime shared:9 - autofs systemd-1 rw,fd=50
31 22 0:41 / /home rw,relatime shared:10 - autofs systemd-1 rw,fd=51
32 31 8:2 / /home rw,relatime shared:11 - ext4 /dev/sda2 rw
33 22 0:42 / /mnt/lab\\040share rw,relatime - fuse.rclone lab: rw
34 32 0:43 / /home/u/NAS rw,nosuid - fuse.sshfs u@nas: rw
";
        for (folder, remote) in [
            ("/opt/tools", true),
            ("/usr/lib", false),
            ("/home/u/Old", false),
            ("/mnt/lab share/Domy", true),
            ("/mnt/lab", false),
            ("/home/u/NAS/Domy", true),
        ] {
            assert_eq!(
                on_remote_mount(mountinfo, Path::new(folder)),
                remote,
                "{folder}"
            );
        }
    }

    /// Review 2026-11-07 r26 capability finding on its P3 fix: a server that
    /// says a folder is not there has answered, so a folder deleted in a home
    /// folder or a lab share on NFS or CIFS is gone (review 2026-11-04 r19 P3).
    /// It says nothing while it is down; only a mount on demand not made then
    /// says a folder is not there.
    #[test]
    fn a_folder_a_server_says_is_not_there_answers() {
        let mountinfo = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
40 22 0:50 / /home rw,relatime - nfs4 server:/home rw
41 22 0:51 / /mnt/lab rw,relatime - cifs //server/lab rw
42 22 0:52 / /opt rw,relatime shared:9 - autofs systemd-1 rw,fd=50
43 42 0:53 / /opt/tools rw,relatime - nfs4 server:/tools rw
";
        for (folder, remote) in [
            ("/home/u/Projects", false),
            ("/mnt/lab/Domy", false),
            ("/opt/tools/bin", false),
            ("/opt/other", true),
        ] {
            assert_eq!(
                on_remote_mount(mountinfo, Path::new(folder)),
                remote,
                "{folder}"
            );
        }
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
