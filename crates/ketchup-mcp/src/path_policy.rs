//! The one path policy for files the window or this server touches on behalf of
//! a client: absolute, bounded and on a local drive. It reads nothing from a share
//! (on Windows only the drive table, to see a drive letter mapped to a share; on
//! macOS and Linux the mount table, and the symbolic links of the path's directories
//! that are on local mounts), so a network path is refused before any request
//! reaches its server (and before Windows sends it the user's NTLM hash).

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf, Prefix};

/// Longest accepted path, in bytes.
const PATH_BYTES: usize = 4096;

/// Windows device names, which name a device in any directory and with any
/// extension (`C:\work\nul.pdf`).
const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Whether `path` is an absolute path of a file on a local drive. On Windows
/// that is a drive letter not mapped to a share and a root (`C:\…` or `C:/…`)
/// and nothing else: no share or device however its separators are mixed (`\\server`, `\/server`,
/// `\\?\`, `\\.\`), no alternate data stream (`file.pdf:stream`) and no device
/// name. On macOS and Linux it is a path on no network mount, nor one the automounter
/// serves (on macOS also `/net`, `/Network`, `/home`; on Linux an `autofs` mount).
#[must_use]
pub fn local_absolute_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_BYTES
        && !path.contains('\0')
        && Path::new(path).is_absolute()
        && (!cfg!(windows) || local_windows_path(path, remote_drive))
        && (!cfg!(any(target_os = "macos", target_os = "linux"))
            || mounts().is_ok_and(|m| physically_local_path(path, &m, AUTOMOUNTED, read_link)))
}

/// Top-level directories macOS's automounter serves from the network; on Linux
/// `/home` is a local directory and the automounter shows as an `autofs` mount.
const AUTOMOUNTED: &[&str] = if cfg!(target_os = "macos") {
    &["net", "network", "home"]
} else {
    &[]
};

/// Linux file systems that reach another computer, or that the automounter mounts
/// from one when a path under them is touched (review 2026-11-02 SEC-101). Besides
/// these, every FUSE file system without a block device (see [`linux_local_type`]).
/// Under WSL the Windows drives are `9p` or `drvfs`, which may be a mapped share; a
/// virtual machine reaches its host by `9p` or `virtiofs`, under VirtualBox by
/// `vboxsf` and Parallels by `prl_fs`; `gpfs` is a cluster's (review 2026-11-04
/// r19, 2026-11-06 P3).
const NETWORK_FILE_SYSTEMS: [&str; 19] = [
    "autofs",
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "afs",
    "ceph",
    "glusterfs",
    "lustre",
    "ncpfs",
    "coda",
    "davfs",
    "9p",
    "drvfs",
    "virtiofs",
    "vboxsf",
    "prl_fs",
    "gpfs",
];

/// Whether a Linux file system type is local. A FUSE file system without a block
/// device (`fuse`, `fuse.<program>`) is whatever its program serves, and those
/// programs mostly reach the network (`sshfs`, `rclone`, `s3fs`, `gvfsd-fuse`), so
/// none is taken as local (review 2026-11-04 SEC-101); `fuseblk` reads a disk.
fn linux_local_type(kind: &str) -> bool {
    !(NETWORK_FILE_SYSTEMS.contains(&kind) || kind == "fuse" || kind.starts_with("fuse."))
}

/// The data volume's firmlinks make `/System/Volumes/Data/Volumes` the same
/// directory as `/Volumes`, and so for every top-level directory.
const DATA_VOLUME: [&str; 3] = ["system", "volumes", "data"];

/// A mount point as components, and whether it is a local file system.
pub(crate) struct Mount {
    on: Vec<String>,
    local: bool,
    /// A Linux `proc` file system, whose links to a process's root, working
    /// directory or files lead the kernel to the object, not to their text.
    proc: bool,
    /// Whether a name is matched ignoring case and Unicode form, as the default
    /// macOS file system does; `on` is then kept [`folded`]. A Linux name is
    /// matched exactly (review 2026-11-09 SEC-303).
    folds: bool,
}

