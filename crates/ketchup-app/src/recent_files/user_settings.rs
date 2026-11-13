//! The user settings of Kečup where there is no Windows registry: the
//! language choice and File > Open Recent, in
//! `~/Library/Application Support/Ketchup/settings.txt` on macOS and in
//! `$XDG_CONFIG_HOME/Ketchup/settings.txt` or `~/.config/Ketchup/settings.txt`
//! elsewhere, the folder the Viewer keeps its settings in. Files of that form
//! beside it keep other things of the user's, the drawing files each document
//! wrote (`drawings.txt`), on Windows in `%LOCALAPPDATA%\Ketchup`.
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
#[cfg_attr(windows, allow(dead_code))]
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
        Self::of_user_named(FILE_NAME)
    }

    /// The file `name` of the user who started this window in the folder of
    /// these settings; on Windows, which keeps them in the registry,
    /// `%LOCALAPPDATA%\Ketchup`.
    pub(crate) fn of_user_named(name: &str) -> Option<Self> {
        let var = |name| std::env::var_os(name).map(PathBuf::from);
        let base = if cfg!(windows) {
            var("LOCALAPPDATA")
        } else if cfg!(target_os = "macos") {
            var("HOME").map(|home| home.join("Library").join("Application Support"))
        } else {
            var("XDG_CONFIG_HOME")
                .filter(|base| base.is_absolute())
                .or_else(|| var("HOME").map(|home| home.join(".config")))
        };
        base.filter(|base| base.is_absolute())
            .map(|base| Self::at(base.join("Ketchup").join(name)))
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

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
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
        self.update(key, |_| values.to_vec())
    }

    /// As [`Self::replace`], the values of `key` made by `values` from those
    /// in the file while it waits for no other window, so a value another
    /// window saved meanwhile is not lost.
    pub(crate) fn update(
        &self,
        key: &str,
        values: impl FnOnce(Vec<String>) -> Vec<String>,
    ) -> io::Result<()> {
        self.rewrite(|lines| {
            let (mut lines, of_key): (Vec<String>, Vec<String>) =
                lines.into_iter().partition(|line| !of(line, key));
            let before = of_key
                .into_iter()
                .filter_map(|line| line.split_once('=').map(|(_, value)| value.to_owned()))
                .collect();
            lines.extend(values(before).iter().map(|value| format!("{key}={value}")));
            lines
        })
    }

    /// As [`Self::replace`], each key of `keyed` given its value at one
    /// turn: the BTLx options waited for the lock three times on the window
    /// (review 2026-11-07 r26 P3).
    pub(crate) fn replace_each(&self, keyed: &[(&str, String)]) -> io::Result<()> {
        self.rewrite(|lines| {
            let mut lines: Vec<String> = lines
                .into_iter()
                .filter(|line| !keyed.iter().any(|(key, _)| of(line, key)))
                .collect();
            lines.extend(keyed.iter().map(|(key, value)| format!("{key}={value}")));
            lines
        })
    }

    /// The lines of the file made by `change` from those in it, while it
    /// waits for no other window. Under root all of it, the folder and the
    /// files beside the settings too, is done as the user they belong to
    /// (review 2026-11-12 R1112-SEC-500).
    fn rewrite(&self, change: impl FnOnce(Vec<String>) -> Vec<String>) -> io::Result<()> {
        crate::live_bridge::as_configuration_user(|| self.rewrite_as_found(change))
    }

    /// [`Self::rewrite`] as this thread is.
    fn rewrite_as_found(&self, change: impl FnOnce(Vec<String>) -> Vec<String>) -> io::Result<()> {
        let folder = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(folder)?;
        let lock = side_file(&self.path.with_extension("txt.lock"), false)?;
        take_turn(|| lock.try_lock(), || seen(&self.path), TURN_WAIT, TURN_CAP)?;
        let turns = turns_of(&self.path);
        // Told to the windows waiting: on FAT, HFS+ or SMB a save of the same
        // length within the second or two of the time kept changes nothing
        // they see of the settings (review 2026-11-07 P3). It only tells them
        // to wait: one not written does not hold the save (r26 P3).
        let turn = std::fs::read_to_string(&turns)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let counted = side_file(&turns, true).and_then(|mut file| {
            io::Write::write_all(&mut file, turn.wrapping_add(1).to_string().as_bytes())
        });
        if let Err(error) = counted {
            ketchup_rejection::log!("the settings are saved without their turn counted: {error}");
        }
        let (lines, damaged) = self.read()?;
        if damaged {
            let whole = std::fs::read(&self.path)?;
            io::Write::write_all(
                &mut side_file(&self.path.with_extension("txt.bad"), true)?,
                &whole,
            )?;
        }
        let lines = change(lines);
        let mut text = lines.join("\n");
        if !lines.is_empty() {
            text.push('\n');
        }
        crate::live_bridge::write_keeping_file(&self.path, &text, false)
    }

    /// The language tag chosen to be remembered.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn language(&self) -> Option<String> {
        self.values(LANGUAGE).pop()
    }

    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn save_language(&self, tag: &str) -> io::Result<()> {
        self.replace(LANGUAGE, &[tag.to_owned()])
    }
}

