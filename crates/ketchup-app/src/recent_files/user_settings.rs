//! The user settings of Kečup where there is no Windows registry: the
//! language choice and File > Open Recent, in
//! `~/Library/Application Support/Ketchup/settings.txt` on macOS and in
//! `$XDG_CONFIG_HOME/Ketchup/settings.txt` or `~/.config/Ketchup/settings.txt`
//! elsewhere, the folder the Viewer keeps its settings in.
//!
//! One `key=value` per line, the value as written up to the line's end. A
//! key may repeat (one line per recent document); a line this window does
//! not know is kept as it was, so a newer Kečup's settings survive an older
//! one. A path that cannot be one line of text (a line break, or not UTF-8)
//! is not stored.

use super::RecentFilesStore;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FILE_NAME: &str = "settings.txt";
const LANGUAGE: &str = "language";
const RECENT_FILE: &str = "recent";

/// The settings file at one path.
#[derive(Clone, Debug)]
pub(crate) struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    /// The settings file of the user who started this window, when the
    /// platform names a folder for it.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn of_user() -> Option<Self> {
        let var = |name| std::env::var_os(name).map(PathBuf::from);
        let base = if cfg!(target_os = "macos") {
            var("HOME").map(|home| home.join("Library").join("Application Support"))
        } else {
            var("XDG_CONFIG_HOME")
                .filter(|base| base.is_absolute())
                .or_else(|| var("HOME").map(|home| home.join(".config")))
        };
        base.filter(|base| base.is_absolute())
            .map(|base| Self::at(base.join("Ketchup").join(FILE_NAME)))
    }

    pub(crate) const fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The lines of the file, none when there is no file. A line that is not
    /// UTF-8 is left out, and `true` says the file had one. Any other failure
    /// to read is an error, not an empty file: writing over it would lose the
    /// language, the list and a newer Kečup's lines (review 2026-11-04 r19 UI-162).
    fn read(&self) -> io::Result<(Vec<String>, bool)> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((Vec::new(), false));
            }
            Err(error) => return Err(error),
        };
        let mut damaged = false;
        let mut lines: Vec<String> = bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                let text = std::str::from_utf8(line).ok();
                damaged |= text.is_none();
                text.map(str::to_owned)
            })
            .collect();
        if bytes.ends_with(b"\n") {
            lines.pop();
        }
        Ok((lines, damaged))
    }

    /// Every value of `key`, in the order written; none when the file cannot
    /// be read.
    pub(crate) fn values(&self, key: &str) -> Vec<String> {
        self.read()
            .map(|(lines, _)| lines)
            .unwrap_or_default()
            .iter()
            .filter_map(|line| line.split_once('='))
            .filter(|(name, _)| name.trim() == key)
            .map(|(_, value)| value.to_owned())
            .collect()
    }

    /// Replace every value of `key` with `values`, the other lines kept. A
    /// file that cannot be read is not written over; one with a line that is
    /// not UTF-8 is first kept whole as `settings.txt.bad`, and its other
    /// lines carried over.
    ///
    /// Another window saving meanwhile waits on `settings.txt.lock`, so it
    /// reads this one's change before making its own; it waits two seconds at
    /// most, and a file system without locks saves without it. The new text is on the
    /// disk before it replaces the old, through a symlink to the file, keeping
    /// its permissions; a new file is its owner's only, and no part file is
    /// left behind (review 2026-11-04 r19 P3).
    pub(crate) fn replace(&self, key: &str, values: &[String]) -> io::Result<()> {
        let folder = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(folder)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("txt.lock"))?;
        let _lock = take_turn(move || lock.lock().map(|()| lock), TURN_WAIT)?;
        let (lines, damaged) = self.read()?;
        if damaged {
            std::fs::copy(&self.path, self.path.with_extension("txt.bad"))?;
        }
        let mut lines: Vec<String> = lines
            .into_iter()
            .filter(|line| {
                line.split_once('=')
                    .is_none_or(|(name, _)| name.trim() != key)
            })
            .collect();
        lines.extend(values.iter().map(|value| format!("{key}={value}")));
        let mut text = lines.join("\n");
        text.push('\n');
        crate::live_bridge::write_keeping_file(&self.path, &text, false)
    }

    /// The language tag chosen to be remembered.
    pub(crate) fn language(&self) -> Option<String> {
        self.values(LANGUAGE).pop()
    }

    pub(crate) fn save_language(&self, tag: &str) -> io::Result<()> {
        self.replace(LANGUAGE, &[tag.to_owned()])
    }
}

