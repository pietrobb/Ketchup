//! One reading for every value typed into a field of the window: units (mm,
//! cm, m, feet and inches), arithmetic such as `600+2*18`, and several values
//! separated by `;`, `x` (or `×`, the same letter as typeset) or `:`; `*`
//! multiplies. The decimal separator is the system's, and a
//! point always; a comma never separates values. A bare number is
//! millimetres. A product of two lengths is an area, not a length, and is
//! refused rather than silently read as millimetres.

use std::ops::RangeInclusive;

const MAX_INPUT_CHARS: usize = 256;
const MAX_NESTING: usize = 32;
const MM_PER_INCH: f64 = 25.4;
const MM_PER_FOOT: f64 = 304.8;
pub const VALUE_SEPARATORS: [char; 5] = [';', 'x', 'X', '×', ':'];

/// The decimal separator of the system's number format. A point is read as a
/// decimal separator with either one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecimalSeparator {
    #[default]
    Point,
    Comma,
}

impl DecimalSeparator {
    /// The user's number format, read once when the window starts.
    #[must_use]
    pub fn at_startup() -> Self {
        system_decimal_separator()
    }
}

/// Windows keeps the user's decimal separator in the Region settings.
#[cfg(windows)]
fn system_decimal_separator() -> DecimalSeparator {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let separator: Option<String> = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Control Panel\International")
        .and_then(|key| key.get_value("sDecimal"))
        .ok();
    if separator.is_some_and(|separator| separator.trim() == ",") {
        DecimalSeparator::Comma
    } else {
        DecimalSeparator::Point
    }
}

/// Elsewhere the numeric locale names a language, whose usual separator it is.
#[cfg(not(windows))]
fn system_decimal_separator() -> DecimalSeparator {
    let locale = ["LC_ALL", "LC_NUMERIC", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .or_else(sys_locale::get_locale)
        .unwrap_or_default();
    decimal_separator_of_locale(&locale)
}

/// The usual decimal separator of a locale such as `sk_SK.UTF-8` or `de-CH`.
#[cfg_attr(windows, allow(dead_code))]
fn decimal_separator_of_locale(locale: &str) -> DecimalSeparator {
    const COMMA_LANGUAGES: [&str; 46] = [
        "af", "az", "be", "bg", "bs", "ca", "cs", "da", "de", "el", "es", "et", "eu", "fi", "fo",
        "fr", "gl", "hr", "hu", "hy", "id", "is", "it", "ka", "kk", "ky", "lt", "lv", "mk", "mn",
        "nb", "nl", "nn", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sq", "sr", "sv", "tr", "uk",
        "vi",
    ];
    let mut parts = locale.split(['_', '-', '.', '@']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    let territory = parts.next().unwrap_or_default().to_ascii_uppercase();
    let point_in_territory = territory == "CH" && matches!(language.as_str(), "de" | "it");
    if COMMA_LANGUAGES.contains(&language.as_str()) && !point_in_territory {
        DecimalSeparator::Comma
    } else {
        DecimalSeparator::Point
    }
}

/// Why typed text is not values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Unreadable,
    /// A comma where the decimal separator is a point: values are separated
    /// by `;`, `x` or `:` (`80;60`), so the comma is refused, not guessed.
    Comma,
}

impl Refusal {
    /// The catalog message that tells the user why, in their language.
    #[must_use]
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Unreadable => "value-unreadable",
            Self::Comma => "value-comma-not-decimal",
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unreadable => "not a number, length or expression",
            Self::Comma => "a comma is no decimal separator here; separate values with ; x or :",
        })
    }
}

impl std::error::Error for Refusal {}

/// A typed value with its power of length: 0 for a factor, a count or an
/// angle, 1 for a length in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    pub mm: f64,
    pub power: i8,
}

