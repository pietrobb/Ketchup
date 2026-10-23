//! One catalog message rendered in a single pass: `{ $name }` placeholders and
//! Fluent select expressions that choose the plural form of a count,
//!
//! ```text
//! digest-deleted = Deleted { $count ->
//!         [one] { $count } occurrence
//!        *[other] { $count } occurrences
//!     } · 1 undo step
//! ```
//!
//! A value is inserted as it is and never read again, so a name that happens
//! to contain `{ $path }` stays that text.

use std::collections::BTreeMap;

/// The CLDR plural categories of whole and fractional numbers in one language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PluralRules {
    English,
    Slovak,
}

impl PluralRules {
    fn category(self, value: &str) -> Option<&'static str> {
        if let Ok(whole) = value.parse::<u64>() {
            return Some(match (self, whole) {
                (_, 1) => "one",
                (Self::Slovak, 2..=4) => "few",
                _ => "other",
            });
        }
        value
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(|_| match self {
                Self::English => "other",
                Self::Slovak => "many",
            })
    }
}

enum Element<'a> {
    Text(&'a str),
    Argument(&'a str),
    Select {
        selector: &'a str,
        variants: Vec<(&'a str, Vec<Element<'a>>)>,
        default: usize,
    },
}

/// Why a pattern does not parse.
enum Fault {
    /// It ends inside a select, which a following line may still close.
    Open,
    Malformed,
}

impl Fault {
    /// The fault of a variant's own text. That text has already ended, so a
    /// select it leaves open can no longer be closed by a following line.
    fn inside_variant(self) -> Self {
        match self {
            Self::Open | Self::Malformed => Self::Malformed,
        }
    }
}

/// Whether `pattern` parses: every select has variants, exactly one default
/// and its closing brace.
pub(crate) fn is_well_formed(pattern: &str) -> bool {
    parse(pattern).is_ok()
}

/// Whether `pattern` ends inside a select, so that a line starting with `}`
/// continues it.
pub(crate) fn ends_inside_select(pattern: &str) -> bool {
    matches!(parse(pattern), Err(Fault::Open))
}

pub(crate) fn render(
    pattern: &str,
    arguments: &BTreeMap<&str, String>,
    plurals: PluralRules,
) -> String {
    let Ok(elements) = parse(pattern) else {
        return pattern.to_owned();
    };
    let mut output = String::with_capacity(pattern.len());
    write(&elements, arguments, plurals, &mut output);
    output
}

fn write(
    elements: &[Element<'_>],
    arguments: &BTreeMap<&str, String>,
    plurals: PluralRules,
    output: &mut String,
) {
    for element in elements {
        match element {
            Element::Text(text) => output.push_str(text),
            Element::Argument(name) => match arguments.get(name) {
                Some(value) => output.push_str(value),
                None => {
                    output.push_str("{ $");
                    output.push_str(name);
                    output.push_str(" }");
                }
            },
            Element::Select {
                selector,
                variants,
                default,
            } => {
                let value = arguments.get(selector).map(|value| value.trim());
                let category = value.and_then(|value| plurals.category(value));
                let chosen = variants
                    .iter()
                    .position(|(key, _)| Some(*key) == value)
                    .or_else(|| variants.iter().position(|(key, _)| Some(*key) == category))
                    .unwrap_or(*default);
                write(&variants[chosen].1, arguments, plurals, output);
            }
        }
    }
}

fn parse(pattern: &str) -> Result<Vec<Element<'_>>, Fault> {
    let mut elements = Vec::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            elements.push(Element::Text(&rest[..open]));
        }
        match placeable(&rest[open + 1..]) {
            Placeable::Element(element, after) => {
                elements.push(element);
                rest = after;
            }
            Placeable::PlainBrace => {
                elements.push(Element::Text("{"));
                rest = &rest[open + 1..];
            }
            Placeable::Fault(fault) => return Err(fault),
        }
    }
    if !rest.is_empty() {
        elements.push(Element::Text(rest));
    }
    Ok(elements)
}

enum Placeable<'a> {
    Element(Element<'a>, &'a str),
    PlainBrace,
    Fault(Fault),
}

/// What follows a `{`: a placeholder or select and the text after it, or a
/// brace that is plain text.
fn placeable(input: &str) -> Placeable<'_> {
    let Some(after_dollar) = input.trim_start_matches(' ').strip_prefix('$') else {
        return Placeable::PlainBrace;
    };
    let length = after_dollar
        .find(|character: char| !(character.is_ascii_alphanumeric() || "_-".contains(character)))
        .unwrap_or(after_dollar.len());
    let (name, after_name) = after_dollar.split_at(length);
    let after_name = after_name.trim_start_matches(' ');
    if name.is_empty() {
        return Placeable::PlainBrace;
    }
    if let Some(after) = after_name.strip_prefix('}') {
        return Placeable::Element(Element::Argument(name), after);
    }
    let Some(body) = after_name.strip_prefix("->") else {
        return Placeable::PlainBrace;
    };
    select(name, body).map_or_else(Placeable::Fault, |(element, after)| {
        Placeable::Element(element, after)
    })
}

fn select<'a>(selector: &'a str, mut rest: &'a str) -> Result<(Element<'a>, &'a str), Fault> {
    let mut variants = Vec::new();
    let mut default = None;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Err(Fault::Open);
        }
        if let Some(after) = rest.strip_prefix('}') {
            return Ok((
                Element::Select {
                    selector,
                    default: default.ok_or(Fault::Malformed)?,
                    variants,
                },
                after,
            ));
        }
        if let Some(after) = rest.strip_prefix('*') {
            if default.replace(variants.len()).is_some() {
                return Err(Fault::Malformed);
            }
            rest = after;
        }
        let (key, after_key) = rest
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
            .ok_or(Fault::Malformed)?;
        let key = key.trim();
        if key.is_empty() || key.contains('\n') {
            return Err(Fault::Malformed);
        }
        let end = variant_end(after_key).ok_or(Fault::Open)?;
        let elements = parse(after_key[..end].trim()).map_err(Fault::inside_variant)?;
        variants.push((key, elements));
        rest = &after_key[end..];
    }
}

/// Where a variant's text ends: at the end of its line, or at the brace that
/// closes the select, outside any placeable of its own. `None` when the
/// pattern ends first.
fn variant_end(input: &str) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, character) in input.char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(index),
            '}' => depth -= 1,
            '\n' if depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}
