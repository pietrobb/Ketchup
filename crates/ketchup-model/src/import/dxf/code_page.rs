//! The code page of a DXF file older than 2007, named by its `$DWGCODEPAGE` header.

use super::DxfImportError;
use std::borrow::Cow;

/// A code page a DXF older than 2007 may be written in: a Windows or East Asian one, or a
/// DOS one `encoding_rs` has no decoder for, read by its table of bytes 128 to 255.
#[derive(Clone, Copy)]
pub(super) enum CodePage {
    Standard(&'static encoding_rs::Encoding),
    Dos(&'static [char; 128]),
}

impl CodePage {
    pub(super) const WINDOWS_1252: Self = Self::Standard(encoding_rs::WINDOWS_1252);

    /// `bytes` read in this code page; `None` where it has no character for them.
    pub(super) fn decode(self, bytes: &[u8]) -> Option<Cow<'_, str>> {
        match self {
            Self::Standard(encoding) => {
                encoding.decode_without_bom_handling_and_without_replacement(bytes)
            }
            Self::Dos(high) => Some(if bytes.is_ascii() {
                Cow::Borrowed(std::str::from_utf8(bytes).ok()?)
            } else {
                Cow::Owned(
                    bytes
                        .iter()
                        .map(|byte| match byte.checked_sub(128) {
                            Some(at) => high[usize::from(at)],
                            None => char::from(*byte),
                        })
                        .collect(),
                )
            }),
        }
    }

    /// Whether the text `high` saw reads wrong in this code page: a byte of it reads as a
    /// C1 control character, which no drawing writes but a Windows code page reads a byte
    /// it leaves undefined as; or, in Windows-1252, more of its words read rightly only in
    /// Windows-1250 than only in 1252 ([`HighBytes::see`]), so that one word does not turn
    /// a Western drawing (review 2026-11-02 UL-120). Either way the text was most likely
    /// written in another code page than its header names.
    pub(super) fn doubtful(self, high: &HighBytes) -> bool {
        self.reads_c1(high)
            || (self.is(encoding_rs::WINDOWS_1252) && high.central_words > high.western_words)
    }

    /// Whether this code page has a byte for `character`.
    fn writes(self, character: char) -> bool {
        match self {
            Self::Standard(encoding) => {
                let mut buffer = [0; 4];
                !encoding.encode(character.encode_utf8(&mut buffer)).2
            }
            Self::Dos(high) => high.contains(&character),
        }
    }

    fn is(self, encoding: &'static encoding_rs::Encoding) -> bool {
        matches!(self, Self::Standard(own) if own == encoding)
    }

    fn reads_c1(self, high: &HighBytes) -> bool {
        let Self::Standard(encoding) = self else {
            return false;
        };
        encoding.is_single_byte()
            && (128..=255_u8)
                .filter(|byte| high.seen[usize::from(*byte - 128)])
                .any(|byte| {
                    self.decode(&[byte]).is_some_and(|text| {
                        text.chars()
                            .any(|character| ('\u{80}'..='\u{9F}').contains(&character))
                    })
                })
    }
}

/// Which of the bytes 128 to 255 a text holds, and how many of its words read rightly
/// only in Windows-1252 and how many only in Windows-1250.
pub(super) struct HighBytes {
    seen: [bool; 128],
    western_words: usize,
    central_words: usize,
}

impl Default for HighBytes {
    fn default() -> Self {
        Self {
            seen: [false; 128],
            western_words: 0,
            central_words: 0,
        }
    }
}

impl HighBytes {
    /// Sees a line of the text, and of each of its words which of Windows-1252 and
    /// Windows-1250 alone reads it rightly ([`reading_of`]).
    pub(super) fn see(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if let Some(at) = byte.checked_sub(128) {
                self.seen[usize::from(at)] = true;
            }
        }
        let in_word = |byte: u8| byte.is_ascii_alphabetic() || byte >= 128;
        let mut at = 0;
        while at < bytes.len() {
            if !in_word(bytes[at]) {
                at += 1;
                continue;
            }
            let start = at;
            while bytes.get(at).is_some_and(|byte| in_word(*byte)) {
                at += 1;
            }
            match reading_of(&bytes[start..at], bytes.get(at) == Some(&b'.')) {
                Some(Reading::Western) => self.western_words += 1,
                Some(Reading::CentralEuropean) => self.central_words += 1,
                None => {}
            }
        }
    }
}

