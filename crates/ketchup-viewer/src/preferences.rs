//! The customer's Viewer settings, kept between starts: the interface
//! language and projection chosen in the Settings menu. The Viewer only holds them; each
//! host stores the text of [`Preferences::to_text`] where it can (a file on the
//! desktop, `localStorage` in a browser, private storage on a phone).
//!
//! The text is `key = value` lines; unknown keys and values are skipped when
//! read and, in the desktop file, kept when saved, so settings written by a
//! newer Viewer still open in an older one and survive it.

use crate::text::Language;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_user_files::{UserFolder, new_side_file, side_file};
use ketchup_view_format::FittedProjection;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Preferences {
    /// `None` until the customer picks one: the system language applies.
    pub language: Option<Language>,
    /// The projection every opened package starts in; `None` follows the
    /// package: its scenes keep their own and a fitted view takes the author's.
    pub projection: Option<FittedProjection>,
}

/// `perspective` / `orthographic`, as the package manifest spells them.
fn projection_name(projection: FittedProjection) -> &'static str {
    match projection {
        FittedProjection::Perspective => "perspective",
        FittedProjection::Orthographic => "orthographic",
    }
}

impl Preferences {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut preferences = Self::default();
        for (key, value) in text
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once('='))
        {
            match key.trim() {
                "language" => preferences.language = Language::from_tag(value.trim()),
                "projection" => {
                    preferences.projection = [
                        FittedProjection::Perspective,
                        FittedProjection::Orthographic,
                    ]
                    .into_iter()
                    .find(|&projection| projection_name(projection) == value.trim());
                }
                _ => {}
            }
        }
        preferences
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\n");
        if let Some(language) = self.language {
            text.push_str(&format!("language = {}\n", language.tag()));
        }
        if let Some(projection) = self.projection {
            text.push_str(&format!("projection = {}\n", projection_name(projection)));
        }
        text
    }

    /// The settings in `path`; none yet means the defaults. A line that is not
    /// UTF-8 is skipped, not the whole file; a file that cannot be read is
    /// logged and the defaults apply, and [`Self::save`] will not write over it.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        match read_lines(path) {
            Ok((lines, _)) => Self::parse(&lines.join("\n")),
            Err(error) => {
                ketchup_rejection::log!(
                    "The Viewer settings {} could not be read; the defaults apply: {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write to `path` through a part file of this Viewer's own, so a cut-off
    /// write keeps the old settings. The lines of the file this Viewer does not know are kept, so a
    /// newer Viewer's settings survive; a file with a line that is not UTF-8 is
    /// first kept whole as `<path>.bad`, and one that cannot be read is not
    /// written over. Another Viewer saving meanwhile waits on `<path>.lock`
    /// (review 2026-11-04 r19 UI-162), two seconds at most; a file system
    /// without locks saves without it.
    ///
    /// No file beside the settings is opened through a link in its place
    /// (review 2026-11-13 R1113-SEC-600: under `sudo` root followed them).
    pub fn save(&self, path: &Path) -> Result<(), Rejection> {
        let failed = |error| save_failed(path, error);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(failed)?;
        }
        let lock = side_file(&path.with_extension("txt.lock"), false).map_err(failed)?;
        take_turn(|| lock.try_lock(), || saved_at(path), TURN_WAIT).map_err(failed)?;
        let (lines, damaged) = read_lines(path).map_err(|error| {
            Rejection::new("viewer.settings.read", RejectionPhase::Io)
                .target(path.display().to_string())
                .reason(
                    "The Viewer settings file could not be read, so it was not written over; \
                     the setting applies until the Viewer closes.",
                )
                .fix_hint("Make the settings file readable, then choose the setting again.")
                .caused_by(error)
        })?;
        if damaged {
            std::fs::read(path)
                .and_then(|whole| {
                    std::io::Write::write_all(
                        &mut side_file(&path.with_extension("txt.bad"), true)?,
                        &whole,
                    )
                })
                .map_err(failed)?;
        }
        let mut text = self.to_text();
        for line in lines.iter().filter(|line| !written_by_to_text(line)) {
            text.push_str(line);
            text.push('\n');
        }
        // A settings file that is a link (dotfiles kept in a repository) is
        // written where it points, so the link stays (review 2026-11-08 VW-250).
        let target = link_target(path).map_err(failed)?;
        // Root, writing as itself, follows only a link to where no other user
        // could change the way (review 2026-11-13 r95 R1113r95-SEC-700: a raced
        // link had root rename the user's lines over a file of root's).
        #[cfg(unix)]
        if target != path
            && ketchup_user_files::effective_user() == 0
            && ketchup_user_files::holder_of_way(&target)
                .map_err(failed)?
                .0
                != 0
        {
            return Err(failed(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the settings are a link to where another user could change the way",
            )));
        }
        // This Viewer's own part file: another one saving without a lock does not
        // rename it away (review 2026-11-06 P3: one fixed `.part`).
        let part = target.with_extension(format!(
            "{}-{}.part",
            std::process::id(),
            PARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        // On disk before the rename, and the rename once it is done, so a power
        // loss keeps the old settings or the new, not an empty file (VW-250).
        let written = new_side_file(&part)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, text.as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&part, &target))
            .and_then(|()| {
                crate::app::session::sync_directory(target.parent().unwrap_or(Path::new(".")))
            });
        if written.is_err() {
            ketchup_rejection::dropped(
                std::fs::remove_file(&part),
                ketchup_rejection::Reason::ReportedElsewhere,
            );
        }
        written.map_err(failed)
    }

    /// [`Self::save`] of the desktop settings in `folder`: under `sudo` as the
    /// user the folder is (review 2026-11-13 R1113-SEC-600: root made, locked and
    /// renamed the files there as root, through the user's links).
    pub fn save_in(&self, folder: &UserFolder) -> Result<(), Rejection> {
        let path = folder.path().join(DESKTOP_FILE);
        folder
            .as_owner(|| Ok::<_, std::io::Error>(self.save(&path)))
            .unwrap_or_else(|error| Err(save_failed(&path, error)))
    }
}

