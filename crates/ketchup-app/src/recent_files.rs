//! File > Open Recent: the documents the user opened or saved last.
//!
//! The list is kept newest first, at most [`RECENT_FILES_LIMIT`] long, each
//! document once however its path was spelled. A window started by the user
//! keeps it with the other user settings (the language choice); a window
//! made by a test keeps it in memory unless the test hands it a store.

pub(crate) use crate::recent_files::presence::Presence;
use crate::recent_files::presence::{local_folder, presence};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub(crate) mod presence;
pub(crate) mod user_settings;

/// How many documents File > Open Recent offers.
pub const RECENT_FILES_LIMIT: usize = 10;

/// Where the list outlives the window.
pub trait RecentFilesStore {
    /// The stored list; empty when none was stored or it cannot be read.
    fn load(&self) -> Vec<PathBuf>;
    /// Replace the stored list.
    fn save(&self, files: &[PathBuf]) -> std::io::Result<()>;
}

/// A list kept in memory and shared by its clones, so that a second window
/// made from a clone reads what the first one stored, as after a restart.
#[derive(Clone, Debug, Default)]
pub struct MemoryRecentFiles(Arc<Mutex<Vec<PathBuf>>>);

impl MemoryRecentFiles {
    /// A store that already holds `files`, newest first.
    #[must_use]
    pub fn holding(files: Vec<PathBuf>) -> Self {
        Self(Arc::new(Mutex::new(files)))
    }

    /// What is stored now.
    #[must_use]
    pub fn stored(&self) -> Vec<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl RecentFilesStore for MemoryRecentFiles {
    fn load(&self) -> Vec<PathBuf> {
        self.stored()
    }

    fn save(&self, files: &[PathBuf]) -> std::io::Result<()> {
        *self.0.lock().unwrap_or_else(|poison| poison.into_inner()) = files.to_vec();
        Ok(())
    }
}

/// The list in the user settings, `HKCU\Software\Ketchup`, next to the
/// language choice.
#[cfg(windows)]
pub(crate) struct RegistryRecentFiles {
    key: &'static str,
}

#[cfg(windows)]
const RECENT_FILES_VALUE: &str = "RecentFiles";

#[cfg(windows)]
impl RegistryRecentFiles {
    pub(crate) const fn user_settings() -> Self {
        Self {
            key: crate::language::SETTINGS_KEY,
        }
    }
}

#[cfg(windows)]
impl RecentFilesStore for RegistryRecentFiles {
    fn load(&self) -> Vec<PathBuf> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;

        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(self.key)
            .and_then(|key| key.get_value::<Vec<std::ffi::OsString>, _>(RECENT_FILES_VALUE))
            .map(|files| files.into_iter().map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    fn save(&self, files: &[PathBuf]) -> std::io::Result<()> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;

        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(self.key)?;
        if files.is_empty() {
            return match key.delete_value(RECENT_FILES_VALUE) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let files: Vec<std::ffi::OsString> = files
            .iter()
            .map(|file| file.as_os_str().to_owned())
            .collect();
        key.set_value(RECENT_FILES_VALUE, &files)
    }
}

/// How long opening the menu waits for the check of the listed documents; a
/// document on a server that does not answer is shown as listed meanwhile.
const CHECK_WAIT: Duration = Duration::from_millis(100);

/// What the list asks the disk; a test stands in for a slow server.
pub(crate) trait PathProbe: Send + Sync {
    fn presence(&self, path: &Path) -> Presence;
    /// `path` with links and `..` resolved; `None` when it is gone or out of reach.
    fn resolved(&self, path: &Path) -> Option<PathBuf>;
}

/// The disk of this computer, a missing folder trusted to be deleted below
/// [`local_folder`] only.
struct Disk {
    local: Option<PathBuf>,
}

impl Disk {
    fn of_this_computer() -> Self {
        Self {
            local: local_folder(),
        }
    }
}

impl PathProbe for Disk {
    fn presence(&self, path: &Path) -> Presence {
        presence(path, self.local.as_deref())
    }

