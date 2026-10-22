//! The bytes a plugin runs are the bytes the user allowed (SEC-2). A plugin
//! folder is copied into a private stage before it is hashed and runs from
//! there, so a file changed or added in the folder while the consent prompt is
//! open, or later, never runs. Every hashed file, staged or outside the folder
//! (the interpreter, a script named on the command line), stays open from its
//! hash until the plugin process has ended; on Windows without write or delete
//! sharing, so nobody can change, replace or rename it meanwhile. Elsewhere the
//! files are hashed again before the process starts.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The most files a plugin run hashes in its folder, and as many again outside
/// it: the interpreter and the files its command line names (review
/// 2026-10-18 MCP-5).
pub(super) const MAX_PLUGIN_FILES: usize = 1_000;
/// The most bytes those files hold, in the folder and again outside it.
pub(super) const MAX_PLUGIN_BYTES: u64 = 256 * 1024 * 1024;

/// Why a run reads no more files: `what` ("the plugin folder holds") is more.
pub(super) fn past_limits(what: &str) -> String {
    format!(
        "{what} more than {MAX_PLUGIN_FILES} files or {} MiB",
        MAX_PLUGIN_BYTES / (1024 * 1024)
    )
}

/// Opens `path` for reading; on Windows nobody may write, rename or delete
/// it while the handle is open. Only a regular file opens (`open_regular`).
pub(super) fn open_held(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        options.share_mode(FILE_SHARE_READ);
    }
    open_checked(path, options)
}

/// Opens `path` for reading if the opened handle is a regular file. On Unix
/// the open does not wait for a writer, so a FIFO in its place is refused
/// instead of blocking the open forever.
pub(super) fn open_regular(path: &Path) -> io::Result<File> {
    open_checked(path, fs::OpenOptions::new())
}

fn open_checked(path: &Path, mut options: fs::OpenOptions) -> io::Result<File> {
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(file)
}

/// What a plugin run holds until its process has ended: the hashed files,
/// then the stage they were copied into, removed last. A file can be added to
/// a folder however its other files are held, so before the process starts
/// the stage must still hold exactly the hashed files (`unchanged`).
#[derive(Debug, Default)]
pub(super) struct HeldFiles {
    files: Vec<File>,
    /// The stage's files by relative path, with the SHA-256 the consent shows.
    staged: BTreeMap<PathBuf, String>,
    /// The files outside the stage (interpreter, scripts on the command line)
    /// by absolute path, with their SHA-256.
    outside: BTreeMap<PathBuf, String>,
    /// The bytes hashed in the stage and outside it, each within `MAX_PLUGIN_BYTES`.
    staged_bytes: u64,
    outside_bytes: u64,
    /// Set when the request that reads the files is cancelled.
    cancelled: Arc<AtomicBool>,
    stage: Option<tempfile::TempDir>,
}

impl HeldFiles {
    /// A new private stage for a plugin folder's copy, in the user's own
    /// directory: from the shared temporary directory an interpreter could
    /// load a module another user planted beside or above the stage (Node's
    /// `node_modules`, review 2026-10-17 SEC-7). Reading stops at the next
    /// file once `cancelled` is set.
    pub(super) fn staged(cancelled: Arc<AtomicBool>) -> io::Result<Self> {
        let root = stage_root()?;
        fs::create_dir_all(&root)?;
        Ok(Self {
            stage: Some(
                tempfile::Builder::new()
                    .prefix("ketchup-plugin-")
                    .tempdir_in(root)?,
            ),
            cancelled,
            ..Self::default()
        })
    }

    pub(super) fn stage(&self) -> Option<&Path> {
        self.stage.as_ref().map(tempfile::TempDir::path)
    }

    /// Fails with `Interrupted` once the request reading the files is cancelled.
    pub(super) fn check_cancelled(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "the request was cancelled while the plugin's files were read",
            ));
        }
        Ok(())
    }

    /// Opens `path` held and returns its SHA-256, read through that handle.
    /// Past `MAX_PLUGIN_FILES` or `MAX_PLUGIN_BYTES` in the stage, or outside
    /// it, the file is refused with `FileTooLarge` and nothing more is read.
    pub(super) fn hash(&mut self, path: &Path) -> io::Result<String> {
        self.check_cancelled()?;
        let relative = self
            .stage()
            .and_then(|stage| path.strip_prefix(stage).ok())
            .map(Path::to_owned);
        let (files, bytes, what) = match relative {
            Some(_) => (
                self.staged.len(),
                self.staged_bytes,
                "the plugin folder holds",
            ),
            None => (
                self.outside.len(),
                self.outside_bytes,
                "the plugin's command line names, outside the plugin folder,",
            ),
        };
        let too_much = || io::Error::new(io::ErrorKind::FileTooLarge, past_limits(what));
        if files >= MAX_PLUGIN_FILES {
            return Err(too_much());
        }
        let remaining = MAX_PLUGIN_BYTES.saturating_sub(bytes);
        let file = open_held(path)?;
        let mut limited = io::Read::take(&file, remaining + 1);
        let sha256 = ketchup_model::graph::sha256_reader_hex(&mut limited)?;
        let read = remaining + 1 - limited.limit();
        if read > remaining {
            return Err(too_much());
        }
        self.files.push(file);
        match relative {
            Some(relative) => {
                self.staged_bytes += read;
                self.staged.insert(relative, sha256.clone());
            }
            None => {
                self.outside_bytes += read;
                self.outside.insert(path.to_owned(), sha256.clone());
            }
        }
        Ok(sha256)
    }

    /// The stage holds exactly the files hashed in it, with the same bytes:
    /// nothing was added or changed since the consent was asked for.
    pub(super) fn unchanged(&self) -> io::Result<()> {
        let changed = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
        // A file outside the stage is not copied: only on Windows does its held
        // handle keep it from being rewritten, so it is hashed again everywhere.
        for (path, sha256) in &self.outside {
            let file = open_regular(path)?;
            if ketchup_model::graph::sha256_reader_hex(&file)? != *sha256 {
                return Err(changed(format!(
                    "{} changed after it was allowed",
                    path.display()
                )));
            }
        }
        let Some(stage) = self.stage() else {
            return Ok(());
        };
        let mut found = Vec::new();
        let mut pending = vec![PathBuf::new()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(stage.join(&directory))? {
                let entry = entry?;
                let relative = directory.join(entry.file_name());
                if fs::symlink_metadata(entry.path())?.is_dir() {
                    pending.push(relative);
                } else if self.staged.contains_key(&relative) {
                    found.push(relative);
                } else {
                    return Err(changed(format!(
                        "{} was added to the plugin's files after they were allowed",
                        relative.display()
                    )));
                }
            }
        }
        if found.len() != self.staged.len() {
            return Err(changed(
                "a file of the plugin was removed after it was allowed".to_owned(),
            ));
        }
        // Elsewhere a held file is not locked: its bytes are hashed again.
        #[cfg(not(windows))]
        for (relative, sha256) in &self.staged {
            let file = open_regular(&stage.join(relative))?;
            if ketchup_model::graph::sha256_reader_hex(&file)? != *sha256 {
                return Err(changed(format!(
                    "{} changed after it was allowed",
                    relative.display()
                )));
            }
        }
        Ok(())
    }

    /// Opens `path` held and returns at most `limit + 1` of its bytes.
    pub(super) fn read(&mut self, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
        let file = open_held(path)?;
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut io::Read::take(&file, limit + 1), &mut bytes)?;
        self.files.push(file);
        Ok(bytes)
    }
}

