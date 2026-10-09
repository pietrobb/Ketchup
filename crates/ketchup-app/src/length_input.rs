//! One length field for every typed distance: units (mm, cm, m, feet and
//! inches), arithmetic such as `600+2*18`, and a decimal comma. A bare number
//! is millimetres. A product of two lengths is an area, not a length, and is
//! refused rather than silently read as millimetres.

const MAX_INPUT_CHARS: usize = 256;
const MAX_NESTING: usize = 32;
const MM_PER_INCH: f64 = 25.4;
const MM_PER_FOOT: f64 = 304.8;

/// Millimetres for a typed length expression, or `None` when it is not one.
#[must_use]
pub fn parse_length_mm(input: &str) -> Option<f64> {
    if input.chars().count() > MAX_INPUT_CHARS {
        return None;
    }
    let mut parser = Parser {
        chars: input.chars().collect(),
        at: 0,
        depth: 0,
    };
    let value = parser.expression()?;
    parser.skip_spaces();
    (parser.at == parser.chars.len() && value.mm.is_finite()).then_some(value.mm)
}

/// The evaluated value of a typed expression, shown next to the field. A
/// plain number needs no preview, so only expressions and unit conversions
/// get one.
#[must_use]
pub fn length_preview_mm(input: &str) -> Option<f64> {
    let trimmed = input.trim();
    let plain = trimmed
        .strip_suffix("mm")
        .or_else(|| trimmed.strip_suffix("MM"))
        .unwrap_or(trimmed)
        .trim();
    if plain.parse::<f64>().is_ok() {
        return None;
    }
    parse_length_mm(input)
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

/// A value with its power of length: 0 for a plain factor, 1 for a length.
#[derive(Clone, Copy)]
struct Quantity {
    mm: f64,
    power: i8,
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
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
        while let Some(operator @ ('*' | '/' | '×' | '÷')) = self.peek() {
            self.at += 1;
            let right = self.unary()?;
            value = if matches!(operator, '*' | '×') {
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
        // A comma is a decimal separator only between digits, so the Slovak
        // "12,5" and the English "12.5" read the same.
        if matches!(self.chars.get(self.at), Some('.' | ',')) && digit(&self.chars, self.at + 1) {
            self.at += 1;
            while digit(&self.chars, self.at) {
                self.at += 1;
            }
        }
        if self.at == start {
            return None;
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

    fn mm(input: &str) -> Option<f64> {
        parse_length_mm(input)
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
        assert_eq!(mm("3×200"), Some(600.0));
    }

    #[test]
    fn non_lengths_and_garbage_are_refused() {
        for input in [
            "", " ", "abc", "1m*1m", "2/0", "2/1m", "1,2,3", "12,", "(1+2", "1+", "1 min", "5 5",
            "1e3",
        ] {
            assert_eq!(mm(input), None, "{input:?}");
        }
        assert_eq!(mm(&"(".repeat(40)), None);
        assert_eq!(mm(&"1+".repeat(200)), None);
    }

    #[test]
    fn only_expressions_and_conversions_get_a_preview() {
        assert_eq!(length_preview_mm("25"), None);
        assert_eq!(length_preview_mm("25 mm"), None);
        assert_eq!(length_preview_mm("600+2*18"), Some(636.0));
        assert_eq!(length_preview_mm("2,5 cm"), Some(25.0));
        assert_eq!(length_preview_mm("1m*1m"), None);
    }
}
