//! A file written whole beside where it goes and moved there, keeping the
//! permissions of the file it replaces and the symbolic links to it (review
//! 2026-11-22 R1122ac-PS-1101: a document with 0664 became 0600, the
//! temporary file's, its `.recovery` too, and a save through a link made the
//! link a plain file and left the file it pointed to as it was).

use super::{FileIdentity, FilePersistenceError};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The file a chain of symbolic links at `path` ends in, though it does not
/// exist yet: a save through a link writes where it points and keeps the link.
pub(super) fn link_target(path: &Path) -> PathBuf {
    let mut file = path.to_owned();
    // A loop of links stops where the system would.
    let mut hops = 0;
    while hops < 40
        && let Ok(target) = fs::read_link(&file)
    {
        file = file.parent().unwrap_or(Path::new(".")).join(target);
        hops += 1;
    }
    file
}

/// Writes `bytes` to a temporary file beside `path` and moves it there, over an
/// existing file only when `replace`; without it an existing `path` is a
/// conflict. The file has the permissions of `like`, else those a new file gets.
pub(super) fn write_staged(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    before_persist: impl FnOnce() -> Result<(), FilePersistenceError>,
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
    like: Option<&fs::Metadata>,
) -> Result<(), FilePersistenceError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = staged_file(parent, like)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    before_persist()?;
    let persisted = if replace {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    persisted.map_err(|error| match error.error.kind() {
        io::ErrorKind::AlreadyExists if !replace => FilePersistenceError::ExternalConflict,
        _ => FilePersistenceError::Io(error.error),
    })?;
    sync_parent(parent).map_err(|error| FilePersistenceError::Published {
        path: path.to_owned(),
        identity: FileIdentity::from_bytes(bytes),
        error,
    })?;
    Ok(())
}

/// A temporary file in `parent` with the permissions of `like`, else created
/// as any new file is, under the umask.
#[cfg(unix)]
fn staged_file(parent: &Path, like: Option<&fs::Metadata>) -> io::Result<tempfile::NamedTempFile> {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o666))
        .tempfile_in(parent)?;
    if let Some(like) = like {
        temporary.as_file().set_permissions(like.permissions())?;
    }
    Ok(temporary)
}

/// A temporary file in `parent`; its attributes here are those of any new file.
#[cfg(not(unix))]
fn staged_file(parent: &Path, _like: Option<&fs::Metadata>) -> io::Result<tempfile::NamedTempFile> {
    tempfile::NamedTempFile::new_in(parent)
}
