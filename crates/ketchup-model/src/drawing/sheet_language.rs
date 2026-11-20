use serde::{Deserialize, Serialize};

/// A language the sheets can be titled in, whatever the window's: a Czech
/// project goes to a Czech building office.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum SheetLanguage {
    #[serde(rename = "cs")]
    Czech,
    #[serde(rename = "sk")]
    Slovak,
    #[serde(rename = "en")]
    English,
}

impl SheetLanguage {
    pub const ALL: [Self; 3] = [Self::Czech, Self::Slovak, Self::English];

    /// The ISO 639-1 code it is kept and asked for by.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Czech => "cs",
            Self::Slovak => "sk",
            Self::English => "en",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
    }

    /// Czech and Slovak sheets write a decimal comma.
    #[must_use]
    pub const fn decimal_comma(self) -> bool {
        !matches!(self, Self::English)
    }
}

/// The words a parts-list row prints beside its part name and quantity, in
/// the language of the sheet's reader; English by default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartsListWords {
    /// Follows the name of an opposite-hand part, in parentheses.
    pub mirrored: String,
    /// Precedes the quantity.
    pub quantity: String,
}

impl Default for PartsListWords {
    fn default() -> Self {
        Self {
            mirrored: "mirrored".to_owned(),
            quantity: "QTY".to_owned(),
        }
    }
}