/// How long a save waits for another window saving the settings.
const TURN_WAIT: Duration = Duration::from_secs(2);

/// Takes the settings lock through `lock`, which waits its turn, on a thread of
/// its own: the window saving waits at most `wait` for the window that holds
/// it, not without end behind one that hangs (review 2026-11-06 P3: `lock()`
/// on the window's thread). The lock held, or `None` on a file system that
/// cannot lock (NFS without lockd), which saves without it rather than never.
/// A lock taken after the save gave up is let go at once.
fn take_turn<T: Send + 'static>(
    lock: impl FnOnce() -> io::Result<T> + Send + 'static,
    wait: Duration,
) -> io::Result<Option<T>> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("settings lock".to_owned())
        .spawn(move || {
            ketchup_rejection::dropped(
                sender.send(lock()),
                ketchup_rejection::Reason::NobodyWaiting,
            );
        })?;
    match receiver.recv_timeout(wait) {
        Ok(Ok(held)) => Ok(Some(held)),
        Ok(Err(error)) if cannot_lock(&error) => Ok(None),
        Ok(Err(error)) => Err(error),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "another Kečup window has been saving the settings too long",
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other(
            "the thread taking the settings lock ended without it",
        )),
    }
}

/// The file system has no locks: `ENOLCK` from NFS without lockd, or none at all.
fn cannot_lock(error: &io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::ENOLCK) {
        return true;
    }
    error.kind() == io::ErrorKind::Unsupported
}