/// Opens a file beside the settings (the lock, the turn, the damaged copy) to
/// write, made its owner's only, and not through a symlink in its place (review
/// 2026-11-12 R1112-SEC-500: one there was followed wherever it led).
fn side_file(path: &Path, truncate: bool) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// How long a save waits for another window saving the settings.
const TURN_WAIT: Duration = Duration::from_secs(2);
/// How long a save waits for other windows in all, however they go on saving:
/// the window's thread is not held longer (review 2026-11-07 P3).
const TURN_CAP: Duration = Duration::from_secs(10);
/// How often a waiting save tries the lock again.
const TURN_POLL: Duration = Duration::from_millis(2);

/// Takes the settings lock through `attempt`, which does not wait: the window
/// saving tries it while other windows save, `saved` telling when one did, but
/// gives up once one held it `wait` without saving, not waiting without end
/// behind one that hangs (review 2026-11-06 P3: `lock()` on the window's
/// thread), and after `cap` in all behind windows that go on saving (review
/// 2026-11-07 P3). On a file system that cannot lock (NFS without lockd) it saves
/// without the lock rather than never. A save that gave up leaves nothing
/// behind (review 2026-11-06 r23 P3: a thread waited on in `lock()` with the
/// file open as long as the other window hung).
fn take_turn<T: PartialEq>(
    attempt: impl FnMut() -> Result<(), std::fs::TryLockError>,
    saved: impl FnMut() -> T,
    wait: Duration,
    cap: Duration,
) -> io::Result<()> {
    take_turn_by(attempt, saved, wait, cap, std::time::Instant::now)
}

/// [`take_turn`] with the time read from `now`, so a test of how long it waits
/// does not hang on how busy the machine is.
fn take_turn_by<T: PartialEq>(
    mut attempt: impl FnMut() -> Result<(), std::fs::TryLockError>,
    mut saved: impl FnMut() -> T,
    wait: Duration,
    cap: Duration,
    mut now: impl FnMut() -> std::time::Instant,
) -> io::Result<()> {
    let mut seen = saved();
    let start = now();
    let mut until = start + wait;
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::Error(error)) if cannot_lock(&error) => return Ok(()),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
            Err(std::fs::TryLockError::WouldBlock) if now() < until.min(start + cap) => {
                std::thread::sleep(TURN_POLL);
                let latest = saved();
                if latest != seen {
                    seen = latest;
                    until = now() + wait;
                }
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "another Kečup window has been saving the settings too long",
                ));
            }
        }
    }
}

/// Whether `line` is a value of `key`.
fn of(line: &str, key: &str) -> bool {
    line.split_once('=')
        .is_some_and(|(name, _)| name.trim() == key)
}

/// The file beside the settings at `path` that counts the turns taken.
fn turns_of(path: &Path) -> PathBuf {
    path.with_extension("txt.turn")
}

/// What a window waiting sees of the saves of others: when and how long the
/// settings were last written, and the count of the turns taken.
fn seen(path: &Path) -> (Option<(std::time::SystemTime, u64)>, Option<Vec<u8>>) {
    (saved_at(path), std::fs::read(turns_of(path)).ok())
}