/// The code page a word reads rightly in.
#[derive(Debug, PartialEq)]
enum Reading {
    Western,
    CentralEuropean,
}

/// Which of Windows-1252 and Windows-1250 alone reads `word` as a word of its languages
/// ([`western_word`], [`central_word`]); `None` where both read it alike, or both or
/// neither read it rightly. `abbreviation`: a period follows it. ª and º, 1252's ordinal
/// indicators and 1250's Ş and ş, are left out: „Nº“, „1ª“ and „45º“ tell neither
/// (review 2026-11-02 UL-120).
fn reading_of(word: &[u8], abbreviation: bool) -> Option<Reading> {
    let word = word
        .iter()
        .copied()
        .filter(|byte| !matches!(byte, 0xAA | 0xBA))
        .collect::<Vec<_>>();
    let read = |encoding: &'static encoding_rs::Encoding| {
        encoding
            .decode_without_bom_handling_and_without_replacement(&word)
            .map(|text| text.chars().collect::<Vec<_>>())
    };
    let (Some(western), Some(central)) = (
        read(encoding_rs::WINDOWS_1252),
        read(encoding_rs::WINDOWS_1250),
    ) else {
        return None;
    };
    if western == central {
        return None;
    }
    match (
        western_word(&western, &central, abbreviation),
        central_word(&central),
    ) {
        (true, false) => Some(Reading::Western),
        (false, true) => Some(Reading::CentralEuropean),
        _ => None,
    }
}

/// Whether a character is a C1 control character, which no drawing writes but a Windows
/// code page reads a byte it leaves undefined as.
fn c1(character: &char) -> bool {
    ('\u{80}'..='\u{9F}').contains(character)
}

/// Whether `western`, a word read in Windows-1252 that Windows-1250 reads `central`, is a
/// word of a Western language. It is not where 1252 reads a sign that 1250 reads as a
/// letter standing as one: between letters, or ending a word after a vowel („Kúpe¾“ for
/// „Kúpeľ“, while „G¾“, „3¾“ and „m³“ keep their signs; review 2026-11-02 UL-120); nor
/// where ø stands beside an accented letter of no Nordic language, or ì, ò or ù inside a
/// word („Pøíèka“, „mìsto“; review 2026-10-26 UL-43); nor where a grave-accented vowel
/// alone is abbreviated, as no Western word begins with one („è. 1“ for „č. 1“); nor
/// where its letters are of no one Western language ([`WesternLanguage::spells`]).
fn western_word(western: &[char], central: &[char], abbreviation: bool) -> bool {
    if western.iter().any(c1) {
        return false;
    }
    let vowel = |character: &char| !character.is_ascii() || "aeiouyAEIOUY".contains(*character);
    let letter_read_as_sign = (0..western.len()).any(|at| {
        let after_letter = at > 0 && western[at - 1].is_alphabetic();
        let before_letter = western.get(at + 1).is_some_and(|next| next.is_alphabetic());
        let ending = at + 1 == western.len() && western[..at].iter().any(vowel);
        !western[at].is_alphabetic()
            && central[at].is_alphabetic()
            && after_letter
            && (before_letter || ending)
    });
    let word = western
        .iter()
        .flat_map(|character| character.to_lowercase())
        .collect::<Vec<_>>();
    const NORDIC: &str = "æåäöüéø";
    let slashed_o_beside_other = word.contains(&'ø')
        && word.iter().any(|letter| {
            letter.is_alphabetic() && !letter.is_ascii() && !NORDIC.contains(*letter)
        });
    let inner_grave = word
        .iter()
        .rev()
        .skip(1)
        .any(|letter| matches!(letter, 'ì' | 'ò' | 'ù'));
    let abbreviated_grave = abbreviation && matches!(word[..], ['à' | 'è' | 'ì' | 'ò' | 'ù']);
    !(letter_read_as_sign || slashed_o_beside_other || inner_grave || abbreviated_grave)
        && WESTERN.iter().any(|language| language.spells(&word))
}