/// That the settings could not be saved in `path`, for `error`.
fn save_failed(path: &Path, error: std::io::Error) -> Rejection {
    Rejection::new("viewer.settings.save", RejectionPhase::Io)
        .target(path.display().to_string())
        .reason("The Viewer settings could not be saved; they apply until the Viewer closes.")
        .fix_hint("Make the folder of the settings file writable, then choose the setting again.")
        .caused_by(error)
}

/// The file `path` names after its links, a link that points nowhere yet
/// included; `path` itself when it is no link.
fn link_target(path: &Path) -> std::io::Result<PathBuf> {
    let mut target = path.to_owned();
    // As many links as Linux follows before ELOOP.
    for _ in 0..40 {
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let link = std::fs::read_link(&target)?;
                target = target.parent().unwrap_or(Path::new(".")).join(link);
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => return Ok(target),
        }
    }
    Err(std::io::Error::other(
        "the settings file is a loop of links",
    ))
}

/// The part files this Viewer has written, each named by its number.
static PARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How long a save waits for another Viewer saving the settings.
const TURN_WAIT: Duration = Duration::from_secs(2);
/// How often a waiting save tries the lock again.
const TURN_POLL: Duration = Duration::from_millis(2);

/// Takes the settings lock through `attempt`, which does not wait: the save
/// tries it while other Viewers save, `saved` telling when one did, but gives
/// up once one held it `wait` without saving, not waiting without end behind
/// one that hangs (review 2026-11-06 P3). On a file system that cannot lock
/// (NFS without lockd) it saves without the lock rather than never. A save that
/// gave up leaves nothing behind (review 2026-11-06 r23 P3: a thread waited on
/// in `lock()` with the file open as long as the other Viewer hung).
fn take_turn<T: PartialEq>(
    mut attempt: impl FnMut() -> Result<(), std::fs::TryLockError>,
    mut saved: impl FnMut() -> T,
    wait: Duration,
) -> std::io::Result<()> {
    let mut seen = saved();
    let mut until = std::time::Instant::now() + wait;
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::Error(error)) if cannot_lock(&error) => return Ok(()),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                std::thread::sleep(TURN_POLL);
                let now = saved();
                if now != seen {
                    seen = now;
                    until = std::time::Instant::now() + wait;
                }
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "another Viewer has been saving the settings too long",
                ));
            }
        }
    }
}