/// When and how long the settings were last written; the same nothing while
/// they cannot be read.
fn saved_at(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    std::fs::metadata(path)
        .and_then(|metadata| Ok((metadata.modified()?, metadata.len())))
        .ok()
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

    /// Review 2026-11-12 R1112-SEC-500: a symlink in place of the turn, the
    /// damaged copy or the lock beside the settings was followed, so a window
    /// run as root wrote the settings' bytes into whatever file it led to.
    #[cfg(unix)]
    #[test]
    fn a_symlink_beside_the_settings_is_not_written_through() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        std::fs::write(&path, b"language=sk\nrecent=/M\xe9chenice.ketchup\n").unwrap();
        let victim = |name: &str| {
            let file = folder.path().join(name);
            std::fs::write(&file, "kept").unwrap();
            file
        };
        let (turn_victim, bad_victim) = (victim("turn victim"), victim("bad victim"));
        std::os::unix::fs::symlink(&turn_victim, turns_of(&path)).unwrap();
        std::os::unix::fs::symlink(&bad_victim, path.with_extension("txt.bad")).unwrap();
        let settings = SettingsFile::at(path.clone());
        settings.save(&[PathBuf::from("/c.ketchup")]).unwrap_err();
        assert_eq!(std::fs::read_to_string(&turn_victim).unwrap(), "kept");
        assert_eq!(std::fs::read_to_string(&bad_victim).unwrap(), "kept");

        // Nor the lock, which the save is then refused.
        std::fs::remove_file(path.with_extension("txt.bad")).unwrap();
        let lock_victim = victim("lock victim");
        std::fs::remove_file(&lock_victim).unwrap();
        std::fs::remove_file(path.with_extension("txt.lock")).unwrap();
        std::os::unix::fs::symlink(&lock_victim, path.with_extension("txt.lock")).unwrap();
        settings.save(&[PathBuf::from("/c.ketchup")]).unwrap_err();
        assert!(!lock_victim.exists());
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

    /// The names in `folder` other than the lock windows take turns on and
    /// the count of the turns taken.
    fn beside_lock(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "settings.txt.lock" && name != "settings.txt.turn")
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
        // Review 2026-11-06 r23 P3: the given-up save left a thread waiting in
        // `lock()` with the file open; nothing of it takes the lock now.
        let other = std::fs::File::create(path.with_extension("txt.lock")).unwrap();
        other.try_lock().unwrap();
        other.unlock().unwrap();
        SettingsFile::at(path.clone()).save_language("sk").unwrap();

        let tries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&tries);
        let busy = move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(std::fs::TryLockError::WouldBlock)
        };
        let error = take_turn(busy, || (), Duration::from_millis(60), TURN_CAP).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            tries.load(std::sync::atomic::Ordering::Relaxed) > 1,
            "it tried again"
        );
        assert_eq!(std::sync::Arc::strong_count(&tries), 1, "nothing tries on");
        // Windows that go on saving are waited for, however long they queue:
        // 40 turns 40 ms apart, far past the wait of 30 ms.
        let mut queued = 0;
        let mut saves = 0;
        take_turn_by(
            || {
                queued += 1;
                if queued < 40 {
                    Err(std::fs::TryLockError::WouldBlock)
                } else {
                    Ok(())
                }
            },
            || {
                saves += 1;
                saves
            },
            Duration::from_millis(30),
            TURN_CAP,
            ticking(),
        )
        .unwrap();
        // Review 2026-11-07 P3: but not past the cap, however they go on.
        let mut saves = 0;
        let error = take_turn(
            || Err(std::fs::TryLockError::WouldBlock),
            || {
                saves += 1;
                saves
            },
            Duration::from_millis(30),
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);

        let refused = |kind| move || Err(std::fs::TryLockError::Error(io::Error::from(kind)));
        take_turn(
            refused(io::ErrorKind::Unsupported),
            || (),
            TURN_WAIT,
            TURN_CAP,
        )
        .unwrap();
        #[cfg(unix)]
        take_turn(
            || {
                Err(std::fs::TryLockError::Error(io::Error::from_raw_os_error(
                    libc::ENOLCK,
                )))
            },
            || (),
            TURN_WAIT,
            TURN_CAP,
        )
        .unwrap();
        assert!(
            take_turn(
                refused(io::ErrorKind::PermissionDenied),
                || (),
                TURN_WAIT,
                TURN_CAP
            )
            .is_err()
        );
    }

    /// Review 2026-11-07 P3: a window waited for others only while the time
    /// or length of the settings changed, which on FAT, HFS+ or SMB stay the
    /// same over saves of a second or two. Each turn taken is counted beside
    /// the lock, and a window waits while the count goes on.
    #[test]
    fn a_save_waits_while_other_windows_take_turns_the_settings_do_not_show() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        let settings = SettingsFile::at(path.clone());
        settings.save_language("sk").unwrap();
        let turns = turns_of(&path);
        let count = || std::fs::read_to_string(&turns).unwrap();
        let first = count();
        settings.save_language("sk").unwrap();
        assert_ne!(count(), first, "a turn taken is counted");

        // Other windows take 60 turns, 40 ms apart, far past the wait of 30 ms,
        // the settings as they were. The clock is the test's, so a busy
        // machine does not make one turn outlast the wait.
        let mut queued = 0;
        take_turn_by(
            || {
                queued += 1;
                std::fs::write(&turns, format!("other {queued}")).unwrap();
                if queued < 60 {
                    Err(std::fs::TryLockError::WouldBlock)
                } else {
                    Ok(())
                }
            },
            || seen(&path),
            Duration::from_millis(30),
            TURN_CAP,
            ticking(),
        )
        .unwrap();
    }

    /// A clock that moves on 20 ms each time it is read: a turn that reads it
    /// twice is 40 ms.
    fn ticking() -> impl FnMut() -> std::time::Instant {
        let mut at = std::time::Instant::now();
        move || {
            at += Duration::from_millis(20);
            at
        }
    }

    /// Review 2026-11-07 r26 P3: the turn counted only tells other windows to
    /// wait, yet one not written failed the whole save.
    #[test]
    fn a_turn_not_counted_does_not_hold_the_save() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE_NAME);
        std::fs::create_dir(turns_of(&path)).unwrap();
        let settings = SettingsFile::at(path);
        settings.save_language("sk").unwrap();
        assert_eq!(settings.language().as_deref(), Some("sk"));
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