impl Mount {
    /// A macOS mount point, matched ignoring case and Unicode form.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn new(on: &str, local: bool) -> Self {
        Self {
            on: lexical_components(on)
                .iter()
                .map(|name| folded(name))
                .collect(),
            local,
            proc: false,
            folds: true,
        }
    }

    /// A Linux mount point, matched by its exact name.
    #[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
    fn exact(on: &str, local: bool, proc: bool) -> Self {
        Self {
            on: lexical_components(on),
            local,
            proc,
            folds: false,
        }
    }

    fn holds(&self, components: &[String]) -> bool {
        components.len() >= self.on.len()
            && self.on.iter().zip(components).all(|(on, name)| {
                if self.folds {
                    *on == folded(name)
                } else {
                    on == name
                }
            })
    }
}

/// The mount `components` lie on: the deepest, and of two at one point the
/// later, on top (systemd's autofs trigger under binfmt_misc); `max_by_key`
/// keeps the last of equals.
fn mount_of<'a>(components: &[String], mounts: &'a [Mount]) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|mount| mount.holds(components))
        .max_by_key(|mount| mount.on.len())
}

fn local_mounted_path(path: &str, mounts: &[Mount], automounted: &[&str]) -> bool {
    let components = lexical_components(path);
    let through_data_volume = (components.len() >= DATA_VOLUME.len()
        && DATA_VOLUME
            .iter()
            .zip(&components)
            .all(|(volume, name)| *volume == folded(name)))
    .then(|| &components[DATA_VOLUME.len()..]);
    [Some(&components[..]), through_data_volume]
        .into_iter()
        .flatten()
        .all(|components| {
            !components
                .first()
                .is_some_and(|top| automounted.contains(&folded(top).as_str()))
                && mount_of(components, mounts).is_none_or(|mount| mount.local)
        })
}

/// Most symbolic links followed in one path, as Linux's `MAXSYMLINKS`.
const LINK_HOPS: usize = 40;

/// Whether `path` stays on local mounts once its symbolic links are followed (review
/// 2026-11-04 SEC-101). Each directory is checked against the mount table before its
/// link is read, so nothing on a share is ever read; a link that cannot be read, or
/// too many of them, is not local.
fn physically_local_path(
    path: &str,
    mounts: &[Mount],
    automounted: &[&str],
    read_link: impl Fn(&Path) -> std::io::Result<Option<PathBuf>>,
) -> bool {
    let mut pending: VecDeque<String> = walk_components(path).collect();
    let mut resolved: Vec<String> = Vec::new();
    let mut hops = 0;
    while let Some(component) = pending.pop_front() {
        if component == ".." {
            resolved.pop();
            continue;
        }
        resolved.push(component);
        let at = format!("/{}", resolved.join("/"));
        if !local_mounted_path(&at, mounts, automounted) {
            return false;
        }
        match read_link(Path::new(&at)) {
            Ok(None) => {}
            Ok(Some(target)) => {
                hops += 1;
                if hops > LINK_HOPS {
                    return false;
                }
                let target = target.to_string_lossy();
                // procfs's own links (`self`, `mounts`) are relative names in
                // it; a magic one reads absolute (`root`, `cwd` of a process in
                // another mount namespace) or names no path (`socket:[99]`).
                if (target.starts_with('/') || target.contains(':'))
                    && mount_of(&lexical_components(&at), mounts).is_some_and(|mount| mount.proc)
                {
                    return false;
                }
                resolved.pop();
                if target.starts_with('/') {
                    resolved.clear();
                }
                let rest = std::mem::take(&mut pending);
                pending.extend(walk_components(&target));
                pending.extend(rest);
            }
            Err(error) => {
                // A link that cannot be read might lead anywhere, onto a share too.
                debug_assert_ne!(
                    error.kind(),
                    ErrorKind::NotFound,
                    "a missing name is no link"
                );
                return false;
            }
        }
    }
    true
}