    fn resolved(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
}

/// One listed document as the check found it.
struct Checked {
    path: PathBuf,
    presence: Presence,
    key: Option<String>,
}

/// A picked document whose check runs on a worker.
struct Picking {
    path: PathBuf,
    answer: mpsc::Receiver<Presence>,
    /// The window's document it was picked in (review 2026-11-06 UI-180).
    generation: u64,
}

/// The list File > Open Recent shows, and the store that keeps it. Neither
/// drawing the menu nor adding a document reads the disk for the documents
/// already listed: on a network path that does not answer, each read held the
/// window until the system gave up (review 2026-11-04 UI-141).
pub(crate) struct RecentFiles {
    files: Vec<PathBuf>,
    /// Per file, what names its document: the resolved path once known,
    /// otherwise the path as spelled.
    keys: Vec<String>,
    store: Box<dyn RecentFilesStore>,
    probe: Arc<dyn PathProbe>,
    /// Whether each file was there when the menu last checked.
    on_disk: HashMap<PathBuf, Presence>,
    checking: Option<mpsc::Receiver<Checked>>,
    /// The document picked last, while its check runs on a worker.
    picking: Option<Picking>,
    /// The pass the menu was last drawn in.
    shown_pass: Option<u64>,
}

impl Default for RecentFiles {
    fn default() -> Self {
        Self::from_store(Box::new(MemoryRecentFiles::default()))
    }
}

impl RecentFiles {
    /// The list `store` holds, each document once and no more than the limit.
    pub(crate) fn from_store(store: Box<dyn RecentFilesStore>) -> Self {
        Self::with_probe(store, Arc::new(Disk::of_this_computer()))
    }

    fn with_probe(store: Box<dyn RecentFilesStore>, probe: Arc<dyn PathProbe>) -> Self {
        let mut list = Self {
            files: Vec::new(),
            keys: Vec::new(),
            store,
            probe,
            on_disk: HashMap::new(),
            checking: None,
            picking: None,
            shown_pass: None,
        };
        for file in list.store.load() {
            let key = spelled_key(&file);
            if !list.keys.contains(&key) && list.files.len() < RECENT_FILES_LIMIT {
                list.files.push(file);
                list.keys.push(key);
            }
        }
        list
    }

    pub(crate) fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Put `path` first, dropping an older entry of the same document and the
    /// oldest one past the limit. Only `path` is resolved.
    pub(crate) fn add(&mut self, path: &Path) -> std::io::Result<()> {
        let path = std::path::absolute(path)?;
        let spelled = spelled_key(&path);
        let key = self
            .probe
            .resolved(&path)
            .map_or_else(|| spelled.clone(), |resolved| folded(&resolved));
        self.retain(|_, kept| *kept != key && *kept != spelled);
        self.on_disk.insert(path.clone(), Presence::Here);
        self.files.insert(0, path);
        self.keys.insert(0, key);
        self.files.truncate(RECENT_FILES_LIMIT);
        self.keys.truncate(RECENT_FILES_LIMIT);
        self.store.save(&self.files)
    }

    pub(crate) fn remove(&mut self, path: &Path) -> std::io::Result<()> {
        let key = spelled_key(path);
        self.retain(|file, kept| file != path && *kept != key);
        self.on_disk.remove(path);
        self.store.save(&self.files)
    }

    pub(crate) fn clear(&mut self) -> std::io::Result<()> {
        self.files.clear();
        self.keys.clear();
        self.on_disk.clear();
        self.store.save(&self.files)
    }

    fn retain(&mut self, keep: impl Fn(&Path, &String) -> bool) {
        let (files, keys) = self
            .files
            .drain(..)
            .zip(self.keys.drain(..))
            .filter(|(file, key)| keep(file, key))
            .unzip();
        (self.files, self.keys) = (files, keys);
    }

