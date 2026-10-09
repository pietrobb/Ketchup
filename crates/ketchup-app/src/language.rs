//! The language of the user interface: English or Slovak.
//!
//! At start the language comes from `KETCHUP_LANGUAGE`, then from the choice
//! saved in Window > Language, then from the system language, else English.

use ketchup_interaction::LocaleCatalog;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiLanguage {
    English,
    Slovak,
}

impl UiLanguage {
    pub const ALL: [Self; 2] = [Self::English, Self::Slovak];

    /// A language tag such as `sk`, `sk-SK` or `en_US.UTF-8`; `None` for any
    /// language Kečup has no translation for.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag
            .trim()
            .split(['-', '_', '.'])
            .next()?
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Self::English),
            "sk" => Some(Self::Slovak),
            _ => None,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::Slovak => "sk-SK",
        }
    }

    /// The language's own name, so it can be found whatever language is active.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Slovak => "Slovenčina",
        }
    }

    #[must_use]
    pub fn catalog(self) -> LocaleCatalog {
        match self {
            Self::English => LocaleCatalog::english(),
            Self::Slovak => LocaleCatalog::slovak(),
        }
    }

    /// The first known language of an explicit override, the saved choice and
    /// the system language, in that order; English when none is known.
    #[must_use]
    pub fn resolve(override_tag: Option<&str>, saved: Option<&str>, system: Option<&str>) -> Self {
        [override_tag, saved, system]
            .into_iter()
            .flatten()
            .find_map(Self::from_tag)
            .unwrap_or(Self::English)
    }

    /// The language to start the window in.
    #[must_use]
    pub fn at_startup() -> Self {
        let override_tag = std::env::var("KETCHUP_LANGUAGE").ok();
        Self::resolve(
            override_tag.as_deref(),
            saved_choice().as_deref(),
            system_language().as_deref(),
        )
    }
}

#[cfg(windows)]
const SETTINGS_KEY: &str = r"Software\Ketchup";
#[cfg(windows)]
const LANGUAGE_VALUE: &str = "Language";

#[cfg(windows)]
fn saved_choice() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(SETTINGS_KEY)
        .ok()?
        .get_value(LANGUAGE_VALUE)
        .ok()
}

#[cfg(not(windows))]
fn saved_choice() -> Option<String> {
    None
}

/// Remember `language` for the next start.
#[cfg(windows)]
pub fn save_choice(language: UiLanguage) -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(SETTINGS_KEY)?;
    key.set_value(LANGUAGE_VALUE, &language.tag())
}

#[cfg(not(windows))]
pub fn save_choice(_language: UiLanguage) -> std::io::Result<()> {
    Ok(())
}

/// The user's display language, e.g. `sk-SK`.
#[cfg(windows)]
fn system_language() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Control Panel\International")
        .ok()?
        .get_value("LocaleName")
        .ok()
}

#[cfg(not(windows))]
fn system_language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::UiLanguage;

    #[test]
    fn tags_name_the_language_whatever_their_region_or_encoding() {
        for (tag, language) in [
            ("sk", Some(UiLanguage::Slovak)),
            ("sk-SK", Some(UiLanguage::Slovak)),
            ("SK_sk.UTF-8", Some(UiLanguage::Slovak)),
            ("en-GB", Some(UiLanguage::English)),
            ("cs-CZ", None),
            ("", None),
        ] {
            assert_eq!(UiLanguage::from_tag(tag), language, "{tag}");
        }
    }

    #[test]
    fn override_beats_saved_choice_beats_system_language() {
        assert_eq!(
            UiLanguage::resolve(None, None, Some("sk-SK")),
            UiLanguage::Slovak
        );
        assert_eq!(
            UiLanguage::resolve(None, Some("en-US"), Some("sk-SK")),
            UiLanguage::English
        );
        assert_eq!(
            UiLanguage::resolve(Some("sk"), Some("en-US"), None),
            UiLanguage::Slovak
        );
        assert_eq!(
            UiLanguage::resolve(Some("de"), None, Some("cs-CZ")),
            UiLanguage::English
        );
    }
}