impl RecentFilesStore for SettingsFile {
    fn load(&self) -> Vec<PathBuf> {
        self.values(RECENT_FILE)
            .into_iter()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    fn save(&self, files: &[PathBuf]) -> io::Result<()> {
        let files: Vec<String> = files
            .iter()
            .filter_map(|file| file.to_str())
            .filter(|file| !file.contains(['\n', '\r']))
            .map(str::to_owned)
            .collect();
        self.replace(RECENT_FILE, &files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_file_keeps_the_language_and_the_list_between_windows() {
        let folder =
            std::env::temp_dir().join(format!("ketchup-app-settings-{}", std::process::id()));
        let path = folder.join(FILE_NAME);
        let first = SettingsFile::at(path.clone());
        assert_eq!(first.language(), None);
        assert!(first.load().is_empty());

        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&path, "future-setting=3\n").unwrap();
        let files = vec![
            PathBuf::from("/Users/eva/Domy/Mníchovice = 2.ketchup"),
            PathBuf::from(" /Volumes/NAS/a.ketchup "),
        ];
        first.save_language("sk").unwrap();
        first.save(&files).unwrap();
        let unstorable = PathBuf::from("/tmp/two\nlines.ketchup");
        let mut with_unstorable = files.clone();
        with_unstorable.insert(1, unstorable);
        first.save(&with_unstorable).unwrap();

        let second = SettingsFile::at(path.clone());
        assert_eq!(second.language().as_deref(), Some("sk"));
        assert_eq!(second.load(), files);
        assert_eq!(second.values("future-setting"), ["3"]);

        second.save(&[]).unwrap();
        second.save_language("cs").unwrap();
        let third = SettingsFile::at(path);
        assert!(third.load().is_empty());
        assert_eq!(third.language().as_deref(), Some("cs"));
        assert_eq!(third.values("future-setting"), ["3"]);
        assert_eq!(beside_lock(&folder), [FILE_NAME]);
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// Review 2026-11-04 r19 UI-162: one byte that is not UTF-8 made the file
    /// read as empty, so the next save wrote only its own key and the
    /// language, the list and a newer Kečup's lines were lost.
    #[test]
    fn a_damaged_settings_file_is_kept_aside_and_its_other_lines_carried_over() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        let damaged = b"language=sk\r\nfuture-setting=3\nrecent=/Domy/M\xe9chenice.ketchup\nrecent=/b.ketchup\n";
        std::fs::write(&path, damaged).unwrap();
        let settings = SettingsFile::at(path.clone());
        assert_eq!(settings.language().as_deref(), Some("sk"));
        assert_eq!(settings.load(), [PathBuf::from("/b.ketchup")]);

        settings.save(&[PathBuf::from("/c.ketchup")]).unwrap();
        assert_eq!(settings.language().as_deref(), Some("sk"));
        assert_eq!(settings.values("future-setting"), ["3"]);
        assert_eq!(settings.load(), [PathBuf::from("/c.ketchup")]);
        assert_eq!(
            std::fs::read(path.with_extension("txt.bad")).unwrap(),
            damaged,
            "the damaged file is kept whole"
        );
    }

    /// A settings file that cannot be read is not written over.
    #[test]
    fn a_settings_file_that_cannot_be_read_is_not_written_over() {
        let folder = tempfile::tempdir().unwrap();
        // A folder where the file should be cannot be read as one.
        let path = folder.path().join(FILE_NAME);
        std::fs::create_dir(&path).unwrap();
        let settings = SettingsFile::at(path.clone());
        assert!(settings.save_language("cs").is_err());
        assert!(path.is_dir());
        assert_eq!(
            beside_lock(folder.path()),
            [FILE_NAME],
            "nothing is left beside it"
        );
    }

    /// The names in `folder` other than the lock windows take turns on.
    fn beside_lock(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "settings.txt.lock")
            .collect();
        names.sort();
        names
    }

    /// Review 2026-11-04 r19 P3: two windows saving at once each read the
    /// file, changed their key and wrote it back, so one window's change was
    /// lost; the second also renamed the first one's `.part` file away.
    #[test]
    fn windows_saving_at_once_keep_each_others_settings() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        let windows: Vec<_> = (0..8)
            .map(|window| {
                let settings = SettingsFile::at(path.clone());
                std::thread::spawn(move || {
                    for round in 0..25 {
                        settings
                            .replace(&format!("window{window}"), &[round.to_string()])
                            .unwrap();
                    }
                })
            })
            .collect();
        for window in windows {
            window.join().unwrap();
        }
        let settings = SettingsFile::at(path);
        for window in 0..8 {
            assert_eq!(settings.values(&format!("window{window}")), ["24"]);
        }
        let left: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "no part file is left: {left:?}");
    }

    /// Review 2026-11-04 r19 P3: the settings (recent documents) were left
    /// readable by everyone, and a symlinked settings file became a plain one.
    #[cfg(unix)]
    #[test]
    fn the_settings_file_is_private_and_a_symlink_keeps_pointing_at_it() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        SettingsFile::at(path.clone()).save_language("sk").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "only its owner reads it: {mode:o}");

        let dotfiles = folder.path().join("dotfiles.txt");
        std::fs::rename(&path, &dotfiles).unwrap();
        std::os::unix::fs::symlink(&dotfiles, &path).unwrap();
        SettingsFile::at(path.clone()).save_language("cs").unwrap();
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(
            std::fs::read_to_string(&dotfiles)
                .unwrap()
                .contains("language=cs")
        );
    }

    /// Review 2026-11-06 P3: `lock()` held the window without end behind a
    /// window that hung while saving, and a file system without locks (NFS
    /// without lockd) never let it save.
    #[test]
    fn a_save_waits_for_another_window_a_while_and_saves_without_locks() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        let held = std::fs::File::create(path.with_extension("txt.lock")).unwrap();
        held.lock().unwrap();
        let error = SettingsFile::at(path.clone())
            .save_language("sk")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(!path.exists(), "nothing is written without the turn");
        held.unlock().unwrap();
        // The lock the given-up save took once it was let go is let go too.
        SettingsFile::at(path.clone()).save_language("sk").unwrap();

        let refused = |kind| move || Err::<(), _>(io::Error::from(kind));
        assert!(matches!(
            take_turn(refused(io::ErrorKind::Unsupported), TURN_WAIT),
            Ok(None)
        ));
        #[cfg(unix)]
        assert!(matches!(
            take_turn(
                || Err::<(), _>(io::Error::from_raw_os_error(libc::ENOLCK)),
                TURN_WAIT
            ),
            Ok(None)
        ));
        assert!(take_turn(refused(io::ErrorKind::PermissionDenied), TURN_WAIT).is_err());
    }

    #[test]
    fn the_user_folder_is_the_platforms_settings_folder() {
        let Some(file) = SettingsFile::of_user() else {
            return;
        };
        let folder = file.path.parent().unwrap();
        assert!(folder.ends_with("Ketchup"));
        if cfg!(target_os = "macos") {
            assert!(folder.ends_with("Library/Application Support/Ketchup"));
        }
    }
}