/// When and how long the settings were last written; the same nothing while
/// they cannot be read.
fn saved_at(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    std::fs::metadata(path)
        .and_then(|metadata| Ok((metadata.modified()?, metadata.len())))
        .ok()
}

/// The file system has no locks: `ENOLCK` from NFS without lockd, or none at all.
fn cannot_lock(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(libc::ENOLCK) {
        return true;
    }
    error.kind() == std::io::ErrorKind::Unsupported
}

const HEADER: &str = "# Kečup Viewer settings";

/// The header and the keys [`Preferences::to_text`] writes itself.
fn written_by_to_text(line: &str) -> bool {
    line.trim() == HEADER
        || line
            .split_once('=')
            .is_some_and(|(key, _)| matches!(key.trim(), "language" | "projection"))
}

/// The lines of the file, none when there is no file. A line that is not
/// UTF-8 is left out, and `true` says the file had one. Any other failure to
/// read is an error, not an empty file: writing over it would lose the
/// language and a newer Viewer's lines (review 2026-11-04 r19 UI-162).
fn read_lines(path: &Path) -> std::io::Result<(Vec<String>, bool)> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), false));
        }
        Err(error) => return Err(error),
    };
    let mut damaged = false;
    let lines = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let text = std::str::from_utf8(line).ok();
            damaged |= text.is_none();
            text.map(str::to_owned)
        })
        .collect();
    Ok((lines, damaged))
}

/// The desktop Viewer's settings file in [`desktop_settings_folder`].
pub const DESKTOP_FILE: &str = "viewer-settings.txt";