/// A Unix path's names and `..`, without the root and `.`.
fn walk_components(path: &str) -> impl Iterator<Item = String> + '_ {
    path.split('/')
        .filter(|name| !name.is_empty() && *name != ".")
        .map(str::to_owned)
}

/// A link as this computer's file system has it; a name that is not a link, or not
/// there yet, is not one.
fn read_link(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(path).map(Some),
        Ok(_) => Ok(None),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The path's directories, with `.` dropped and `..` applied.
fn lexical_components(path: &str) -> Vec<String> {
    let mut components = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(name) => {
                components.push(name.to_string_lossy().into_owned());
            }
            Component::ParentDir => {
                components.pop();
            }
            _ => {}
        }
    }
    components
}

/// A name lowercase and composed (NFC): the default macOS file system ignores case
/// and Unicode form alike, so `Výkresy` in NFD names the share mounted in NFC
/// (review 2026-11-03 SEC-142).
fn folded(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.to_lowercase().nfc().collect()
}

#[cfg(target_os = "macos")]
fn mounts() -> std::io::Result<Vec<Mount>> {
    let table = crate::local_auth::mount_table()?;
    Ok(table
        .iter()
        .map(|(on, local)| Mount::new(on, *local))
        .collect())
}

#[cfg(target_os = "linux")]
fn mounts() -> std::io::Result<Vec<Mount>> {
    Ok(linux_mounts(
        &std::fs::read_to_string("/proc/self/mounts")?,
        read_link,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mounts() -> std::io::Result<Vec<Mount>> {
    Ok(Vec::new())
}

/// The mounts of a Linux mount table (`/proc/self/mounts`): `device mount-point type
/// options ...` per line, a space, tab, newline or backslash in the mount point written
/// as its octal escape (`\040`). An overlay is local when each of its layers is an
/// absolute path that stays on local mounts of those mounted before it, its links
/// followed (review 2026-11-03 SEC-141: one with a layer on a share passed; 2026-11-09
/// SEC-302: so did one through a link onto a share, or a relative one, whose working
/// directory the table does not show).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn linux_mounts(
    table: &str,
    read_link: impl Fn(&Path) -> std::io::Result<Option<PathBuf>>,
) -> Vec<Mount> {
    let mut mounts = Vec::new();
    for line in table.lines() {
        let mut fields = line.split(' ');
        let (Some(_device), Some(on), Some(kind)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let options = fields.next().unwrap_or("");
        let local = linux_local_type(kind)
            && (kind != "overlay"
                || overlay_layers(options).iter().all(|layer| {
                    layer.starts_with('/') && physically_local_path(layer, &mounts, &[], &read_link)
                }));
        mounts.push(Mount::exact(
            &unescape_mount_point(on),
            local,
            kind == "proc",
        ));
    }
    mounts
}

/// The directories an overlay's mount options name as its layers: those of `lowerdir`
/// (`:` between them, `\:` within a name), `upperdir`, `workdir`, and the `lowerdir+`
/// and `datadir+` of a newer kernel.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn overlay_layers(options: &str) -> Vec<String> {
    let mut layers = Vec::new();
    for (key, value) in options
        .split(',')
        .filter_map(|option| option.split_once('='))
    {
        match key {
            "lowerdir" => {
                let mut layer = String::new();
                let mut characters = value.chars();
                while let Some(character) = characters.next() {
                    match character {
                        '\\' if characters.as_str().starts_with(':') => {
                            layer.push(':');
                            characters.next();
                        }
                        ':' => layers.push(std::mem::take(&mut layer)),
                        _ => layer.push(character),
                    }
                }
                layers.push(layer);
            }
            "upperdir" | "workdir" | "lowerdir+" | "datadir+" => layers.push(value.to_owned()),
            _ => {}
        }
    }
    layers
        .iter()
        .filter(|layer| !layer.is_empty())
        .map(|layer| unescape_mount_point(layer))
        .collect()
}

fn unescape_mount_point(on: &str) -> String {
    let bytes = on.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let octal = bytes.get(at + 1..at + 4).filter(|digits| {
            bytes[at] == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        });
        match octal {
            Some(digits) => {
                let value = digits
                    .iter()
                    .fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0'));
                out.push(u8::try_from(value).unwrap_or(b'?'));
                at += 4;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

#[cfg(test)]
mod mount_tests {
    use super::{local_mounted_path, physically_local_path};
    use std::path::Path;

    /// A mount table read where no layer of an overlay is a link.
    fn linux_mounts(table: &str) -> Vec<super::Mount> {
        super::linux_mounts(table, |_: &Path| Ok(None))
    }

    // Review 2026-11-09 SEC-302: an overlay's layer reached through a link onto a
    // share, or named relative to a working directory this table does not show,
    // passed as local.
    #[test]
    fn an_overlay_layer_through_a_link_onto_a_share_or_relative_is_not_local() {
        use std::path::PathBuf;
        let table = "/dev/sda1 / ext4 rw 0 0\n\
                     nas:/export /mnt/nfs nfs4 rw 0 0\n\
                     overlay /mnt/linked overlay rw,lowerdir=/home/me/nas/base,\
                     upperdir=/home/me/u,workdir=/home/me/w 0 0\n\
                     overlay /mnt/relative overlay rw,lowerdir=base,\
                     upperdir=/home/me/u,workdir=/home/me/w 0 0\n\
                     overlay /mnt/local overlay rw,lowerdir=/home/me/disk/base,\
                     upperdir=/home/me/u,workdir=/home/me/w 0 0\n";
        let mounts = super::linux_mounts(table, |path: &Path| {
            Ok(match path.to_str() {
                Some("/home/me/nas") => Some(PathBuf::from("/mnt/nfs")),
                Some("/home/me/disk") => Some(PathBuf::from("/srv/disk")),
                Some(other) if other.starts_with("/mnt/nfs") => {
                    panic!("read a link on the share: {other}")
                }
                _ => None,
            })
        });
        assert!(local_mounted_path("/mnt/local/x.pdf", &mounts, &[]));
        for remote in ["/mnt/linked/x.pdf", "/mnt/relative/x.pdf"] {
            assert!(!local_mounted_path(remote, &mounts, &[]), "{remote:?}");
        }
    }

    // Review 2026-11-09 SEC-303: Linux names differ by case and Unicode form, so a
    // share at `/mnt/NAS` was found as the local disk mounted later at `/mnt/nas`.
    #[test]
    fn a_linux_mount_point_is_matched_by_its_exact_name() {
        let (composed, decomposed) = ("V\u{FD}kresy", "Vy\u{301}kresy");
        for table in [
            "/dev/sda1 / ext4 rw 0 0\n\
             nas:/export /mnt/NAS nfs4 rw 0 0\n\
             /dev/sdb1 /mnt/nas ext4 rw 0 0\n"
                .to_owned(),
            format!(
                "/dev/sda1 / ext4 rw 0 0\n\
                 nas:/export /mnt/{composed} nfs4 rw 0 0\n\
                 /dev/sdb1 /mnt/{decomposed} ext4 rw 0 0\n"
            ),
        ] {
            let mounts = linux_mounts(&table);
            let share = table.lines().nth(1).and_then(|l| l.split(' ').nth(1));
            let disk = table.lines().nth(2).and_then(|l| l.split(' ').nth(1));
            let (share, disk) = (share.expect("share"), disk.expect("disk"));
            assert!(
                !local_mounted_path(&format!("{share}/x.pdf"), &mounts, &[]),
                "{share}"
            );
            assert!(local_mounted_path(&format!("{disk}/x.pdf"), &mounts, &[]));
        }
    }

    // Review 2026-11-02 SEC-101: on Linux a share and an automounted directory passed.
    #[test]
    fn a_path_on_a_linux_network_or_automounted_file_system_is_not_local() {
        let mounts = linux_mounts(
            "/dev/nvme0n1p2 / ext4 rw,relatime 0 0\n\
             /dev/nvme0n1p3 /home ext4 rw,relatime 0 0\n\
             //nas/backup /mnt/backup cifs rw,vers=3.1.1 0 0\n\
             nas:/export /srv/export nfs4 rw 0 0\n\
             systemd-1 /net autofs rw,fd=40 0 0\n\
             auto.misc /misc autofs rw 0 0\n\
             user@host:/ /home/me/remote fuse.sshfs rw 0 0\n\
             //nas/shared\\040files /mnt/shared\\040files smb3 rw 0 0\n\
             /dev/sdb1 /mnt/usb\\040stick vfat rw 0 0\n\
             gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw 0 0\n\
             drive: /home/me/drive fuse.rclone rw 0 0\n\
             s3fs /mnt/bucket fuse.s3fs rw 0 0\n\
             cloud /mnt/cloud fuse rw 0 0\n\
             /dev/sdc1 /mnt/ntfs fuseblk rw 0 0\n\
             C:\\134 /mnt/c 9p rw,aname=drvfs 0 0\n\
             D: /mnt/d drvfs rw 0 0\n\
             share /mnt/host virtiofs rw 0 0\n\
             scale /gpfs/projects gpfs rw 0 0\n\
             host /media/sf_host vboxsf rw 0 0\n\
             none /media/psf/Home prl_fs rw 0 0\n",
        );
        for local in [
            "/home/me/desk.pdf",
            "/mnt/usb stick/x.pdf",
            "/mnt/ntfs/x.pdf",
            "/mnt/backup2/x.pdf",
            "/tmp/x.pdf",
        ] {
            assert!(local_mounted_path(local, &mounts, &[]), "{local:?}");
        }
        for remote in [
            "/mnt/backup/x.pdf",
            "/mnt/./backup/x.pdf",
            "/srv/export/x.pdf",
            "/net/server/x.pdf",
            "/misc/cd/x.pdf",
            "/home/me/remote/x.pdf",
            "/mnt/shared files/x.pdf",
            "/mnt/usb stick/../backup/x.pdf",
            // Review 2026-11-04 SEC-101: FUSE file systems the list did not name.
            "/run/user/1000/gvfs/smb-share:server=nas,share=x/x.pdf",
            "/home/me/drive/x.pdf",
            "/mnt/bucket/x.pdf",
            "/mnt/cloud/x.pdf",
            // Review 2026-11-04 r19: the Windows drives of WSL (9p, drvfs), a virtual
            // machine's host folder (virtiofs) and a cluster file system (gpfs).
            "/mnt/c/Users/x.pdf",
            "/mnt/d/x.pdf",
            "/mnt/host/x.pdf",
            "/gpfs/projects/x.pdf",
            // Review 2026-11-06 P3: the host folders of VirtualBox and Parallels.
            "/media/sf_host/x.pdf",
            "/media/psf/Home/x.pdf",
        ] {
            assert!(!local_mounted_path(remote, &mounts, &[]), "{remote:?}");
        }
    }

    // Review 2026-11-03 SEC-141: an overlay is as local as its layers, and one whose
    // lower or upper layer lies on a share passed as local.
    #[test]
    fn an_overlay_with_a_layer_on_a_share_is_not_local() {
        let mounts = linux_mounts(
            "/dev/sda2 / ext4 rw 0 0\n\
             nas:/export /srv/export nfs4 rw 0 0\n\
             //nas/a\\040b /mnt/a\\040b cifs rw 0 0\n\
             overlay /var/lib/docker/overlay2/x/merged overlay \
             rw,lowerdir=/var/lib/docker/overlay2/l/A:/var/lib/docker/overlay2/l/B,\
             upperdir=/var/lib/docker/overlay2/x/diff,workdir=/var/lib/docker/overlay2/x/work 0 0\n\
             overlay /mnt/lower-share overlay rw,lowerdir=/home/me/l:/srv/export/base,\
             upperdir=/home/me/u,workdir=/home/me/w 0 0\n\
             overlay /mnt/upper-share overlay rw,lowerdir=/home/me/l,\
             upperdir=/srv/export/u,workdir=/srv/export/w 0 0\n\
             overlay /mnt/escaped overlay rw,lowerdir=/home/me/c\\:d:/mnt/a\\040b/l,\
             upperdir=/home/me/u,workdir=/home/me/w 0 0\n\
             overlay /mnt/new overlay rw,lowerdir+=/home/me/l,datadir+=/srv/export/d,\
             upperdir=/home/me/u,workdir=/home/me/w 0 0\n\
             overlay /mnt/stacked overlay rw,lowerdir=/mnt/lower-share,\
             upperdir=/home/me/u2,workdir=/home/me/w2 0 0\n",
        );
        assert!(local_mounted_path(
            "/var/lib/docker/overlay2/x/merged/etc/x.pdf",
            &mounts,
            &[]
        ));
        for remote in [
            "/mnt/lower-share/x.pdf",
            "/mnt/upper-share/x.pdf",
            "/mnt/escaped/x.pdf",
            "/mnt/new/x.pdf",
            "/mnt/stacked/x.pdf",
        ] {
            assert!(!local_mounted_path(remote, &mounts, &[]), "{remote:?}");
        }
    }

    // Review 2026-11-03 SEC-142: macOS takes a name in NFC and in NFD for one, so a
    // share mounted as `/Volumes/Výkresy` in one form was not found for a path in the
    // other, and the path was taken as local.
    #[test]
    fn a_path_in_another_unicode_form_than_its_mount_is_on_it() {
        let (composed, decomposed) = ("V\u{FD}kresy", "Vy\u{301}kresy");
        for (on, path) in [(composed, decomposed), (decomposed, composed)] {
            let mounts = [
                super::Mount::new("/", true),
                super::Mount::new(&format!("/Volumes/{on}"), false),
            ];
            let path = format!("/Volumes/{path}/x.pdf");
            assert!(!local_mounted_path(&path, &mounts, &[]), "{path:?}");
            let upper = path.to_uppercase();
            assert!(!local_mounted_path(&upper, &mounts, &[]), "{upper:?}");
        }
    }

    // Review 2026-11-04 SEC-101: the check was lexical, so a link on a local disk
    // that leads onto a share passed.
    #[test]
    fn a_symbolic_link_that_leads_onto_a_share_is_not_local() {
        use std::collections::HashMap;
        use std::io::{Error, ErrorKind};
        use std::path::{Path, PathBuf};

        let mounts = linux_mounts(
            "/dev/sda1 / ext4 rw 0 0\n\
             nas:/export /mnt/nfs nfs4 rw 0 0\n",
        );
        let links = HashMap::from([
            ("/home/me/nas", "/mnt/nfs"),
            ("/home/me/projects", "../../mnt/nfs/projects"),
            ("/home/me/desk.pdf", "/mnt/nfs/desk.pdf"),
            ("/home/me/chain", "/home/me/nas"),
            ("/home/me/local", "/srv/work"),
            ("/home/me/up", "/srv/work/sub"),
            ("/home/me/loop", "/home/me/loop"),
            ("/home/me/unreadable", ""),
        ]);
        let read_link = |path: &Path| match links.get(path.to_str().expect("a test path")) {
            Some(&"") => Err(Error::from(ErrorKind::PermissionDenied)),
            Some(target) => Ok(Some(PathBuf::from(target))),
            None if path.starts_with("/mnt/nfs") => panic!("read a link on the share: {path:?}"),
            None => Ok(None),
        };
        for local in [
            "/home/me/new.pdf",
            "/home/me/local/x.pdf",
            "/home/me/up/../x.pdf",
            // `..` after a link leaves the link's target: this is /srv/nas.
            "/home/me/local/../nas/x.pdf",
            "/srv/work/nas/x.pdf",
        ] {
            assert!(
                physically_local_path(local, &mounts, &[], read_link),
                "{local:?}"
            );
        }
        for remote in [
            "/home/me/nas/x.pdf",
            "/home/me/nas",
            "/home/me/projects/x.pdf",
            "/home/me/desk.pdf",
            "/home/me/chain/x.pdf",
            "/home/me/local/../../home/me/nas/x.pdf",
            "/home/me/loop/x.pdf",
            "/home/me/unreadable/x.pdf",
        ] {
            assert!(
                !physically_local_path(remote, &mounts, &[], read_link),
                "{remote:?}"
            );
        }
    }

    /// Review 2026-11-08 P3: `/proc/<pid>/root` of a process in another mount
    /// namespace reads `/`, but the kernel goes through that namespace's mounts,
    /// which this table does not list; `cwd`, `exe` and `fd/N` alike lead to an
    /// object, not to the text read.
    #[test]
    fn a_magic_link_of_proc_is_not_followed_as_its_text() {
        use std::collections::HashMap;
        use std::path::{Path, PathBuf};

        let mounts = linux_mounts(
            "/dev/sda1 / ext4 rw 0 0\n\
             proc /proc proc rw,nosuid 0 0\n\
             nas:/export /mnt/nfs nfs4 rw 0 0\n",
        );
        let links = HashMap::from([
            ("/proc/self", "4242"),
            ("/proc/mounts", "self/mounts"),
            ("/proc/4242/root", "/"),
            ("/proc/4242/cwd", "/home/me"),
            ("/proc/4242/fd/3", "socket:[99]"),
        ]);
        let read_link = |path: &Path| {
            Ok(links
                .get(path.to_str().expect("a test path"))
                .map(PathBuf::from))
        };
        assert!(physically_local_path(
            "/proc/mounts",
            &mounts,
            &[],
            read_link
        ));
        for magic in [
            "/proc/4242/root/home/me/x.pdf",
            "/proc/self/root/home/me/x.pdf",
            "/proc/self/cwd/x.pdf",
            "/proc/4242/fd/3",
        ] {
            assert!(
                !physically_local_path(magic, &mounts, &[], read_link),
                "{magic:?}"
            );
        }
    }

    #[test]
    fn of_two_mounts_at_one_point_the_later_one_on_top_decides() {
        let covered_trigger = linux_mounts(
            "/dev/sda1 / ext4 rw 0 0\n\
             systemd-1 /proc/sys/fs/binfmt_misc autofs rw 0 0\n\
             binfmt_misc /proc/sys/fs/binfmt_misc binfmt_misc rw 0 0\n",
        );
        assert!(local_mounted_path(
            "/proc/sys/fs/binfmt_misc/x.pdf",
            &covered_trigger,
            &[]
        ));
        let share_over_disk = linux_mounts(
            "/dev/sda1 / ext4 rw 0 0\n\
             /dev/sdb1 /mnt/data ext4 rw 0 0\n\
             //nas/data /mnt/data cifs rw 0 0\n",
        );
        assert!(!local_mounted_path(
            "/mnt/data/x.pdf",
            &share_over_disk,
            &[]
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_path_on_a_mount_of_this_computer_that_is_not_local_is_local() {
        let mounts = super::mounts().expect("the mount table of this computer");
        assert!(
            mounts
                .iter()
                .any(|mount| mount.on.is_empty() && mount.local)
        );
        for (at, mount) in mounts.iter().enumerate().filter(|(_, mount)| !mount.local) {
            if mounts[at + 1..].iter().any(|over| over.on == mount.on) {
                continue;
            }
            let path = format!("/{}/x.pdf", mount.on.join("/"));
            assert!(!super::local_absolute_path(&path), "{path}");
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::{AUTOMOUNTED, Mount, local_absolute_path, local_mounted_path};

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
            assert!(local_mounted_path(local, &mounts, AUTOMOUNTED), "{local:?}");
        }
        for remote in [
            "/Volumes/backup",
            "/Volumes/backup/x.pdf",
            "/volumes/BACKUP/x.pdf",
            "/Volumes/./backup/x.pdf",
            "/Volumes/USB/../backup/x.pdf",
            "/System/Volumes/Data/Volumes/backup/x.pdf",
        ] {
            assert!(
                !local_mounted_path(remote, &mounts, AUTOMOUNTED),
                "{remote:?}"
            );
        }
    }
}
