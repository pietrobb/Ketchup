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

    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.path)
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// Every value of `key`, in the order written.
    pub(crate) fn values(&self, key: &str) -> Vec<String> {
        self.lines()
            .iter()
            .filter_map(|line| line.split_once('='))
            .filter(|(name, _)| name.trim() == key)
            .map(|(_, value)| value.to_owned())
            .collect()
    }

    /// Replace every value of `key` with `values`, the other lines kept.
    pub(crate) fn replace(&self, key: &str, values: &[String]) -> io::Result<()> {
        let mut lines: Vec<String> = self
            .lines()
            .into_iter()
            .filter(|line| {
                line.split_once('=')
                    .is_none_or(|(name, _)| name.trim() != key)
            })
            .collect();
        lines.extend(values.iter().map(|value| format!("{key}={value}")));
        let folder = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(folder)?;
        let part = self
            .path
            .with_extension(format!("{}.part", std::process::id()));
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(&part, text)?;
        std::fs::rename(&part, &self.path)
    }

    /// The language tag chosen to be remembered.
    pub(crate) fn language(&self) -> Option<String> {
        self.values(LANGUAGE).pop()
    }

    pub(crate) fn save_language(&self, tag: &str) -> io::Result<()> {
        self.replace(LANGUAGE, &[tag.to_owned()])
    }
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
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        std::fs::remove_dir_all(folder).unwrap();
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
