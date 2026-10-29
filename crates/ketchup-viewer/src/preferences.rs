//! The customer's Viewer settings, kept between starts: the interface
//! language and projection chosen in the Settings menu. The Viewer only holds them; each
//! host stores the text of [`Preferences::to_text`] where it can (a file on the
//! desktop, `localStorage` in a browser, private storage on a phone).
//!
//! The text is `key = value` lines; unknown keys and values are skipped, so
//! settings written by a newer Viewer still open in an older one.

use crate::text::Language;
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::FittedProjection;
use std::path::{Path, PathBuf};

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
        let mut text = String::from("# Kečup Viewer settings\n");
        if let Some(language) = self.language {
            text.push_str(&format!("language = {}\n", language.tag()));
        }
        if let Some(projection) = self.projection {
            text.push_str(&format!("projection = {}\n", projection_name(projection)));
        }
        text
    }

    /// The settings in `path`; none yet (or unreadable) means the defaults.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    /// Write to `path` through `<path>.part`, so a cut-off write keeps the old settings.
    pub fn save(&self, path: &Path) -> Result<(), Rejection> {
        let failed = |error: std::io::Error| {
            Rejection::new("viewer.settings.save", RejectionPhase::Io)
                .target(path.display().to_string())
                .reason(
                    "The Viewer settings could not be saved; they apply until the Viewer closes.",
                )
                .fix_hint(
                    "Make the folder of the settings file writable, then choose the setting again.",
                )
                .caused_by(error)
        };
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(failed)?;
        }
        let part = path.with_extension("part");
        std::fs::write(&part, self.to_text()).map_err(failed)?;
        std::fs::rename(&part, path).map_err(failed)
    }
}

/// Where the desktop Viewer keeps its settings: `%APPDATA%\Ketchup\viewer-settings.txt`
/// on Windows, `~/Library/Application Support/Ketchup/` on macOS, else
/// `$XDG_CONFIG_HOME/Ketchup/` or `~/.config/Ketchup/`.
#[must_use]
pub fn desktop_settings_path() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Application Support"))
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))
    };
    base.filter(|base| base.is_absolute())
        .map(|base| base.join("Ketchup").join("viewer-settings.txt"))
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
        assert!(!path.with_extension("part").exists());
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
}
