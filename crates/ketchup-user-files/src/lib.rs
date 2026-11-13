//! The files of a user's own that Kečup writes in their home: settings, logs and
//! plugin stages (review 2026-11-13 R1113-SEC-600..602).
//!
//! Under `sudo` this process is root while the home is the user's, who may have
//! put a link anywhere in it. So every folder and file there is made, locked and
//! written by a thread that is that user ([`UserFolder::as_owner`]), whom the
//! system refuses root's folders and files, and a file beside another is never
//! opened through a link in its place ([`side_file`]). The user is the owner of
//! the base folder the path was found in, `HOME` or the XDG folder that replaces
//! it; a base whose owner is not also the owner of `HOME` is refused (R1113-SEC-602:
//! with `sudo -E` and `HOME=/root`, root wrote the user's `XDG_CONFIG_HOME` as root).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod user;
#[cfg(unix)]
pub use user::{as_user, effective_group, effective_user};

/// A folder of the user's and the base folder the environment named it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserFolder {
    base: PathBuf,
    folder: PathBuf,
}

impl UserFolder {
    /// `<base>/<below…>/<name>` for the first `(variable, below)` of `bases` whose
    /// variable names an absolute folder.
    #[must_use]
    pub fn find(bases: &[(&str, &[&str])], name: &str) -> Option<Self> {
        bases.iter().find_map(|(variable, below)| {
            let base = PathBuf::from(std::env::var_os(variable)?);
            let folder = below
                .iter()
                .fold(base.clone(), |folder, part| folder.join(part));
            base.is_absolute().then(|| Self {
                folder: folder.join(name),
                base,
            })
        })
    }

    /// The settings folder `name`: in the folder `windows` names on Windows,
    /// `~/Library/Application Support` on macOS and `$XDG_CONFIG_HOME` or
    /// `~/.config` elsewhere.
    #[must_use]
    pub fn settings(windows: &str, name: &str) -> Option<Self> {
        if cfg!(windows) {
            Self::find(&[(windows, &[])], name)
        } else if cfg!(target_os = "macos") {
            Self::find(&[("HOME", &["Library", "Application Support"])], name)
        } else {
            Self::find(&[("XDG_CONFIG_HOME", &[]), ("HOME", &[".config"])], name)
        }
    }

    /// The log folder `name`: in `%LOCALAPPDATA%` on Windows, `~/Library/Logs` on
    /// macOS and `$XDG_STATE_HOME` or `~/.local/state` elsewhere.
    #[must_use]
    pub fn logs(name: &str) -> Option<Self> {
        if cfg!(windows) {
            Self::find(&[("LOCALAPPDATA", &[])], name)
        } else if cfg!(target_os = "macos") {
            Self::find(&[("HOME", &["Library", "Logs"])], name)
        } else {
            Self::find(
                &[("XDG_STATE_HOME", &[]), ("HOME", &[".local", "state"])],
                name,
            )
        }
    }

    /// `folder` in `base`, as [`Self::find`] would have found it.
    #[must_use]
    pub fn at(base: PathBuf, folder: PathBuf) -> Self {
        Self { base, folder }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.folder
    }

    #[must_use]
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Runs `work` on this folder: under root as the folder's user, the owner of
    /// its base, unless that is root itself; otherwise as this process is.
    ///
    /// # Errors
    ///
    /// The base cannot be read, is root's but others may write into it, or belongs
    /// to another user than `HOME`; the thread cannot become the user or root
    /// again; or what `work` answers.
    pub fn as_owner<T>(&self, work: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        #[cfg(unix)]
        if effective_user() == 0 {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let owner = owner_of_base(&self.base, home.as_deref())?;
            if owner.0 != 0 {
                return as_user(owner, work);
            }
        }
        work()
    }

    /// Whether this folder is the running user's own: under root, one whose base
    /// is root's; any other user is held to their own by the system.
    #[must_use]
    pub fn is_running_users(&self) -> bool {
        #[cfg(unix)]
        if effective_user() == 0 {
            return user_of_home(&self.base).is_ok_and(|(uid, _)| uid == 0);
        }
        true
    }
}

/// The user, and group, whose base folder `base` is, refused when `home` is
/// another user's.
#[cfg(unix)]
fn owner_of_base(base: &Path, home: Option<&Path>) -> io::Result<(u32, u32)> {
    let owner = user_of_home(base)?;
    if let Some(home) = home.filter(|home| *home != base) {
        let of_home = user_of_home(home)?;
        if of_home.0 != owner.0 {
            return Err(HomeError::OwnersDiffer {
                home: home.to_owned(),
                base: base.to_owned(),
            }
            .into());
        }
    }
    Ok(owner)
}