/// Whether `central`, a word read in Windows-1250, has its accented letters of one
/// Central European language.
fn central_word(central: &[char]) -> bool {
    if central.iter().any(c1) {
        return false;
    }
    let accented = central
        .iter()
        .filter(|character| !character.is_ascii() && character.is_alphabetic())
        .flat_map(|character| character.to_lowercase())
        .collect::<Vec<_>>();
    CENTRAL_EUROPEAN
        .iter()
        .any(|language| accented.iter().all(|letter| language.contains(*letter)))
}

/// A Western language written in Windows-1252, as far as telling its words from Central
/// European words read in 1252 needs.
struct WesternLanguage {
    /// Its accented letters.
    accented: &'static str,
    /// The letters of the Latin alphabet no native word of it spells: no Italian word has
    /// k or y, so „Kuchynì“ is „Kuchyně“.
    foreign: &'static str,
    /// Whether it accents a stressed vowel ending a word only after a consonant or i or u
    /// (Italian „città“, „più“, „può“), so that „Predsieò“ is „Predsieň“.
    stressed_end: bool,
}

impl WesternLanguage {
    const fn new(accented: &'static str) -> Self {
        Self {
            accented,
            foreign: "",
            stressed_end: false,
        }
    }

    /// Whether the lower-case `word` is spelt in this language.
    fn spells(&self, word: &[char]) -> bool {
        let letters = word.iter().all(|letter| {
            if letter.is_ascii() {
                !self.foreign.contains(*letter)
            } else {
                !letter.is_alphabetic() || self.accented.contains(*letter)
            }
        });
        let stressed_end_after_vowel =
            matches!(word, [.., 'a' | 'e' | 'o', 'à' | 'è' | 'ì' | 'ò' | 'ù']);
        letters && !(self.stressed_end && stressed_end_after_vowel)
    }
}

/// The Western languages written in Windows-1252.
const WESTERN: &[WesternLanguage] = &[
    WesternLanguage::new("àâæçéèêëîïôœùûüÿ"), // French
    WesternLanguage::new("áéíñóúü"),          // Spanish
    WesternLanguage::new("áâãàçéêíóôõú"),     // Portuguese
    WesternLanguage {
        foreign: "jkwxy",
        stressed_end: true,
        ..WesternLanguage::new("àèéìíîòóùú")
    }, // Italian
    WesternLanguage {
        stressed_end: true,
        ..WesternLanguage::new("àçèéíïòóúü")
    }, // Catalan
    WesternLanguage::new("äöüß"),             // German
    WesternLanguage::new("éèëïöüà"),          // Dutch
    WesternLanguage {
        foreign: "cqwxz",
        ..WesternLanguage::new("æøåéó")
    }, // Danish and Norwegian
    WesternLanguage::new("åäöé"),             // Swedish
    WesternLanguage {
        foreign: "cqwz",
        ..WesternLanguage::new("áðéíóúýþæö")
    }, // Icelandic
    WesternLanguage {
        foreign: "cqwxz",
        ..WesternLanguage::new("áðíóúýæø")
    }, // Faroese
    WesternLanguage::new("äöåšž"),            // Finnish
    WesternLanguage::new("äöõüšž"),           // Estonian
    WesternLanguage::new("áéíóú"),            // Irish
];

/// The accented letters of the Central European languages written in Windows-1250.
const CENTRAL_EUROPEAN: &[&str] = &[
    "áčďéěíňóřšťúůýž",   // Czech
    "áäčďéíĺľňóôŕšťúýž", // Slovak
    "ąćęłńóśźż",         // Polish
    "áéíóöőúüű",         // Hungarian
    "čćđšž",             // Slovenian and Croatian
    "ăâîşţ",             // Romanian
    "äöüß",              // German
];

/// Windows-1250, which Central European text read wrongly in Windows-1252 was written in.
const CENTRAL_EUROPE: CodePage = CodePage::Standard(encoding_rs::WINDOWS_1250);