    /// The menu is drawn in `pass`. When it just opened, its documents are
    /// checked once on a worker, waited for up to [`CHECK_WAIT`]; a check
    /// still running is not started again.
    pub(crate) fn shown(&mut self, pass: u64, context: &egui::Context) {
        let opened = self.shown_pass.is_none_or(|last| last + 1 < pass);
        self.shown_pass = Some(pass);
        self.collect(None);
        if !opened || self.checking.is_some() {
            return;
        }
        let (send, checked) = mpsc::channel();
        let (files, probe, context) =
            (self.files.clone(), Arc::clone(&self.probe), context.clone());
        std::thread::spawn(move || {
            for path in files {
                let presence = probe.presence(&path);
                let key = probe.resolved(&path).map(|resolved| folded(&resolved));
                if send
                    .send(Checked {
                        path,
                        presence,
                        key,
                    })
                    .is_err()
                {
                    return;
                }
            }
            context.request_repaint();
        });
        self.checking = Some(checked);
        self.collect(Some(Instant::now() + CHECK_WAIT));
    }

    /// Takes what the check found so far, waiting until `until` for more.
    fn collect(&mut self, until: Option<Instant>) {
        while let Some(checking) = &self.checking {
            let left = until.and_then(|until| until.checked_duration_since(Instant::now()));
            let next = match left {
                Some(left) => checking.recv_timeout(left).map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => mpsc::TryRecvError::Empty,
                    mpsc::RecvTimeoutError::Disconnected => mpsc::TryRecvError::Disconnected,
                }),
                None => checking.try_recv(),
            };
            match next {
                Ok(checked) => self.apply(checked),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => self.checking = None,
            }
        }
    }

    fn apply(&mut self, checked: Checked) {
        if let Some(index) = self.files.iter().position(|file| *file == checked.path)
            && let Some(key) = checked.key
        {
            self.keys[index] = key;
        }
        self.on_disk.insert(checked.path, checked.presence);
    }

    /// Whether `path` was there when the menu checked; `None` until known.
    pub(crate) fn on_disk(&self, path: &Path) -> Option<Presence> {
        self.on_disk.get(path).copied()
    }

    /// Whether `path`, just picked, is there. One the menu found out of reach
    /// is answered at once; otherwise the disk is asked on a worker, waited for
    /// up to [`CHECK_WAIT`] as the menu waits. `None` while it does not
    /// answer: [`Self::picked`] gives the answer when it comes, and `context`
    /// is woken for it. Asking on the window held it until the system gave up
    /// on a server that does not answer (review 2026-11-04 r19 UI-161).
    /// `generation` is the window's document it is picked in
    /// ([`crate::app_state::FileState::generation`]), given back with the answer.
    pub(crate) fn pick(
        &mut self,
        path: &Path,
        context: Option<egui::Context>,
        generation: u64,
    ) -> Option<Presence> {
        if self.on_disk(path) == Some(Presence::Unknown) {
            return Some(Presence::Unknown);
        }
        let (send, answer) = mpsc::channel();
        let (probe, asked) = (Arc::clone(&self.probe), path.to_owned());
        std::thread::spawn(move || {
            if send.send(probe.presence(&asked)).is_ok()
                && let Some(context) = context
            {
                context.request_repaint();
            }
        });
        self.picking = Some(Picking {
            path: path.to_owned(),
            answer,
            generation,
        });
        self.picked(Some(Instant::now() + CHECK_WAIT))
            .map(|(_, presence, _)| presence)
    }