/// The one reading of every field of the window: the values separated by
/// `;`, `x` or `:`, each one full expression. Every field, its preview and
/// what it does read the text through here.
pub fn read_values(input: &str, decimal: DecimalSeparator) -> Result<Vec<Quantity>, Refusal> {
    if decimal == DecimalSeparator::Point && input.contains(',') {
        return Err(Refusal::Comma);
    }
    if input.chars().count() > MAX_INPUT_CHARS {
        return Err(Refusal::Unreadable);
    }
    input
        .split(VALUE_SEPARATORS)
        .map(|part| evaluate(part, decimal))
        .collect::<Option<Vec<_>>>()
        .ok_or(Refusal::Unreadable)
}

/// The one value of `input` when there is exactly one.
fn read_one(input: &str, decimal: DecimalSeparator) -> Result<Quantity, Refusal> {
    match read_values(input, decimal)?.as_slice() {
        [value] => Ok(*value),
        _ => Err(Refusal::Unreadable),
    }
}

/// Millimetres for a typed length expression, with why it is not one.
pub fn read_length(input: &str, decimal: DecimalSeparator) -> Result<f64, Refusal> {
    read_one(input, decimal).map(|value| value.mm)
}

/// A number without a unit, with why the text is not one.
pub fn read_number(input: &str, decimal: DecimalSeparator) -> Result<f64, Refusal> {
    let value = read_one(input, decimal)?;
    if value.power == 0 {
        Ok(value.mm)
    } else {
        Err(Refusal::Unreadable)
    }
}

/// Millimetres for a typed length expression, or `None` when it is not one.
#[must_use]
pub fn parse_length_mm(input: &str, decimal: DecimalSeparator) -> Option<f64> {
    read_length(input, decimal).ok()
}

fn evaluate(input: &str, decimal: DecimalSeparator) -> Option<Quantity> {
    let mut parser = Parser {
        chars: input.chars().collect(),
        at: 0,
        depth: 0,
        decimal,
    };
    let value = parser.expression()?;
    parser.skip_spaces();
    (parser.at == parser.chars.len() && value.mm.is_finite()).then_some(value)
}

/// Several lengths typed in one field, such as a rectangle's sides or a Move
/// vector: `None` unless every value is a length and their number is in
/// `counts`.
#[must_use]
pub fn parse_length_list(
    input: &str,
    decimal: DecimalSeparator,
    counts: RangeInclusive<usize>,
) -> Option<Vec<f64>> {
    let values = read_values(input, decimal).ok()?;
    counts.contains(&values.len()).then_some(())?;
    Some(values.iter().map(|value| value.mm).collect())
}

/// A number without a unit, as typed in a field that takes none, such as an
/// angle or a count.
#[must_use]
pub fn parse_number(input: &str, decimal: DecimalSeparator) -> Option<f64> {
    read_number(input, decimal).ok()
}

/// The evaluated value of a typed expression, shown next to the field. A
/// plain number needs no preview, so only expressions and unit conversions
/// get one.
#[must_use]
pub fn length_preview_mm(input: &str, decimal: DecimalSeparator) -> Option<f64> {
    if is_plain_mm(input, decimal) {
        return None;
    }
    parse_length_mm(input, decimal)
}