/// The code page to read a text in whose `$DWGCODEPAGE` header is `name`, `high` having
/// seen the text: the one it names, or Windows-1250 in place of a doubtful Windows-1252
/// that 1250 reads without a C1 character. With its name, and the name in the header
/// when that is [doubtful](CodePage::doubtful).
pub(super) fn text_code_page(name: &[u8], high: &HighBytes) -> Option<(CodePage, CodePageRead)> {
    let named = code_page_encoding(name)?;
    if !named.doubtful(high) {
        return Some((named, (code_page_name(name), None)));
    }
    let doubted = Some(code_page_name(name));
    if named.is(encoding_rs::WINDOWS_1252) && !CENTRAL_EUROPE.reads_c1(high) {
        return Some((CENTRAL_EUROPE, ("ANSI_1250".to_owned(), doubted)));
    }
    Some((named, (code_page_name(name), doubted)))
}

/// The name of the code page a text was read in, and that of the code page its header
/// names when that is [doubtful](CodePage::doubtful).
pub(super) type CodePageRead = (String, Option<String>);

/// The text of a DXF file, and the code page it was read in, if any. DXF 2007 and later
/// are UTF-8; an older file is written in the code page its `$DWGCODEPAGE` header names,
/// so a file is read in that code page when it is older than 2007 and names one, unless
/// it is UTF-8 that [fits](utf8_fits) the code page, as some programs write.
pub(super) fn dxf_source_text(
    source: &[u8],
) -> Result<(Cow<'_, str>, Option<CodePageRead>), DxfImportError> {
    if source.is_empty() {
        return Err(DxfImportError::Empty);
    }
    // A UTF-8 byte order mark only says how the rest is written.
    let source = source
        .strip_prefix(super::BYTE_ORDER_MARK)
        .unwrap_or(source);
    let header_value = |variable: &[u8]| {
        let mut lines = source.split(|byte| *byte == b'\n').map(<[u8]>::trim_ascii);
        lines.position(|line| line == variable)?;
        lines.nth(1)
    };
    let name = header_value(b"$ACADVER")
        .is_none_or(|version| version < b"AC1021".as_slice())
        .then(|| header_value(b"$DWGCODEPAGE"))
        .flatten();
    if let Ok(text) = std::str::from_utf8(source)
        && name
            .and_then(code_page_encoding)
            .is_none_or(|named| utf8_fits(text.chars(), named))
    {
        return Ok((Cow::Borrowed(text), None));
    }
    let name = name.ok_or(DxfImportError::NonAscii)?;
    let mut high = HighBytes::default();
    for line in source.split(|byte| *byte == b'\n') {
        high.see(line);
    }
    let (code_page, read) = text_code_page(name, &high).ok_or(DxfImportError::NonAscii)?;
    let text = code_page.decode(source).ok_or(DxfImportError::NonAscii)?;
    Ok((text, Some(read)))
}

/// Whether `text`, a file older than DXF 2007 that is valid UTF-8, was written in UTF-8
/// rather than in the code page `named` its header names: each character beyond ASCII is
/// one `named` writes or a Latin letter. Bytes of a code page form valid UTF-8 only by
/// chance, and then mostly characters of other scripts: „Rohr Ø½“ in Windows-1252 is
/// „Rohr ؽ“ (review 2026-11-02 UL-121).
pub(super) fn utf8_fits(text: impl IntoIterator<Item = char>, named: CodePage) -> bool {
    text.into_iter()
        .filter(|character| !character.is_ascii())
        .all(|character| {
            let latin = matches!(character, '\u{C0}'..='\u{24F}' | '\u{1E00}'..='\u{1EFF}')
                && character.is_alphabetic();
            latin || named.writes(character)
        })
}