    /// The document picked last, whether it is there and the generation it was
    /// picked in, once its check answers, waiting until `until` for it; a check
    /// that ended without an answer counts as out of reach.
    pub(crate) fn picked(&mut self, until: Option<Instant>) -> Option<(PathBuf, Presence, u64)> {
        let answer = &self.picking.as_ref()?.answer;
        let left = until.and_then(|until| until.checked_duration_since(Instant::now()));
        let presence = match left {
            Some(left) => match answer.recv_timeout(left) {
                Ok(presence) => presence,
                Err(mpsc::RecvTimeoutError::Timeout) => return None,
                Err(mpsc::RecvTimeoutError::Disconnected) => Presence::Unknown,
            },
            None => match answer.try_recv() {
                Ok(presence) => presence,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => Presence::Unknown,
            },
        };
        let Picking {
            path, generation, ..
        } = self.picking.take()?;
        self.on_disk.insert(path.clone(), presence);
        Some((path, presence, generation))
    }
}

/// What names a document whose path is not resolved: the absolute path as
/// spelled. A document that is gone or out of reach has nothing to resolve;
/// that is a normal state of the list, not a failure to report.
fn spelled_key(path: &Path) -> String {
    folded(
        &std::path::absolute(path)
            .ok()
            .unwrap_or_else(|| path.to_owned()),
    )
}

/// On Windows the same file whatever the letter case.
fn folded(path: &Path) -> String {
    let key = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(store: &MemoryRecentFiles) -> RecentFiles {
        RecentFiles::from_store(Box::new(store.clone()))
    }

    #[test]
    fn the_newest_comes_first_and_the_eleventh_drops_out() {
        let directory = tempfile::tempdir().unwrap();
        let store = MemoryRecentFiles::default();
        let mut recent = list(&store);
        let paths: Vec<PathBuf> = (0..=RECENT_FILES_LIMIT)
            .map(|index| directory.path().join(format!("{index}.ketchup")))
            .collect();
        for path in &paths {
            recent.add(path).unwrap();
        }
        let expected: Vec<PathBuf> = paths
            .iter()
            .rev()
            .take(RECENT_FILES_LIMIT)
            .cloned()
            .collect();
        assert_eq!(recent.files(), expected.as_slice());
        assert_eq!(store.stored(), expected);
    }

    #[test]
    fn a_document_is_listed_once_however_its_path_is_spelled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("House.ketchup");
        std::fs::write(&path, b"x").unwrap();
        let other = directory.path().join("other.ketchup");
        // On Unix `sub/..` names the directory only when `sub` exists.
        std::fs::create_dir(directory.path().join("sub")).unwrap();
        let mut recent = list(&MemoryRecentFiles::default());
        recent.add(&path).unwrap();
        recent.add(&other).unwrap();
        recent
            .add(
                &directory
                    .path()
                    .join("sub")
                    .join("..")
                    .join("House.ketchup"),
            )
            .unwrap();
        assert_eq!(recent.files().len(), 2, "{:?}", recent.files());
        assert_eq!(
            std::fs::canonicalize(&recent.files()[0]).unwrap(),
            std::fs::canonicalize(&path).unwrap()
        );
        if cfg!(windows) {
            recent.add(&directory.path().join("HOUSE.KETCHUP")).unwrap();
            assert_eq!(recent.files().len(), 2, "{:?}", recent.files());
        }
    }

