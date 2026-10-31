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
    /// it leaves undefined as; or, in Windows-1252, a word of it reads as Central European
    /// text read in 1252 does ([`HighBytes::misread_words`]). Either way the text was most
    /// likely written in another code page than its header names.
    pub(super) fn doubtful(self, high: &HighBytes) -> bool {
        self.reads_c1(high) || (self.is(encoding_rs::WINDOWS_1252) && high.misread_words > 0)
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

/// Which of the bytes 128 to 255 a text holds, and how many of its words read as Central
/// European text read in Windows-1252.
pub(super) struct HighBytes {
    seen: [bool; 128],
    misread_words: usize,
}

impl Default for HighBytes {
    fn default() -> Self {
        Self {
            seen: [false; 128],
            misread_words: 0,
        }
    }
}

impl HighBytes {
    /// Sees a line of the text.
    pub(super) fn see(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if let Some(at) = byte.checked_sub(128) {
                self.seen[usize::from(at)] = true;
            }
        }
        let words = bytes.split(|byte| !(byte.is_ascii_alphabetic() || *byte >= 128));
        self.misread_words += words.filter(|word| misread_in_1252(word)).count();
    }
}

/// Whether `word`, read in Windows-1252, reads as Central European text written in
/// Windows-1250 does, which no language written in 1252 spells (review 2026-10-26 UL-43:
/// „Příčka“ read „Pøíèka“). 1250's ř, ě, ň and ů are 1252's ø, ì, ò and ù: ø stands
/// beside no accented letter but the Nordic ones and é, and ì, ò and ù end a word
/// (Italian „più“, French „où“) but stand in one only as ě, ň and ů do.
fn misread_in_1252(word: &[u8]) -> bool {
    const NORDIC: &[u8] = b"\xE6\xC6\xE5\xC5\xE4\xC4\xF6\xD6\xFC\xDC\xE9\xC9\xF8\xD8";
    let slashed_o = word.iter().any(|byte| matches!(byte, 0xF8 | 0xD8));
    let beside_slashed_o = word
        .iter()
        .any(|byte| *byte >= 128 && !NORDIC.contains(byte));
    let inner_grave = word
        .iter()
        .rev()
        .skip(1)
        .any(|byte| matches!(byte, 0xEC | 0xF2 | 0xF9 | 0xCC | 0xD2 | 0xD9));
    (slashed_o && beside_slashed_o) || inner_grave
}

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
/// so a file that is not UTF-8 is read in that code page when it is older than 2007 and
/// names one.
pub(super) fn dxf_source_text(
    source: &[u8],
) -> Result<(Cow<'_, str>, Option<CodePageRead>), DxfImportError> {
    if source.is_empty() {
        return Err(DxfImportError::Empty);
    }
    // A UTF-8 byte order mark only says how the rest is written.
    let source = source.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(source);
    if let Ok(text) = std::str::from_utf8(source) {
        return Ok((Cow::Borrowed(text), None));
    }
    let header_value = |variable: &[u8]| {
        let mut lines = source.split(|byte| *byte == b'\n').map(<[u8]>::trim_ascii);
        lines.position(|line| line == variable)?;
        lines.nth(1)
    };
    let name = header_value(b"$ACADVER")
        .is_none_or(|version| version < b"AC1021".as_slice())
        .then(|| header_value(b"$DWGCODEPAGE"))
        .flatten()
        .ok_or(DxfImportError::NonAscii)?;
    let mut high = HighBytes::default();
    for line in source.split(|byte| *byte == b'\n') {
        high.see(line);
    }
    let (code_page, read) = text_code_page(name, &high).ok_or(DxfImportError::NonAscii)?;
    let text = code_page.decode(source).ok_or(DxfImportError::NonAscii)?;
    Ok((text, Some(read)))
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