/// Whether the text is a plain number of millimetres, which needs no preview.
/// A point on a comma system is never plain: there it is also the thousands
/// separator, so `1.200` (read as 1.2 mm) is shown before it is taken.
#[must_use]
pub fn is_plain_mm(input: &str, decimal: DecimalSeparator) -> bool {
    if decimal == DecimalSeparator::Comma && input.contains('.') {
        return false;
    }
    let trimmed = input.trim();
    let plain = trimmed
        .strip_suffix("mm")
        .or_else(|| trimmed.strip_suffix("MM"))
        .unwrap_or(trimmed)
        .trim();
    let mut number = Parser {
        chars: plain.chars().collect(),
        at: 0,
        depth: 0,
        decimal,
    };
    !plain.is_empty() && number.number().is_some() && number.at == number.chars.len()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

impl Unit {
    const fn mm(self) -> f64 {
        match self {
            Self::Millimetre => 1.0,
            Self::Centimetre => 10.0,
            Self::Metre => 1000.0,
            Self::Inch => MM_PER_INCH,
            Self::Foot => MM_PER_FOOT,
        }
    }
}

fn digit(chars: &[char], at: usize) -> bool {
    chars.get(at).is_some_and(char::is_ascii_digit)
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
    decimal: DecimalSeparator,
}

impl Parser {
    fn skip_spaces(&mut self) {
        while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_spaces();
        self.chars.get(self.at).copied()
    }

    fn expression(&mut self) -> Option<Quantity> {
        let mut value = self.term()?;
        while let Some(operator @ ('+' | '-')) = self.peek() {
            self.at += 1;
            let right = self.term()?;
            let power = match (value.power, right.power) {
                (left, right) if left == right => left,
                // A plain number added to a length is millimetres: `600+2*18`.
                _ => 1,
            };
            let mm = if operator == '+' {
                value.mm + right.mm
            } else {
                value.mm - right.mm
            };
            value = Quantity { mm, power };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<Quantity> {
        let mut value = self.unary()?;
        while let Some(operator @ ('*' | '/' | '÷')) = self.peek() {
            self.at += 1;
            let right = self.unary()?;
            value = if operator == '*' {
                let power = value.power + right.power;
                (power <= 1).then_some(Quantity {
                    mm: value.mm * right.mm,
                    power,
                })?
            } else {
                let power = value.power - right.power;
                (power >= 0 && right.mm != 0.0).then_some(Quantity {
                    mm: value.mm / right.mm,
                    power,
                })?
            };
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<Quantity> {
        match self.peek()? {
            '-' | '−' => {
                self.at += 1;
                let value = self.unary()?;
                Some(Quantity {
                    mm: -value.mm,
                    ..value
                })
            }
            '+' => {
                self.at += 1;
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<Quantity> {
        if self.peek()? == '(' {
            self.at += 1;
            self.depth += 1;
            if self.depth > MAX_NESTING {
                return None;
            }
            let value = self.expression()?;
            self.depth -= 1;
            if self.peek()? != ')' {
                return None;
            }
            self.at += 1;
            return Some(value);
        }
        let number = self.number()?;
        let Some(unit) = self.unit() else {
            return Some(Quantity {
                mm: number,
                power: 0,
            });
        };
        let mut mm = number * unit.mm();
        // Feet followed straight by inches is one length: 5'6" or 5′ 6″.
        if unit == Unit::Foot {
            let resume = self.at;
            match self.number().zip(self.unit()) {
                Some((inches, Unit::Inch)) => mm += inches * MM_PER_INCH,
                _ => self.at = resume,
            }
        }
        Some(Quantity { mm, power: 1 })
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_spaces();
        let start = self.at;
        while digit(&self.chars, self.at) {
            self.at += 1;
        }
        // A point is always a decimal separator, and a comma between digits
        // where it is the system's, so the Slovak "12,5" reads as "12.5".
        let separator = match self.chars.get(self.at) {
            Some('.') => true,
            Some(',') => self.decimal == DecimalSeparator::Comma,
            _ => false,
        };
        if separator && digit(&self.chars, self.at + 1) {
            self.at += 1;
            while digit(&self.chars, self.at) {
                self.at += 1;
            }
        }
        if self.at == start {
            return None;
        }
        // An exponent, as in the modulus 2.1e5; no unit starts with an e.
        if matches!(self.chars.get(self.at), Some('e' | 'E')) {
            let sign = usize::from(matches!(self.chars.get(self.at + 1), Some('+' | '-')));
            if digit(&self.chars, self.at + 1 + sign) {
                self.at += 1 + sign;
                while digit(&self.chars, self.at) {
                    self.at += 1;
                }
            }
        }
        let text: String = self.chars[start..self.at]
            .iter()
            .map(|&c| if c == ',' { '.' } else { c })
            .collect();
        text.parse().ok()
    }

    /// The unit written next, consuming it.
    fn unit(&mut self) -> Option<Unit> {
        let resume = self.at;
        self.skip_spaces();
        for (name, unit) in [
            ("mm", Unit::Millimetre),
            ("cm", Unit::Centimetre),
            ("ft", Unit::Foot),
            ("in", Unit::Inch),
            ("m", Unit::Metre),
            ("′", Unit::Foot),
            ("'", Unit::Foot),
            ("″", Unit::Inch),
            ("\"", Unit::Inch),
        ] {
            let end = self.at + name.chars().count();
            let matches = self.chars.get(self.at..end).is_some_and(|written| {
                written
                    .iter()
                    .zip(name.chars())
                    .all(|(&c, expected)| c.to_ascii_lowercase() == expected)
            });
            // A letter unit must end the word: "min" is not metres.
            let ends_word = !self
                .chars
                .get(end)
                .is_some_and(|c| c.is_alphabetic() && name.chars().all(char::is_alphabetic));
            if matches && ends_word {
                self.at = end;
                return Some(unit);
            }
        }
        self.at = resume;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DecimalSeparator::{Comma, Point};

    fn mm(input: &str) -> Option<f64> {
        parse_length_mm(input, Comma)
    }

    #[test]
    fn units_convert_to_millimetres() {
        assert_eq!(mm("25"), Some(25.0));
        assert_eq!(mm(" 25 mm "), Some(25.0));
        assert_eq!(mm("2.5cm"), Some(25.0));
        assert_eq!(mm("1,2 m"), Some(1200.0));
        assert_eq!(mm("1M"), Some(1000.0));
        assert_eq!(mm("1\""), Some(25.4));
        assert_eq!(mm("2″"), Some(50.8));
        assert_eq!(mm("1'"), Some(304.8));
        assert_eq!(mm("1 ft"), Some(304.8));
        assert_eq!(mm("5'6\""), Some(5.0 * 304.8 + 6.0 * 25.4));
        assert_eq!(mm("5′ 6″"), Some(5.0 * 304.8 + 6.0 * 25.4));
    }

    #[test]
    fn arithmetic_follows_precedence_and_parentheses() {
        assert_eq!(mm("600+2*18"), Some(636.0));
        assert_eq!(mm("(600+2)*2"), Some(1204.0));
        assert_eq!(mm("1m - 2*18mm"), Some(964.0));
        assert_eq!(mm("2400/3"), Some(800.0));
        assert_eq!(mm("1m/4"), Some(250.0));
        assert_eq!(mm("-25"), Some(-25.0));
        assert_eq!(mm("-(10+5)"), Some(-15.0));
        assert_eq!(mm("12,5+0,5"), Some(13.0));
    }

    #[test]
    fn non_lengths_and_garbage_are_refused() {
        for input in [
            "", " ", "abc", "1m*1m", "2/0", "2/1m", "1,2,3", "12,", "(1+2", "1+", "1 min", "5 5",
            "1e", "1e+", "80;60",
        ] {
            assert_eq!(mm(input), None, "{input:?}");
        }
        assert_eq!(mm(&"(".repeat(40)), None);
        assert_eq!(mm(&"1+".repeat(200)), None);
    }

    /// Operator decision 2026-10-17 (review D-2): the decimal separator is
    /// the system's and a point always; a comma never separates values.
    #[test]
    fn the_decimal_separator_is_the_systems_and_a_point_always() {
        assert_eq!(parse_length_mm("12,5", Comma), Some(12.5));
        assert_eq!(parse_length_mm("12.5", Comma), Some(12.5));
        assert_eq!(parse_length_mm("12.5", Point), Some(12.5));
        assert_eq!(parse_length_mm("2*20,30", Comma), Some(40.6));
        assert_eq!(parse_length_mm("2*20.30", Point), Some(40.6));
        assert_eq!(parse_number("0,5", Comma), Some(0.5));
        assert_eq!(parse_number("90/2", Point), Some(45.0));
        assert_eq!(parse_number("5 mm", Point), None);
    }

    /// A modulus is typed with an exponent, as `str::parse` took it before
    /// the dialog fields moved to this reader.
    #[test]
    fn a_number_may_carry_an_exponent() {
        assert_eq!(read_number("2.1e5", Point), Ok(210_000.0));
        assert_eq!(read_number("2,1e5", Comma), Ok(210_000.0));
        assert_eq!(read_number("2.1E+5", Comma), Ok(210_000.0));
        assert_eq!(read_number("5e-1*4", Point), Ok(2.0));
        assert_eq!(parse_length_mm("1e3 mm", Point), Some(1000.0));
        assert_eq!(
            parse_length_list("1e3;2e2", Point, 2..=2),
            Some(vec![1000.0, 200.0])
        );
    }

    /// Where the decimal separator is a point a comma is refused as such, so
    /// the field can say `80;60` instead of guessing what was meant.
    #[test]
    fn a_comma_where_the_decimal_separator_is_a_point_is_refused_as_a_comma() {
        for input in ["80,60", "12,5", "2*20,30", "1,5;2"] {
            assert_eq!(read_values(input, Point), Err(Refusal::Comma), "{input}");
            assert_eq!(parse_length_mm(input, Point), None, "{input}");
            assert_eq!(parse_length_list(input, Point, 2..=3), None, "{input}");
        }
        assert_eq!(read_values("8O", Point), Err(Refusal::Unreadable));
    }

    #[test]
    fn values_are_separated_by_semicolon_x_or_colon_alike() {
        for decimal in [Comma, Point] {
            let list = |input: &str| parse_length_list(input, decimal, 2..=3);
            for input in [
                "80;60",
                "80x60",
                "80X60",
                "80×60",
                "80:60",
                " 80 ; 60 ",
                "8 cm x 6 cm",
            ] {
                assert_eq!(list(input), Some(vec![80.0, 60.0]), "{input} {decimal:?}");
            }
            assert_eq!(list("2*20;30"), Some(vec![40.0, 30.0]));
            assert_eq!(list("10+5:20:1m"), Some(vec![15.0, 20.0, 1000.0]));
            assert_eq!(list("12.5;0"), Some(vec![12.5, 0.0]));
            for input in ["80", "80*60", "1;2;3;4", "1m*1m;2", "a;b", "80;", ""] {
                assert_eq!(list(input), None, "{input:?} {decimal:?}");
            }
            // Review 2026-10-17 P3-G: `×` is `x`, so `80×60` is never 4 800 mm.
            assert_eq!(parse_length_mm("80×60", decimal), None);
        }
        let list = |input: &str| parse_length_list(input, Comma, 2..=2);
        assert_eq!(list("12,5x3"), Some(vec![12.5, 3.0]));
        assert_eq!(list("2*20,30;12,5"), Some(vec![40.6, 12.5]));
        // One value, not two: on a comma system `80,60` is 80.6 mm.
        assert_eq!(list("80,60"), None);
    }

    #[test]
    fn only_expressions_and_conversions_get_a_preview() {
        assert_eq!(length_preview_mm("25", Point), None);
        assert_eq!(length_preview_mm("25 mm", Point), None);
        assert_eq!(length_preview_mm("12,5", Comma), None);
        assert_eq!(length_preview_mm("600+2*18", Point), Some(636.0));
        assert_eq!(length_preview_mm("2,5 cm", Comma), Some(25.0));
        assert_eq!(length_preview_mm("1m*1m", Point), None);
    }

    /// UI-8 (review 2026-10-17): on a comma system a point is also the
    /// thousands separator, so what `1.200` was read as is always shown.
    #[test]
    fn a_point_on_a_comma_system_is_never_plain() {
        assert_eq!(length_preview_mm("1.200", Comma), Some(1.2));
        assert_eq!(length_preview_mm("1.200 mm", Comma), Some(1.2));
        assert_eq!(length_preview_mm("1.200", Point), None);
        assert!(!is_plain_mm("1.5", Comma));
        assert!(is_plain_mm("1,5", Comma));
    }

    #[test]
    fn a_locale_names_its_usual_decimal_separator() {
        for (locale, decimal) in [
            ("sk_SK.UTF-8", Comma),
            ("de-DE", Comma),
            ("de_CH.UTF-8", Point),
            ("en_US.UTF-8", Point),
            ("C", Point),
            ("", Point),
        ] {
            assert_eq!(decimal_separator_of_locale(locale), decimal, "{locale}");
        }
    }
}