/// Where plugin stages are made: `%LOCALAPPDATA%\Ketchup\plugin-stages` on
/// Windows, `$XDG_RUNTIME_DIR` (or `~/.cache`) `/ketchup/plugin-stages`
/// elsewhere. Without such a directory no plugin runs.
fn stage_root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Ketchup"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|path| path.join("ketchup"));
    base.filter(|path| path.is_absolute())
        .map(|path| path.join("plugin-stages"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no per-user directory for a plugin stage",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_held_file_cannot_be_written_renamed_or_deleted_but_runs_and_reads() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("plugin.py");
        fs::write(&script, "x = 1\n").unwrap();
        let mut held = HeldFiles::default();
        let sha256 = held.hash(&script).unwrap();
        assert_eq!(sha256, ketchup_model::graph::sha256_hex(b"x = 1\n"));
        assert!(fs::write(&script, "x = 2\n").is_err());
        assert!(fs::rename(&script, directory.path().join("moved.py")).is_err());
        assert!(fs::remove_file(&script).is_err());
        assert_eq!(fs::read(&script).unwrap(), b"x = 1\n");
        drop(held);
        fs::write(&script, "x = 2\n").unwrap();
    }

    /// Review 2026-10-15 status: on Linux and macOS a held file outside the
    /// plugin folder (an interpreter, a script on the command line) is not
    /// locked, so a file rewritten while the consent prompt was open must stop
    /// the run. Dropping the handles stands in for a platform without locks.
    #[test]
    fn a_file_outside_the_stage_rewritten_after_hashing_stops_the_run() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("plugin.py");
        fs::write(&script, "x = 1\n").unwrap();
        let mut held = HeldFiles::staged(Arc::default()).unwrap();
        held.hash(&script).unwrap();
        held.unchanged().unwrap();
        held.files.clear();
        fs::write(&script, "x = 2\n").unwrap();
        let error = held.unchanged().unwrap_err();
        assert!(
            error.to_string().contains("changed after it was allowed"),
            "{error}"
        );
    }

    #[test]
    fn a_stage_is_removed_with_its_files_after_the_run() {
        let mut held = HeldFiles::staged(Arc::default()).unwrap();
        let stage = held.stage().unwrap().to_owned();
        fs::write(stage.join("plugin.py"), "x = 1\n").unwrap();
        held.hash(&stage.join("plugin.py")).unwrap();
        drop(held);
        assert!(!stage.exists());
    }

    /// Review 2026-10-18 MCP-5: the files a command line names are counted
    /// with their bytes, the one past `MAX_PLUGIN_FILES` is refused unread, and
    /// a cancelled request reads no further file.
    #[test]
    fn hashing_stops_past_the_file_limit_and_once_cancelled() {
        let directory = tempfile::tempdir().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut held = HeldFiles::staged(Arc::clone(&cancelled)).unwrap();
        for index in 0..MAX_PLUGIN_FILES {
            let file = directory.path().join(format!("{index}.py"));
            fs::write(&file, "x = 1\n").unwrap();
            held.hash(&file).unwrap();
        }
        assert_eq!(held.outside_bytes, 6 * MAX_PLUGIN_FILES as u64);
        let last = directory.path().join("last.py");
        fs::write(&last, "x = 1\n").unwrap();
        let error = held.hash(&last).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert!(
            error
                .to_string()
                .contains("more than 1000 files or 256 MiB"),
            "{error}"
        );
        assert!(!held.outside.contains_key(&last));

        let mut held = HeldFiles::staged(Arc::clone(&cancelled)).unwrap();
        cancelled.store(true, Ordering::Release);
        let error = held.hash(&directory.path().join("0.py")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(held.outside.is_empty());
    }
}
