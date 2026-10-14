//! The language of the user interface: English or Slovak.
//!
//! At start the language comes from `KETCHUP_LANGUAGE`, then from the choice
//! saved in Window > Language, then from the system display languages, else
//! English.

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
    /// the system display languages in preference order; English when none is
    /// known.
    #[must_use]
    pub fn resolve<'a>(
        override_tag: Option<&'a str>,
        saved: Option<&'a str>,
        system: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        [override_tag, saved]
            .into_iter()
            .flatten()
            .chain(system)
            .find_map(Self::from_tag)
            .unwrap_or(Self::English)
    }

    /// The language to start the window in. The system languages are the
    /// user's display languages (as the Viewer reads them), not the regional
    /// format, which may well differ.
    #[must_use]
    pub fn at_startup() -> Self {
        let override_tag = std::env::var("KETCHUP_LANGUAGE").ok();
        let system: Vec<String> = sys_locale::get_locales().collect();
        Self::resolve(
            override_tag.as_deref(),
            saved_choice().as_deref(),
            system.iter().map(String::as_str),
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
            UiLanguage::resolve(None, None, ["sk-SK"]),
            UiLanguage::Slovak
        );
        assert_eq!(
            UiLanguage::resolve(None, Some("en-US"), ["sk-SK"]),
            UiLanguage::English
        );
        assert_eq!(
            UiLanguage::resolve(Some("sk"), Some("en-US"), []),
            UiLanguage::Slovak
        );
        assert_eq!(
            UiLanguage::resolve(Some("de"), None, ["cs-CZ"]),
            UiLanguage::English
        );
    }

    #[test]
    fn the_first_translated_display_language_wins() {
        // A German display language with Slovak as the second preference.
        assert_eq!(
            UiLanguage::resolve(None, None, ["de-DE", "sk-SK", "en-US"]),
            UiLanguage::Slovak
        );
        // An English display language: English even for a Slovak user.
        assert_eq!(
            UiLanguage::resolve(None, None, ["en-GB", "sk-SK"]),
            UiLanguage::English
        );
        // English Windows with the Slovak regional format: the regional format
        // is not a display language, so only `en-US` reaches the resolver.
        assert_eq!(
            UiLanguage::resolve(None, None, ["en-US"]),
            UiLanguage::English
        );
    }
}