/// A `$DWGCODEPAGE` name [`code_page_encoding`] reads, as AutoCAD writes it: `ANSI_1250`.
pub(super) fn code_page_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// The code page of an AutoCAD `$DWGCODEPAGE` name such as `ANSI_1250` or `DOS852`.
pub(super) fn code_page_encoding(name: &[u8]) -> Option<CodePage> {
    let upper = name.to_ascii_uppercase();
    if let Some(number) = upper.strip_prefix(b"DOS") {
        return match number {
            b"850" => Some(CodePage::Dos(&DOS_850)),
            b"852" => Some(CodePage::Dos(&DOS_852)),
            b"866" => Some(CodePage::Standard(encoding_rs::IBM866)),
            _ => None,
        };
    }
    let number = upper.strip_prefix(b"ANSI_")?;
    let encoding = match number {
        b"874" | b"1250" | b"1251" | b"1252" | b"1253" | b"1254" | b"1255" | b"1256" | b"1257"
        | b"1258" => {
            let mut label = b"windows-".to_vec();
            label.extend_from_slice(number);
            encoding_rs::Encoding::for_label(&label)?
        }
        b"932" => encoding_rs::SHIFT_JIS,
        b"936" => encoding_rs::GBK,
        b"949" => encoding_rs::EUC_KR,
        b"950" => encoding_rs::BIG5,
        _ => return None,
    };
    Some(CodePage::Standard(encoding))
}

/// Bytes 128 to 255 of DOS code page 850 (Western Europe).
#[rustfmt::skip]
static DOS_850: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00F8}', '\u{00A3}', '\u{00D8}', '\u{00D7}', '\u{0192}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    '\u{00BF}', '\u{00AE}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00C1}', '\u{00C2}', '\u{00C0}',
    '\u{00A9}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{00A2}', '\u{00A5}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{00E3}', '\u{00C3}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{00A4}',
    '\u{00F0}', '\u{00D0}', '\u{00CA}', '\u{00CB}', '\u{00C8}', '\u{0131}', '\u{00CD}', '\u{00CE}',
    '\u{00CF}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{00A6}', '\u{00CC}', '\u{2580}',
    '\u{00D3}', '\u{00DF}', '\u{00D4}', '\u{00D2}', '\u{00F5}', '\u{00D5}', '\u{00B5}', '\u{00FE}',
    '\u{00DE}', '\u{00DA}', '\u{00DB}', '\u{00D9}', '\u{00FD}', '\u{00DD}', '\u{00AF}', '\u{00B4}',
    '\u{00AD}', '\u{00B1}', '\u{2017}', '\u{00BE}', '\u{00B6}', '\u{00A7}', '\u{00F7}', '\u{00B8}',
    '\u{00B0}', '\u{00A8}', '\u{00B7}', '\u{00B9}', '\u{00B3}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

/// Bytes 128 to 255 of DOS code page 852 (Central Europe).
#[rustfmt::skip]
static DOS_852: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{016F}', '\u{0107}', '\u{00E7}',
    '\u{0142}', '\u{00EB}', '\u{0150}', '\u{0151}', '\u{00EE}', '\u{0179}', '\u{00C4}', '\u{0106}',
    '\u{00C9}', '\u{0139}', '\u{013A}', '\u{00F4}', '\u{00F6}', '\u{013D}', '\u{013E}', '\u{015A}',
    '\u{015B}', '\u{00D6}', '\u{00DC}', '\u{0164}', '\u{0165}', '\u{0141}', '\u{00D7}', '\u{010D}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{0104}', '\u{0105}', '\u{017D}', '\u{017E}',
    '\u{0118}', '\u{0119}', '\u{00AC}', '\u{017A}', '\u{010C}', '\u{015F}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00C1}', '\u{00C2}', '\u{011A}',
    '\u{015E}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{017B}', '\u{017C}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{0102}', '\u{0103}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{00A4}',
    '\u{0111}', '\u{0110}', '\u{010E}', '\u{00CB}', '\u{010F}', '\u{0147}', '\u{00CD}', '\u{00CE}',
    '\u{011B}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{0162}', '\u{016E}', '\u{2580}',
    '\u{00D3}', '\u{00DF}', '\u{00D4}', '\u{0143}', '\u{0144}', '\u{0148}', '\u{0160}', '\u{0161}',
    '\u{0154}', '\u{00DA}', '\u{0155}', '\u{0170}', '\u{00FD}', '\u{00DD}', '\u{0163}', '\u{00B4}',
    '\u{00AD}', '\u{02DD}', '\u{02DB}', '\u{02C7}', '\u{02D8}', '\u{00A7}', '\u{00F7}', '\u{00B8}',
    '\u{00B0}', '\u{00A8}', '\u{02D9}', '\u{0171}', '\u{0158}', '\u{0159}', '\u{25A0}', '\u{00A0}',
];
