//! The code page of a DXF file older than 2007, named by its `$DWGCODEPAGE` header.

use super::DxfImportError;
use std::borrow::Cow;
use std::collections::BTreeMap;

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
            Self::Dos(high) => Some(
                if bytes.is_ascii()
                    && let Ok(ascii) = std::str::from_utf8(bytes)
                {
                    Cow::Borrowed(ascii)
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
                },
            ),
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
        self.encode(character).is_some()
    }

    /// `character` in this code page; `None` where it has none for it.
    pub(super) fn encode(self, character: char) -> Option<Vec<u8>> {
        match self {
            Self::Standard(encoding) => {
                let mut buffer = [0; 4];
                let (bytes, _, unmapped) = encoding.encode(character.encode_utf8(&mut buffer));
                (!unmapped).then(|| bytes.into_owned())
            }
            Self::Dos(high) => (128..=u8::MAX)
                .zip(high.iter())
                .find(|(_, own)| **own == character)
                .map(|(byte, _)| vec![byte]),
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
            let before_number = bytes[at..]
                .iter()
                .find(|byte| **byte != b' ')
                .is_some_and(u8::is_ascii_digit);
            match reading_of(
                &bytes[start..at],
                bytes.get(at) == Some(&b'.'),
                before_number,
            ) {
                Some(Reading::Western) => self.western_words += 1,
                Some(Reading::CentralEuropean) => self.central_words += 1,
                None => {}
            }
        }
    }

    /// Whether the words read rightly only in Windows-1250 outvote those read rightly only
    /// in 1252 with room to spare: two at least, and more than twice as many (review
    /// 2026-11-04 UL-140: one against none, or one byte 1252 leaves undefined, turned a
    /// whole Western drawing).
    fn central_european(&self) -> bool {
        self.central_words >= 2 && self.central_words > 2 * self.western_words
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
/// (review 2026-11-02 UL-120). A word both read rightly is Western where it has a sign
/// 1250 reads as a mark no word has, ² or ½, or is Ø before a number: „m²“, „Ø 100“
/// (review 2026-11-04 UL-140). `before_number`: a number follows it.
fn reading_of(word: &[u8], abbreviation: bool, before_number: bool) -> Option<Reading> {
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
    let sign = western
        .iter()
        .any(|character| matches!(character, '²' | '½'))
        || (before_number && matches!(western[..], ['Ø' | 'ø']));
    match (
        western_word(&western, &central, abbreviation),
        central_word(&central),
    ) {
        (true, false) => Some(Reading::Western),
        (false, true) => Some(Reading::CentralEuropean),
        (true, true) if sign => Some(Reading::Western),
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
/// letter standing as one: between letters, or, ¾ or ¼, ending a word after a vowel
/// („Kúpe¾“ for „Kúpeľ“, while „G¾“, „3¾“ and „m³“ keep their signs; review 2026-11-02
/// UL-120; „Fenster¹“ and „A³“ keep theirs too, review 2026-11-04 UL-140); nor
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
        let ending = at + 1 == western.len()
            && matches!(western[at], '¾' | '¼')
            && western[..at].iter().any(vowel);
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
/// whose words [read](HighBytes::central_european) in 1250 and that 1250 reads without a
/// C1 character. With its name, and the name in the header when that is
/// [doubtful](CodePage::doubtful).
fn text_code_page(name: &[u8], high: &HighBytes) -> Option<(CodePage, CodePageRead)> {
    let named = code_page_encoding(name)?;
    if !named.doubtful(high) {
        return Some((named, (code_page_name(name), None)));
    }
    let doubted = Some(code_page_name(name));
    if named.is(encoding_rs::WINDOWS_1252)
        && high.central_european()
        && !CENTRAL_EUROPE.reads_c1(high)
    {
        return Some((CENTRAL_EUROPE, ("ANSI_1250".to_owned(), doubted)));
    }
    Some((named, (code_page_name(name), doubted)))
}

/// The name of the code page a text was read in, and that of the code page its header
/// names when that is [doubtful](CodePage::doubtful).
pub(super) type CodePageRead = (String, Option<String>);

/// How the text of a DXF file is read: in UTF-8 (`None`) or in a code page, and what is
/// said of it.
pub(super) struct TextEncoding {
    pub(super) code_page: Option<CodePage>,
    pub(super) read: Option<CodePageRead>,
}

/// What the lines of a DXF file hold, seen one by one, for [`TextSeen::decide`]: the one
/// decision of its encoding, for the import, the underlay and the rewrite alike (review
/// 2026-11-04 UL-142: two of them had come apart, and one saw the byte order mark).
pub(super) struct TextSeen {
    lines: usize,
    utf8: bool,
    /// The characters beyond ASCII, each with how often, while the text is UTF-8.
    beyond_ascii: BTreeMap<char, usize>,
    high: HighBytes,
    acadver: HeaderValue,
    code_page: HeaderValue,
}

impl Default for TextSeen {
    fn default() -> Self {
        Self {
            lines: 0,
            utf8: true,
            beyond_ascii: BTreeMap::new(),
            high: HighBytes::default(),
            acadver: HeaderValue::default(),
            code_page: HeaderValue::default(),
        }
    }
}

impl TextSeen {
    /// Sees the next line of the file.
    pub(super) fn see(&mut self, line: &[u8]) {
        // A UTF-8 byte order mark only says how the rest is written.
        let line = match self.lines {
            0 => line.strip_prefix(super::BYTE_ORDER_MARK).unwrap_or(line),
            _ => line,
        };
        self.lines += 1;
        if self.utf8 {
            for chunk in line.utf8_chunks() {
                for character in chunk
                    .valid()
                    .chars()
                    .filter(|character| !character.is_ascii())
                {
                    *self.beyond_ascii.entry(character).or_default() += 1;
                }
                if !chunk.invalid().is_empty() {
                    self.utf8 = false;
                    self.beyond_ascii.clear();
                    break;
                }
            }
        }
        self.high.see(line);
        let trimmed = line.trim_ascii();
        self.acadver.see(trimmed, b"$ACADVER");
        self.code_page.see(trimmed, b"$DWGCODEPAGE");
    }

    /// The encoding of the file seen. DXF 2007 and later are UTF-8; an older file is
    /// written in the code page its `$DWGCODEPAGE` header names, or in the one read in its
    /// place ([`text_code_page`]), unless it is UTF-8 as some programs write it: a file
    /// that is valid UTF-8 is read so when a character of it is of three or four bytes,
    /// which a code page forms only by chance, or when more of its characters of two are
    /// ones the code page writes or Latin letters than are not; „Rohr Ø½“ in Windows-1252
    /// is „Rohr ؽ“ as UTF-8 (review 2026-11-02 UL-121). Where both readings fit some of
    /// its characters, the code page named is doubted (review 2026-11-04 UL-141: one
    /// character sent a whole UTF-8 file through the code page, unsaid).
    pub(super) fn decide(self) -> Result<TextEncoding, DxfImportError> {
        if self.lines == 0 {
            return Err(DxfImportError::Empty);
        }
        let name = self
            .acadver
            .value
            .is_none_or(|version| version.as_slice() < b"AC1021".as_slice())
            .then_some(self.code_page.value)
            .flatten();
        let utf8 = |read| TextEncoding {
            code_page: None,
            read,
        };
        let named = name.as_deref().and_then(code_page_encoding);
        let mut fitting = 0;
        if self.utf8 {
            let Some(named) = named else {
                return Ok(utf8(None));
            };
            if self
                .beyond_ascii
                .keys()
                .any(|character| *character >= '\u{800}')
            {
                return Ok(utf8(None));
            }
            let mut foreign = 0;
            for (&character, &count) in &self.beyond_ascii {
                let latin = matches!(character, '\u{C0}'..='\u{24F}' | '\u{1E00}'..='\u{1EFF}')
                    && character.is_alphabetic();
                if latin || named.writes(character) {
                    fitting += count;
                } else {
                    foreign += count;
                }
            }
            if fitting > foreign {
                let doubted = name.as_deref().filter(|_| foreign > 0).map(code_page_name);
                return Ok(utf8(
                    doubted.map(|doubted| ("UTF-8".to_owned(), Some(doubted))),
                ));
            }
        }
        let name = name.ok_or(DxfImportError::NonAscii)?;
        let (code_page, (read, doubted)) =
            text_code_page(&name, &self.high).ok_or(DxfImportError::NonAscii)?;
        let doubted = doubted.or_else(|| (fitting > 0).then(|| code_page_name(&name)));
        Ok(TextEncoding {
            code_page: Some(code_page),
            read: Some((read, doubted)),
        })
    }
}

/// The text of a DXF file as [`TextSeen::decide`] reads it, and what is said of that.
pub(super) fn dxf_source_text(
    source: &[u8],
) -> Result<(Cow<'_, str>, Option<CodePageRead>), DxfImportError> {
    if source.is_empty() {
        return Err(DxfImportError::Empty);
    }
    let mut seen = TextSeen::default();
    for line in source.split(|byte| *byte == b'\n') {
        seen.see(line);
    }
    let encoding = seen.decide()?;
    let source = source
        .strip_prefix(super::BYTE_ORDER_MARK)
        .unwrap_or(source);
    if encoding.code_page.is_none()
        && let Ok(text) = std::str::from_utf8(source)
    {
        return Ok((Cow::Borrowed(text), encoding.read));
    }
    let text = encoding
        .code_page
        .and_then(|code_page| code_page.decode(source))
        .ok_or(DxfImportError::NonAscii)?;
    Ok((text, encoding.read))
}

/// The value of the first header variable of a name, seen line by line.
#[derive(Default)]
struct HeaderValue {
    /// Lines until the value, once the name was seen.
    until: Option<u8>,
    value: Option<Vec<u8>>,
}

impl HeaderValue {
    fn see(&mut self, line: &[u8], name: &[u8]) {
        match self.until {
            None if self.value.is_none() && line == name => self.until = Some(2),
            Some(1) => {
                self.value = Some(line.to_vec());
                self.until = Some(0);
            }
            Some(left) if left > 1 => self.until = Some(left - 1),
            _ => {}
        }
    }
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
