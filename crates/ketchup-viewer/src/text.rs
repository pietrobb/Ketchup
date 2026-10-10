//! Viewer interface text, read from one `locales/<tag>.ftl` catalog per language.
//!
//! To add a language, write `locales/<tag>.ftl` with exactly the keys of
//! `en-US.ftl` and add one line to [`LOCALES`]; nothing else changes. The tests
//! below refuse a catalog whose keys differ from English.
//!
//! The catalogs use the same `key = value` subset of Fluent as the desktop
//! `locales/*.ftl`, with `{ $name }` placeholders, but stay separate so the
//! Viewer does not ship the whole desktop catalog.

use std::{collections::BTreeMap, sync::OnceLock};

/// Every embedded catalog as `(language tag, resource)`. English comes first:
/// it is the fallback and the reference key set.
const LOCALES: &[(&str, &str)] = &[
    ("en-US", include_str!("../locales/en-US.ftl")),
    ("sk-SK", include_str!("../locales/sk-SK.ftl")),
];

type Catalog = BTreeMap<&'static str, &'static str>;

/// The parsed catalogs, in [`LOCALES`] order, parsed once on first use.
fn catalogs() -> &'static [Catalog] {
    static CATALOGS: OnceLock<Vec<Catalog>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        LOCALES
            .iter()
            .map(|(_, resource)| parse(resource))
            .collect()
    })
}

/// `key = value` lines; blank lines and `#` comments are skipped.
fn parse(resource: &'static str) -> Catalog {
    resource
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

/// The primary language subtag: `sk` of `sk-SK`, `SK_sk.UTF-8` or `sk`.
fn primary(tag: &str) -> Option<String> {
    let primary = tag.trim().split(['-', '_', '.']).next()?;
    (!primary.is_empty()).then(|| primary.to_ascii_lowercase())
}

/// One of the embedded interface languages (an index into [`LOCALES`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Language(usize);

impl Language {
    pub const ENGLISH: Self = Self(0);

    /// Every embedded language, English first.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..LOCALES.len()).map(Self)
    }

    /// The catalog's full tag, e.g. `sk-SK`.
    #[must_use]
    pub fn tag(self) -> &'static str {
        LOCALES[self.0].0
    }

    /// A tag such as `sk`, `sk-SK` or `en_US.UTF-8`; `None` when untranslated.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        let wanted = primary(tag)?;
        Self::all().find(|language| primary(language.tag()).as_deref() == Some(wanted.as_str()))
    }

    /// `KETCHUP_LANGUAGE`, then the system languages in preference order, else English.
    #[must_use]
    pub fn detect() -> Self {
        std::env::var("KETCHUP_LANGUAGE")
            .ok()
            .into_iter()
            .chain(sys_locale::get_locales())
            .find_map(|tag| Self::from_tag(&tag))
            .unwrap_or_default()
    }

    /// The message for `key`; English when this catalog lacks it, the key itself
    /// when English lacks it too, so a missing text is visible instead of blank.
    #[must_use]
    pub fn text(self, key: &'static str) -> &'static str {
        let catalogs = catalogs();
        catalogs[self.0]
            .get(key)
            .or_else(|| catalogs[Self::ENGLISH.0].get(key))
            .copied()
            .unwrap_or(key)
    }

    /// [`Self::text`] with every `{ $name }` replaced by its argument.
    #[must_use]
    pub fn format(self, key: &'static str, arguments: &[(&str, &str)]) -> String {
        arguments
            .iter()
            .fold(self.text(key).to_owned(), |text, (name, value)| {
                text.replace(&format!("{{ ${name} }}"), value)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_has_exactly_the_english_keys_and_no_duplicates() {
        let english: Vec<_> = catalogs()[0].keys().collect();
        for (index, (tag, resource)) in LOCALES.iter().enumerate() {
            let lines = resource
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .count();
            assert_eq!(
                lines,
                catalogs()[index].len(),
                "{tag}: malformed or duplicate line"
            );
            assert_eq!(
                catalogs()[index].keys().collect::<Vec<_>>(),
                english,
                "{tag}"
            );
        }
    }

    #[test]
    fn tags_name_the_language_whatever_their_region_or_encoding() {
        let slovak = Language::from_tag("sk-SK");
        assert_eq!(slovak.map(Language::tag), Some("sk-SK"));
        assert_eq!(Language::from_tag("SK_sk.UTF-8"), slovak);
        assert_eq!(Language::from_tag("en-GB"), Some(Language::ENGLISH));
        assert_eq!(Language::from_tag("xx-YY"), None);
        assert_eq!(Language::from_tag(""), None);
    }

    #[test]
    fn placeholders_are_filled_and_missing_keys_stay_visible() {
        let slovak = Language::from_tag("sk").expect("Slovak catalog");
        assert_eq!(
            slovak.format("viewer-material", &[("material", "HPL")]),
            "Materiál: HPL"
        );
        assert_eq!(
            Language::ENGLISH.text("viewer-no-such-key"),
            "viewer-no-such-key"
        );
    }
}
