//! Layer names of an AutoCAD R12 DXF, one for each source name (a tag).

use std::collections::{BTreeMap, BTreeSet};

/// The longest layer name R12 reads.
const R12_LAYER_NAME_MAX: usize = 31;

/// One R12 layer for each source name. R12 does not tell upper from lower
/// case, so a name that folds onto a layer another source already took gets
/// a `_2`, `_3`, … suffix rather than sharing it.
#[derive(Default)]
pub struct R12Layers {
    by_source: BTreeMap<String, String>,
    taken: BTreeSet<String>,
}

impl R12Layers {
    /// Layers named `reserved` are never given to a source.
    #[must_use]
    pub fn reserving(reserved: &[&str]) -> Self {
        Self {
            by_source: BTreeMap::new(),
            taken: reserved
                .iter()
                .map(|name| name.to_ascii_uppercase())
                .collect(),
        }
    }

    pub fn layer(&mut self, source: &str) -> String {
        if let Some(layer) = self.by_source.get(source) {
            return layer.clone();
        }
        let base = layer_name(source);
        let mut layer = base.clone();
        let mut ordinal = 1_usize;
        while !self.taken.insert(layer.to_ascii_uppercase()) {
            ordinal += 1;
            let suffix = format!("_{ordinal}");
            let kept = R12_LAYER_NAME_MAX.saturating_sub(suffix.len());
            layer = format!("{}{suffix}", &base[..base.len().min(kept)]);
        }
        self.by_source.insert(source.to_owned(), layer.clone());
        layer
    }
}

/// A layer name every R12 reader accepts: at most 31 of `A-Z 0-9 $ _ -` in
/// either case, so accents are dropped (`Pôdorys` → `Podorys`) and anything
/// else, a space included, becomes `_`.
fn layer_name(source: &str) -> String {
    let mut name = String::new();
    for character in source.trim().chars() {
        let lower = character.to_lowercase().next().unwrap_or(character);
        let folded = match lower {
            'á' | 'ä' | 'à' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'č' | 'ć' | 'ç' => "c",
            'ď' | 'đ' => "d",
            'é' | 'ě' | 'ë' | 'è' | 'ê' | 'ē' | 'ę' => "e",
            'í' | 'ì' | 'î' | 'ï' => "i",
            'ľ' | 'ĺ' | 'ł' => "l",
            'ň' | 'ń' | 'ñ' => "n",
            'ó' | 'ô' | 'ö' | 'ò' | 'õ' | 'ő' | 'ø' => "o",
            'ŕ' | 'ř' => "r",
            'š' | 'ś' | 'ş' => "s",
            'ť' | 'ţ' => "t",
            'ú' | 'ů' | 'ü' | 'ù' | 'û' | 'ű' => "u",
            'ý' | 'ÿ' => "y",
            'ž' | 'ź' | 'ż' => "z",
            'ß' => "ss",
            other if other.is_ascii_alphanumeric() || matches!(other, '$' | '_' | '-') => {
                name.push(character);
                continue;
            }
            _ => "_",
        };
        if character.is_uppercase() {
            name.push_str(&folded.to_ascii_uppercase());
        } else {
            name.push_str(folded);
        }
    }
    if name.is_empty() {
        "0".to_owned()
    } else {
        name[..name.len().min(R12_LAYER_NAME_MAX)].to_owned()
    }
}