/// Where the desktop Viewer keeps its settings: `%APPDATA%\Ketchup` on Windows,
/// `~/Library/Application Support/Ketchup` on macOS, else `$XDG_CONFIG_HOME/Ketchup`
/// or `~/.config/Ketchup`.
#[must_use]
pub fn desktop_settings_folder() -> Option<UserFolder> {
    UserFolder::settings("APPDATA", "Ketchup")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_save_and_skip_what_they_do_not_know() {
        let folder =
            std::env::temp_dir().join(format!("ketchup-viewer-settings-{}", std::process::id()));
        let path = folder.join("viewer-settings.txt");
        assert_eq!(Preferences::load(&path), Preferences::default());
        let slovak = Preferences {
            language: Language::from_tag("sk"),
            projection: Some(FittedProjection::Orthographic),
        };
        slovak.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), slovak);
        assert!(parts_in(&folder).is_empty());
        let newer =
            "language = sk-SK\nprojection = orthographic\nfuture-setting = 3\nno equals sign\n";
        assert_eq!(Preferences::parse(newer), slovak);
        assert_eq!(Preferences::parse("projection = fisheye").projection, None);
        assert_eq!(
            Preferences::parse("language = xx-YY"),
            Preferences::default()
        );
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// Review 2026-11-04 r19 UI-162: one byte that is not UTF-8 made the file
    /// read as the defaults, and the next save wrote only the two known keys,
    /// so the language and a newer Viewer's lines were lost.
    #[test]
    fn a_damaged_settings_file_is_kept_aside_and_its_other_lines_carried_over() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("viewer-settings.txt");
        let damaged = b"language = sk\r\nfuture-setting = 3\nnote = M\xe9\n";
        std::fs::write(&path, damaged).unwrap();
        let mut preferences = Preferences::load(&path);
        assert_eq!(preferences.language, Language::from_tag("sk"));

        preferences.projection = Some(FittedProjection::Orthographic);
        preferences.save(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(Preferences::parse(&saved), preferences);
        assert!(saved.contains("future-setting = 3"), "{saved}");
        assert_eq!(
            std::fs::read(path.with_extension("txt.bad")).unwrap(),
            damaged,
            "the damaged file is kept whole"
        );
    }

    /// A newer Viewer's lines survive a setting changed in an older one.
    #[test]
    fn a_save_keeps_the_lines_it_does_not_know() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("viewer-settings.txt");
        std::fs::write(
            &path,
            "language = sk\nfuture-setting = 3\nprojection = perspective\n",
        )
        .unwrap();
        let mut preferences = Preferences::load(&path);
        preferences.projection = Some(FittedProjection::Orthographic);
        preferences.save(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(Preferences::parse(&saved), preferences);
        assert!(saved.contains("future-setting = 3"), "{saved}");
        assert!(!saved.contains("perspective"), "{saved}");
        assert_eq!(
            saved.matches("# Kečup Viewer settings").count(),
            1,
            "{saved}"
        );
    }

    /// A settings file that cannot be read is not written over.
    #[test]
    fn a_settings_file_that_cannot_be_read_is_not_written_over() {
        let folder = tempfile::tempdir().unwrap();
        // A folder where the file should be cannot be read as one.
        let path = folder.path().join("viewer-settings.txt");
        std::fs::create_dir(&path).unwrap();
        let preferences = Preferences {
            language: Language::from_tag("cs"),
            projection: None,
        };
        assert!(preferences.save(&path).is_err());
        assert!(path.is_dir());
        assert!(parts_in(folder.path()).is_empty());
    }

    /// Review 2026-11-08 VW-250: a save renamed its part file over a settings
    /// file kept as a link, so the link became a plain file and the settings
    /// it pointed to went stale.
    #[cfg(unix)]
    #[test]
    fn a_settings_file_kept_as_a_link_is_written_where_it_points() {
        let folder = tempfile::tempdir().unwrap();
        let dotfiles = folder.path().join("dotfiles");
        std::fs::create_dir(&dotfiles).unwrap();
        let kept = dotfiles.join("viewer-settings.txt");
        std::fs::write(&kept, "language = cs\nfuture-setting = 3\n").unwrap();
        let path = folder.path().join("viewer-settings.txt");
        std::os::unix::fs::symlink("dotfiles/viewer-settings.txt", &path).unwrap();
        let slovak = Preferences {
            language: Language::from_tag("sk"),
            projection: None,
        };
        slovak.save(&path).unwrap();
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        let saved = std::fs::read_to_string(&kept).unwrap();
        assert_eq!(Preferences::parse(&saved), slovak);
        assert!(saved.contains("future-setting = 3"), "{saved}");
        assert!(parts_in(folder.path()).is_empty() && parts_in(&dotfiles).is_empty());

        // A link to a file not written yet stays a link too.
        let fresh = folder.path().join("fresh.txt");
        std::os::unix::fs::symlink(dotfiles.join("fresh.txt"), &fresh).unwrap();
        slovak.save(&fresh).unwrap();
        assert!(fresh.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(Preferences::load(&dotfiles.join("fresh.txt")), slovak);

        let looped = folder.path().join("looped.txt");
        std::os::unix::fs::symlink("looped.txt", &looped).unwrap();
        assert!(link_target(&looped).is_err());
    }

    /// Review 2026-11-13 R1113-SEC-600: the damaged copy was copied and the lock
    /// opened through a link in their place, so a link to a file of root's had it
    /// written over or made; under `sudo` all of it was root's, through every link
    /// of the user's in their settings folder.
    #[cfg(unix)]
    #[test]
    fn no_file_beside_the_settings_is_written_through_a_link() {
        let folder = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        let secret = roots.path().join("authorized_keys");
        std::fs::write(&secret, "root's\n").unwrap();
        let path = folder.path().join("viewer-settings.txt");
        std::fs::write(&path, b"language = sk\nnote = M\xe9\n").unwrap();
        let czech = Preferences {
            language: Language::from_tag("cs"),
            projection: None,
        };
        let bad = path.with_extension("txt.bad");
        std::os::unix::fs::symlink(&secret, &bad).unwrap();
        assert!(czech.save(&path).is_err());
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "root's\n");
        std::fs::remove_file(&bad).unwrap();
        let lock = path.with_extension("txt.lock");
        std::fs::remove_file(&lock).unwrap();
        let made = roots.path().join("made");
        std::os::unix::fs::symlink(&made, &lock).unwrap();
        assert!(czech.save(&path).is_err());
        assert!(!made.exists());
    }

    /// The home of the user [`under_sudo_the_settings_are_the_users_and_a_link_leads_them_nowhere`]
    /// runs in, set by the run of it that started this one.
    #[cfg(unix)]
    const USERS_HOME: &str = "KETCHUP_TEST_USERS_HOME";

    /// Review 2026-11-13 r95 P3: under `sudo` `HOME` is `/root`, so the settings
    /// folder in the user's home was refused as another user's than `HOME`'s before
    /// anything was written as the user, and the test of the user's link passed for
    /// that. Root runs this test again with `HOME` the user's, as `sudo -E` keeps
    /// it: their own folder gets their settings, and their link in place of it to
    /// a folder of root's leads them nowhere.
    #[cfg(unix)]
    #[test]
    fn under_sudo_the_settings_are_the_users_and_a_link_leads_them_nowhere() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if ketchup_user_files::effective_user() != 0 {
            return;
        }
        let user = (65534, 65534);
        let Some(home) = std::env::var_os(USERS_HOME).map(std::path::PathBuf::from) else {
            let home = tempfile::tempdir().unwrap();
            std::os::unix::fs::chown(home.path(), Some(user.0), Some(user.1)).unwrap();
            let name = "preferences::tests::under_sudo_the_settings_are_the_users_and_a_link_leads_them_nowhere";
            let run = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--test-threads", "1"])
                .env("HOME", home.path())
                .env(USERS_HOME, home.path())
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&run.stdout);
            assert!(
                run.status.success(),
                "{said}{}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert!(said.contains("test result: ok. 1 passed"), "{said}");
            return;
        };
        let czech = Preferences {
            language: Language::from_tag("cs"),
            projection: None,
        };
        let roots = tempfile::tempdir().unwrap();
        std::fs::set_permissions(roots.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let linked = home.join("Linked");
        std::os::unix::fs::symlink(roots.path(), &linked).unwrap();
        std::os::unix::fs::lchown(&linked, Some(user.0), Some(user.1)).unwrap();
        // Refused by the system, as the user, not before as another user's home.
        let refused = czech
            .save_in(&UserFolder::at(home.clone(), linked))
            .unwrap_err();
        assert!(
            format!("{refused:?}").contains("ermission denied"),
            "{refused:?}"
        );
        assert!(!roots.path().join(DESKTOP_FILE).exists());
        assert!(parts_in(roots.path()).is_empty());
        let own = home.join("Ketchup");
        std::fs::create_dir(&own).unwrap();
        std::os::unix::fs::chown(&own, Some(user.0), Some(user.1)).unwrap();
        czech
            .save_in(&UserFolder::at(home.clone(), own.clone()))
            .unwrap();
        let found = std::fs::metadata(own.join(DESKTOP_FILE)).unwrap();
        assert_eq!((found.uid(), found.gid()), user);
    }

    /// Review 2026-11-13 r95 R1113r95-SEC-700: root, saving its own settings that
    /// are a link, renamed them over wherever the link led, a raced one too. It
    /// follows a link only where no other user could change the way.
    #[cfg(unix)]
    #[test]
    fn root_follows_a_settings_link_only_where_no_other_user_could_change_the_way() {
        use std::os::unix::fs::PermissionsExt;
        if ketchup_user_files::effective_user() != 0 {
            return;
        }
        let user = (65534, 65534);
        let roots = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::set_permissions(roots.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let theirs = tempfile::tempdir_in("/tmp").unwrap();
        std::os::unix::fs::chown(theirs.path(), Some(user.0), Some(user.1)).unwrap();
        let profile = theirs.path().join("profile.sh");
        std::fs::write(&profile, "theirs\n").unwrap();
        let path = roots.path().join("viewer-settings.txt");
        std::os::unix::fs::symlink(&profile, &path).unwrap();
        let czech = Preferences {
            language: Language::from_tag("cs"),
            projection: None,
        };
        let refused = czech.save(&path).unwrap_err();
        assert!(
            format!("{refused:?}").contains("another user"),
            "{refused:?}"
        );
        assert_eq!(std::fs::read_to_string(&profile).unwrap(), "theirs\n");
        assert!(parts_in(theirs.path()).is_empty());
        // A link of root's to a file of root's is written where it points.
        let kept = roots.path().join("dotfiles");
        std::fs::create_dir(&kept).unwrap();
        let own = kept.join("viewer-settings.txt");
        std::fs::write(&own, "language = sk\n").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&own, &path).unwrap();
        let ortho = Preferences {
            projection: Some(FittedProjection::Orthographic),
            ..czech
        };
        ortho.save(&path).unwrap();
        assert!(
            std::fs::read_to_string(&own)
                .unwrap()
                .contains("orthographic")
        );
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    /// The part files left in `folder`.
    fn parts_in(folder: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().ends_with(".part"))
            .collect()
    }

    /// Review 2026-11-06 P3: a save waited without end behind a Viewer that hung
    /// while saving, a file system without locks never let it save, and two
    /// Viewers wrote one fixed `.part`.
    #[test]
    fn a_save_waits_a_while_saves_without_locks_and_writes_its_own_part() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("viewer-settings.txt");
        let held = std::fs::File::create(path.with_extension("txt.lock")).unwrap();
        held.lock().unwrap();
        let slovak = Preferences {
            language: Language::from_tag("sk"),
            projection: None,
        };
        assert!(slovak.save(&path).is_err());
        assert!(!path.exists());
        held.unlock().unwrap();

        // A part file another Viewer is writing is not this one's.
        let theirs = path.with_extension("part");
        std::fs::write(&theirs, "another Viewer's").unwrap();
        slovak.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), slovak);
        assert_eq!(
            std::fs::read_to_string(&theirs).unwrap(),
            "another Viewer's"
        );
        assert_eq!(parts_in(folder.path()), [theirs.file_name().unwrap()]);

        let unsupported = || {
            Err(std::fs::TryLockError::Error(std::io::Error::from(
                std::io::ErrorKind::Unsupported,
            )))
        };
        take_turn(unsupported, || (), TURN_WAIT).unwrap();
        #[cfg(unix)]
        take_turn(
            || {
                Err(std::fs::TryLockError::Error(
                    std::io::Error::from_raw_os_error(libc::ENOLCK),
                ))
            },
            || (),
            TURN_WAIT,
        )
        .unwrap();

        // Review 2026-11-06 r23 P3: a save that gave up left a thread waiting in
        // `lock()` with the file open as long as the other Viewer hung.
        let tries = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&tries);
        let busy = move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(std::fs::TryLockError::WouldBlock)
        };
        let error = take_turn(busy, || (), Duration::from_millis(60)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            tries.load(std::sync::atomic::Ordering::Relaxed) > 1,
            "it tried again"
        );
        assert_eq!(std::sync::Arc::strong_count(&tries), 1, "nothing tries on");
    }
}