    #[test]
    fn a_stored_list_is_read_back_once_per_document_and_within_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.ketchup");
        let mut stored = vec![path.clone(), path.clone()];
        stored.extend((0..20).map(|index| directory.path().join(format!("{index}.ketchup"))));
        let recent = list(&MemoryRecentFiles::holding(stored));
        assert_eq!(recent.files().len(), RECENT_FILES_LIMIT);
        assert_eq!(recent.files()[0], path);
        assert_ne!(recent.files()[1], path);
    }

    #[test]
    fn clearing_and_removing_are_stored() {
        let directory = tempfile::tempdir().unwrap();
        let (first, second) = (
            directory.path().join("1.ketchup"),
            directory.path().join("2.ketchup"),
        );
        let store = MemoryRecentFiles::default();
        let mut recent = list(&store);
        recent.add(&first).unwrap();
        recent.add(&second).unwrap();
        recent.remove(&first).unwrap();
        assert_eq!(store.stored(), vec![second]);
        recent.clear().unwrap();
        assert!(store.stored().is_empty());
        assert!(list(&store).files().is_empty());
    }

    /// A disk like a server that does not answer until `release` is sent;
    /// counts what it is asked.
    #[derive(Default)]
    struct SlowServer {
        release: Mutex<Option<mpsc::Receiver<()>>>,
        is_file: Mutex<Vec<PathBuf>>,
        resolved: Mutex<Vec<PathBuf>>,
    }

    impl PathProbe for SlowServer {
        fn presence(&self, path: &Path) -> Presence {
            if let Some(release) = self.release.lock().unwrap().take() {
                release.recv().unwrap();
            }
            self.is_file.lock().unwrap().push(path.to_owned());
            if path.ends_with("gone.ketchup") {
                Presence::Gone
            } else {
                Presence::Here
            }
        }

        fn resolved(&self, path: &Path) -> Option<PathBuf> {
            self.resolved.lock().unwrap().push(path.to_owned());
            None
        }
    }

    /// Review 2026-11-04 UI-141: Open Recent asked the disk whether each
    /// document was there in every frame the menu was open, and resolved all
    /// ten paths on the UI thread at every Open and Save As; a document on a
    /// server that did not answer held the window each time. Now opening the
    /// menu checks once, on a worker, and adding resolves only the new path.
    #[test]
    fn open_recent_checks_its_documents_once_per_opening_off_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let (kept, gone) = (
            directory.path().join("kept.ketchup"),
            directory.path().join("gone.ketchup"),
        );
        let (release, released) = mpsc::channel();
        let server = Arc::new(SlowServer {
            release: Mutex::new(Some(released)),
            ..SlowServer::default()
        });
        let store = MemoryRecentFiles::holding(vec![gone.clone(), kept.clone()]);
        let mut recent = RecentFiles::with_probe(Box::new(store), server.clone());
        assert!(
            server.resolved.lock().unwrap().is_empty(),
            "reading the list resolves nothing"
        );
        let context = egui::Context::default();

        // The server does not answer: the menu is drawn, frame after frame,
        // and the check is not started again.
        for pass in 1..=5 {
            recent.shown(pass, &context);
        }
        assert_eq!(recent.on_disk(&gone), None, "not known yet");
        release.send(()).unwrap();
        while recent.checking.is_some() {
            recent.collect(Some(Instant::now() + Duration::from_secs(5)));
        }
        assert_eq!(
            (recent.on_disk(&gone), recent.on_disk(&kept)),
            (Some(Presence::Gone), Some(Presence::Here))
        );
        assert_eq!(
            server.is_file.lock().unwrap().len(),
            2,
            "each document once"
        );

        // Shown again in the next pass: the same opening. After a gap: a new one.
        recent.shown(6, &context);
        recent.shown(9, &context);
        while recent.checking.is_some() {
            recent.collect(Some(Instant::now() + Duration::from_secs(5)));
        }
        assert_eq!(server.is_file.lock().unwrap().len(), 4);

        // Adding a document resolves only that one.
        server.resolved.lock().unwrap().clear();
        let added = directory.path().join("new.ketchup");
        recent.add(&added).unwrap();
        assert_eq!(
            *server.resolved.lock().unwrap(),
            [std::path::absolute(&added).unwrap()]
        );
    }

    /// Review 2026-11-04 r19 UI-161: a click on a recent document asked the
    /// disk on the window, so one the menu had already shown out of reach held
    /// the window until the server timed out. That one is answered at once;
    /// any other is asked on a worker, and the answer comes in a later frame.
    #[test]
    fn a_picked_recent_document_is_checked_off_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let offline = directory.path().join("offline.ketchup");
        let stored = || Box::new(MemoryRecentFiles::holding(vec![offline.clone()]));

        let answering = Arc::new(SlowServer::default());
        let mut recent = RecentFiles::with_probe(stored(), answering.clone());
        recent.on_disk.insert(offline.clone(), Presence::Unknown);
        assert_eq!(recent.pick(&offline, None, 0), Some(Presence::Unknown));
        assert!(
            answering.is_file.lock().unwrap().is_empty(),
            "the menu's answer is not asked again"
        );

        let (release, released) = mpsc::channel();
        let server = Arc::new(SlowServer {
            release: Mutex::new(Some(released)),
            ..SlowServer::default()
        });
        let mut recent = RecentFiles::with_probe(stored(), server.clone());
        // The server answers only when released below, so a pick that asked
        // it here would never return.
        assert_eq!(recent.pick(&offline, None, 7), None, "no answer yet");
        assert_eq!(recent.picked(None), None);
        release.send(()).unwrap();
        assert_eq!(
            recent.picked(Some(Instant::now() + Duration::from_secs(5))),
            Some((offline.clone(), Presence::Here, 7))
        );
        assert_eq!(recent.on_disk(&offline), Some(Presence::Here));
        assert_eq!(recent.picked(None), None, "answered once");
    }

    /// Review 2026-11-06 UI-180: a recent document on a server that answered
    /// late was opened when it did, after the user had made a new model or
    /// opened another one, replacing that one. It opens only if no other
    /// document came to the window since it was picked.
    #[test]
    fn a_late_answer_opens_nothing_once_another_document_came() {
        let directory = tempfile::tempdir().unwrap();
        let picked = directory.path().join("picked.ketchup");
        let other = directory.path().join("other.ketchup");
        for path in [&picked, &other] {
            let mut source = crate::KetchupApp::new();
            assert!(source.create_box());
            assert!(source.save_document_to(path));
        }
        // Review 2026-11-06 r23 P3: one opened for a migration review did
        // not count, so the late answer opened over the review.
        let legacy = directory.path().join("legacy.ketchup");
        std::fs::write(&legacy, crate::tests::lossy_legacy_document()).unwrap();
        type Then = fn(&mut crate::KetchupApp, &Path, &Path);
        let meanwhile: [(&str, Then); 4] = [
            ("nothing", |_, _, _| {}),
            ("New", |app, _, _| app.new_document()),
            ("Open", |app, other, _| {
                assert!(app.open_document_from(other))
            }),
            ("Open for review", |app, _, legacy| {
                assert!(!app.open_document_from(legacy));
                assert!(app.has_review_candidate());
            }),
        ];
        for (what, then) in meanwhile {
            let (release, released) = mpsc::channel();
            let server = Arc::new(SlowServer {
                release: Mutex::new(Some(released)),
                ..SlowServer::default()
            });
            let mut app = crate::KetchupApp::new();
            app.window.recent = RecentFiles::with_probe(
                Box::new(MemoryRecentFiles::holding(vec![picked.clone()])),
                server,
            );
            app.open_recent(&picked);
            assert_eq!(app.document_path(), None, "{what}: not answered yet");
            then(&mut app, &other, &legacy);
            let before = app.document_path().map(Path::to_owned);
            release.send(()).unwrap();
            let until = Instant::now() + Duration::from_secs(60);
            while app.window.recent.picking.is_some() && Instant::now() < until {
                app.poll_recent_pick();
            }
            let expected = if what == "nothing" {
                Some(picked.clone())
            } else {
                before
            };
            assert_eq!(app.document_path().map(Path::to_owned), expected, "{what}");
            if what == "Open for review" {
                assert!(app.has_review_candidate(), "the review is kept");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_user_settings_keep_the_list_between_windows() {
        let key: &'static str = Box::leak(
            format!(r"Software\Ketchup-test-recent-{}", std::process::id()).into_boxed_str(),
        );
        let store = RegistryRecentFiles { key };
        let files = vec![
            PathBuf::from(r"C:\Domy\Měchenice.ketchup"),
            PathBuf::from(r"D:\b.ketchup"),
        ];
        store.save(&files).unwrap();
        assert_eq!(RegistryRecentFiles { key }.load(), files);
        store.save(&[]).unwrap();
        assert!(RegistryRecentFiles { key }.load().is_empty());
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .delete_subkey_all(key)
            .unwrap();
    }
}
