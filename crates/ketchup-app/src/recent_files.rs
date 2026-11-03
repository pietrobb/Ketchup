//! File > Open Recent: the documents the user opened or saved last.
//!
//! The list is kept newest first, at most [`RECENT_FILES_LIMIT`] long, each
//! document once however its path was spelled. A window started by the user
//! keeps it with the other user settings (the language choice); a window
//! made by a test keeps it in memory unless the test hands it a store.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// The list File > Open Recent shows, and the store that keeps it.
pub(crate) struct RecentFiles {
    files: Vec<PathBuf>,
    store: Box<dyn RecentFilesStore>,
}

impl Default for RecentFiles {
    fn default() -> Self {
        Self::from_store(Box::new(MemoryRecentFiles::default()))
    }
}

impl RecentFiles {
    /// The list `store` holds, each document once and no more than the limit.
    pub(crate) fn from_store(store: Box<dyn RecentFilesStore>) -> Self {
        let mut files: Vec<PathBuf> = Vec::new();
        for file in store.load() {
            if !files.iter().any(|kept| same_document(kept, &file)) {
                files.push(file);
            }
        }
        files.truncate(RECENT_FILES_LIMIT);
        Self { files, store }
    }

    pub(crate) fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Put `path` first, dropping an older entry of the same document and the
    /// oldest one past the limit.
    pub(crate) fn add(&mut self, path: &Path) -> std::io::Result<()> {
        let path = std::path::absolute(path)?;
        self.files.retain(|kept| !same_document(kept, &path));
        self.files.insert(0, path);
        self.files.truncate(RECENT_FILES_LIMIT);
        self.store.save(&self.files)
    }

    pub(crate) fn remove(&mut self, path: &Path) -> std::io::Result<()> {
        self.files.retain(|kept| !same_document(kept, path));
        self.store.save(&self.files)
    }

    pub(crate) fn clear(&mut self) -> std::io::Result<()> {
        self.files.clear();
        self.store.save(&self.files)
    }
}

/// Whether two paths name the same document: the same file once links and
/// `..` are resolved, and on Windows whatever the letter case. A document that
/// is gone or out of reach has nothing to resolve, so its path is compared as
/// spelled; that is a normal state of the list, not a failure to report.
fn same_document(left: &Path, right: &Path) -> bool {
    document_key(left) == document_key(right)
}

fn document_key(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path)
        .ok()
        .or_else(|| std::path::absolute(path).ok())
        .unwrap_or_else(|| path.to_owned());
    let key = resolved.to_string_lossy().into_owned();
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
        assert!(same_document(&recent.files()[0], &path));
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