/// The user, and group, whose configuration in `HOME` this process writes: the
/// owner of that home, else the user `sudo` names; none without either.
///
/// # Errors
///
/// `HOME` cannot be read or is root's and others may write into it, or `sudo`'s
/// variables are no numbers.
#[cfg(unix)]
pub fn configuration_user() -> io::Result<Option<(u32, u32)>> {
    if let Some(home) = std::env::var_os("HOME") {
        return user_of_home(Path::new(&home)).map(Some);
    }
    let named = |name| match std::env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    };
    Ok(named("SUDO_UID")?.zip(named("SUDO_GID")?))
}

/// The owner of the home or base folder `home`, through a link of root's in its
/// place (`/home/u -> /data/u`). One of root's that other users may write into
/// (`HOME=/tmp`) is no one's: they could put links there for root to follow
/// (review 2026-11-12 P3). One not there says which (P3: a bare `NotFound`).
///
/// # Errors
///
/// As said: `home` cannot be read, or is root's and shared with others.
#[cfg(unix)]
pub fn user_of_home(home: &Path) -> io::Result<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    let found = std::fs::metadata(home).map_err(|error| HomeError::Unreadable {
        home: home.to_owned(),
        error,
    })?;
    if found.uid() == 0 && found.mode() & 0o022 != 0 {
        return Err(HomeError::SharedWithOthers {
            home: home.to_owned(),
        }
        .into());
    }
    Ok((found.uid(), found.gid()))
}

/// Why a home or base folder holds no files of a user's to write.
#[cfg(unix)]
#[derive(Debug)]
enum HomeError {
    Unreadable { home: PathBuf, error: io::Error },
    SharedWithOthers { home: PathBuf },
    OwnersDiffer { home: PathBuf, base: PathBuf },
}

#[cfg(unix)]
impl std::fmt::Display for HomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable { home, .. } => write!(
                f,
                "the home {} that HOME names cannot be read; set HOME to your home folder",
                home.display()
            ),
            Self::SharedWithOthers { home } => write!(
                f,
                "the home {} that HOME names is root's and other users may write into it; \
                 set HOME to root's own home or run without sudo",
                home.display()
            ),
            Self::OwnersDiffer { home, base } => write!(
                f,
                "the folder {} belongs to another user than the home {} that HOME names; \
                 run without sudo, or with HOME and the XDG folders of one user",
                base.display(),
                home.display()
            ),
        }
    }
}

#[cfg(unix)]
impl std::error::Error for HomeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { error, .. } => Some(error),
            Self::SharedWithOthers { .. } | Self::OwnersDiffer { .. } => None,
        }
    }
}

#[cfg(unix)]
impl From<HomeError> for io::Error {
    fn from(error: HomeError) -> Self {
        let kind = match &error {
            HomeError::Unreadable { error, .. } => error.kind(),
            HomeError::SharedWithOthers { .. } | HomeError::OwnersDiffer { .. } => {
                io::ErrorKind::PermissionDenied
            }
        };
        Self::new(kind, error)
    }
}

/// Opens a file beside a user's file (a lock, a count, a damaged copy) to write:
/// made its owner's only, and not through a link in its place (review 2026-11-12
/// R1112-SEC-500: one there was followed wherever it led).
///
/// # Errors
///
/// It cannot be opened, or a link is in its place.
pub fn side_file(path: &Path, truncate: bool) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(truncate);
    private(&mut options).open(path)
}

/// A new file beside a user's file, there not yet, nor a link in its place: a
/// part file renamed over the file once written.
///
/// # Errors
///
/// It cannot be made, or something is there already.
pub fn new_side_file(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    private(&mut options).open(path)
}

/// A file of the user's to append to, a log: its owner's only, an older one
/// readable by others closed too, and not through a link in its place (review
/// 2026-11-13 R1113-SEC-601: under root a link there had root append to and
/// close a file of root's).
///
/// # Errors
///
/// It cannot be opened, or a link is in its place.
pub fn append_private(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    let file = private(&mut options).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// `folder` and the folders above it, made its owner's only; one there already
/// is closed to others too, though not through a link in its place (R1113-SEC-601:
/// the link's target was closed instead).
///
/// # Errors
///
/// A folder cannot be made, or its permissions changed.
pub fn create_private_folder(folder: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)?;
        if std::fs::symlink_metadata(folder)?.is_dir() {
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(folder)
    }
}

/// `options` making a file its owner's only and refusing a link in its place.
fn private(options: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
}

#[cfg(test)]
mod tests;
