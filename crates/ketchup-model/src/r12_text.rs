//! A text as an AutoCAD R12 string holds it, the same in every DXF Kečup
//! writes: ASCII, a line break as a space, every other character of the Basic
//! Multilingual Plane as its `\U+XXXX` escape, so the file needs no code page
//! and AutoCAD, ezdxf and acadrust read the same text, and a percent sign as
//! `%%%`, so that it is never read as a control code such as `%%d`. An R12
//! string holds at most [`MAX_STRING_CHARACTERS`] characters once written.

use std::fmt::Write as _;

pub const MAX_STRING_CHARACTERS: usize = 255;

/// One character as an R12 string writes it; `None` for one beyond the Basic
/// Multilingual Plane (an emoji), which no escape holds.
#[must_use]
pub fn character(character: char) -> Option<String> {
    match character {
        '%' => Some("%%%".to_owned()),
        '\r' | '\n' => Some(" ".to_owned()),
        ascii if ascii.is_ascii() => Some(ascii.to_string()),
        other => u16::try_from(u32::from(other)).ok().map(|code| {
            let mut escape = String::with_capacity(7);
            write!(escape, "\\U+{code:04X}").expect("writing to a String cannot fail");
            escape
        }),
    }
}

/// `value` as an R12 string writes it, a character beyond the Basic
/// Multilingual Plane as `?`.
#[must_use]
pub fn lossy(value: &str) -> String {
    value
        .chars()
        .map(|c| character(c).unwrap_or_else(|| "?".to_owned()))
        .collect()
}

/// `value` broken into lines of at most `max_characters` characters that are
/// also R12 strings of at most 255 characters once written (Slovak diacritics
/// take seven characters each): after the last space that fits, or between
/// characters when no space does. The lines join back into `value`.
#[must_use]
pub fn lines(value: &str, max_characters: f64) -> Vec<String> {
    let characters: Vec<char> = value.chars().collect();
    let written = |c: char| character(c).map_or(1, |piece| piece.len());
    let mut lines = Vec::new();
    let mut start = 0;
    while start < characters.len() {
        let mut end = start;
        let mut width = 0;
        let mut count = 0.0;
        while end < characters.len()
            && count + 1.0 <= max_characters
            && width + written(characters[end]) <= MAX_STRING_CHARACTERS
        {
            width += written(characters[end]);
            count += 1.0;
            end += 1;
        }
        if end < characters.len()
            && let Some(space) = characters[start..end].iter().rposition(|c| *c == ' ')
            && space > 0
        {
            end = start + space + 1;
        }
        lines.push(characters[start..end].iter().collect());
        start = end;
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_keeps_its_accents_and_percent_signs_in_an_ascii_string() {
        assert_eq!(
            lossy("Kotva č. 5 – 50 %\n😀"),
            "Kotva \\U+010D. 5 \\U+2013 50 %%% ?"
        );
        assert_eq!(character('😀'), None);
        assert_eq!(lossy("S-05"), "S-05");
    }

    #[test]
    fn a_long_line_breaks_into_r12_strings_counting_what_each_character_writes() {
        // A percent sign writes three characters, so 100 of them need two strings.
        let percents = "%".repeat(100);
        let pieces = lines(&percents, f64::INFINITY);
        assert_eq!(pieces.iter().map(String::len).collect::<Vec<_>>(), [85, 15]);
        assert!(
            pieces
                .iter()
                .all(|piece| lossy(piece).len() <= MAX_STRING_CHARACTERS)
        );
        assert_eq!(pieces.concat(), percents);
    }
}
