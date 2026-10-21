//! Texts of a drawing read as an underlay: TEXT, MTEXT without its formatting codes, the
//! ATTRIBs of an INSERT and a constant ATTDEF, and so the texts of a block and of the
//! block a DIMENSION draws. Whatever of a text is not drawn as its drawing asks (its font,
//! an alignment, a field left unevaluated, mirroring or slant) is counted and named in the
//! report; the text itself is still drawn.

use super::*;
use crate::underlay::text::{
    LINE_PITCH_PER_HEIGHT, TextFit, TextHorizontal, TextVertical, UnderlayText,
};

/// A text and the layer it is drawn on.
#[derive(Clone)]
pub(super) struct OrderedText {
    pub(super) layer: String,
    pub(super) text: UnderlayText,
}

/// The text of a TEXT, ATTRIB, ATTDEF or MTEXT record. `None`, counted, for an attribute
/// that is not shown: an invisible one, or a definition its inserts' ATTRIBs replace.
pub(super) fn parse_text(
    kind: &str,
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedText>, DxfImportError> {
    if kind == "MTEXT" {
        return parse_mtext(record, units, diagnostics).map(Some);
    }
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    let attribute = kind != "TEXT";
    // An attribute's vertical alignment is group 74; its group 73 is a field length.
    let vertical_code = if attribute { 74 } else { 73 };
    let mut accepted = vec![
        1, 5, 7, 8, 10, 20, 30, 11, 21, 31, 40, 41, 50, 51, 71, 72, 73, 100, 210, 220, 230,
    ];
    if attribute {
        accepted.extend([2, 3, 70, 74, 280]);
    }
    report_ignored_groups(kind, record, &accepted, diagnostics)?;
    if attribute {
        let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
        let tag = parse_optional_text(record, 2)?.map(str::to_owned);
        let hidden = if flags & 1 != 0 {
            Some("dxf.attribute-invisible")
        } else if kind == "ATTDEF" && flags & 2 == 0 {
            Some("dxf.attribute-definition-not-drawn")
        } else {
            None
        };
        if let Some(code) = hidden {
            diagnostics.add(ImportDiagnosticSeverity::Info, code, tag, 1)?;
            return Ok(None);
        }
    }
    let scale = units.millimetres_per_unit();
    let raw = parse_optional_text(record, 1)?.ok_or(DxfImportError::MalformedPairs)?;
    report_field(kind, raw, diagnostics)?;
    let height_mm = positive(scale_coordinate(parse_required_number(record, 40)?, scale)?)?;
    let mut text = UnderlayText::new(
        special_characters(raw),
        parse_xy(record, 10, 20, scale)?,
        height_mm,
    );
    text.width_factor = positive(parse_optional_number(record, 41)?.unwrap_or(1.0))?;
    text.rotation_rad = parse_optional_number(record, 50)?
        .unwrap_or(0.0)
        .to_radians();
    let second = match (
        parse_optional_number(record, 11)?,
        parse_optional_number(record, 21)?,
    ) {
        (Some(x), Some(y)) => Some([scale_coordinate(x, scale)?, scale_coordinate(y, scale)?]),
        _ => None,
    };
    justify(
        &mut text,
        kind,
        [
            parse_optional_i64(record, 72)?.unwrap_or(0),
            parse_optional_i64(record, vertical_code)?.unwrap_or(0),
        ],
        second,
        diagnostics,
    )?;
    report_style(kind, record, &[], diagnostics)?;
    if parse_optional_i64(record, 71)?.is_some_and(|flags| flags & 6 != 0) {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.text-mirroring-ignored",
            Some(kind.to_owned()),
            1,
        )?;
    }
    if parse_optional_number(record, 51)?.is_some_and(|angle| angle != 0.0) {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.text-oblique-ignored",
            Some(kind.to_owned()),
            1,
        )?;
    }
    Ok(Some(OrderedText { layer, text }))
}

fn parse_mtext(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedText, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "MTEXT",
        record,
        &[
            1, 3, 5, 7, 8, 10, 20, 30, 11, 21, 31, 40, 41, 42, 43, 44, 46, 50, 71, 72, 73, 100,
            210, 220, 230,
        ],
        diagnostics,
    )?;
    // Long text comes in pieces of group 3 before its last piece, group 1.
    let last = parse_optional_text(record, 1)?.ok_or(DxfImportError::MalformedPairs)?;
    let raw = record
        .iter()
        .filter(|pair| pair.code == 3)
        .map(|pair| pair.value)
        .chain([last])
        .collect::<String>();
    report_field("MTEXT", &raw, diagnostics)?;
    let mut fonts = Vec::new();
    let plain = special_characters(&mtext_plain(&raw, &mut fonts));
    let scale = units.millimetres_per_unit();
    let height_mm = positive(scale_coordinate(parse_required_number(record, 40)?, scale)?)?;
    let mut text = UnderlayText::new(plain, parse_xy(record, 10, 20, scale)?, height_mm);
    // The direction of its baseline, when given, decides over its rotation.
    text.rotation_rad = match (
        parse_optional_number(record, 11)?,
        parse_optional_number(record, 21)?,
    ) {
        (Some(x), Some(y)) if x != 0.0 || y != 0.0 => y.atan2(x),
        _ => parse_optional_number(record, 50)?
            .unwrap_or(0.0)
            .to_radians(),
    };
    text.wrap_width_mm = match parse_optional_number(record, 41)? {
        Some(width) if width > 0.0 => scale_coordinate(width, scale)?,
        _ => 0.0,
    };
    text.line_pitch_mm = height_mm
        * LINE_PITCH_PER_HEIGHT
        * positive(parse_optional_number(record, 44)?.unwrap_or(1.0))?;
    let attachment = parse_optional_i64(record, 71)?.unwrap_or(1);
    if (1..=9).contains(&attachment) {
        let index = usize::try_from(attachment - 1)
            .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
        text.vertical = [
            TextVertical::Top,
            TextVertical::Middle,
            TextVertical::Bottom,
        ][index / 3];
        text.horizontal = [
            TextHorizontal::Left,
            TextHorizontal::Center,
            TextHorizontal::Right,
        ][index % 3];
    } else {
        unsupported_justification(format!("MTEXT 71={attachment}"), diagnostics)?;
        text.vertical = TextVertical::Top;
    }
    let direction = parse_optional_i64(record, 72)?.unwrap_or(1);
    if !matches!(direction, 1 | 5) {
        unsupported_justification(format!("MTEXT 72={direction}"), diagnostics)?;
    }
    report_style("MTEXT", record, &fonts, diagnostics)?;
    Ok(OrderedText { layer, text })
}

/// Places a TEXT or attribute by its groups 72 and 73 (74 for an attribute): left on the
/// baseline at its first point, otherwise at its second, or stretched between both.
fn justify(
    text: &mut UnderlayText,
    kind: &str,
    [horizontal, vertical]: [i64; 2],
    second: Option<[f64; 2]>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    if horizontal == 0 && vertical == 0 {
        return Ok(());
    }
    let vertical_kind = match vertical {
        0 => Some(TextVertical::Baseline),
        1 => Some(TextVertical::Bottom),
        2 => Some(TextVertical::Middle),
        3 => Some(TextVertical::Top),
        _ => None,
    };
    let placed = second.and_then(|second| match (horizontal, vertical_kind) {
        (0..=2, Some(vertical)) => {
            text.anchor_mm = second;
            text.horizontal = match horizontal {
                1 => TextHorizontal::Center,
                2 => TextHorizontal::Right,
                _ => TextHorizontal::Left,
            };
            text.vertical = vertical;
            Some(())
        }
        (4, Some(TextVertical::Baseline)) => {
            text.anchor_mm = second;
            text.horizontal = TextHorizontal::Center;
            text.vertical = TextVertical::Middle;
            Some(())
        }
        (3 | 5, Some(TextVertical::Baseline)) => {
            let span = [0, 1].map(|axis| second[axis] - text.anchor_mm[axis]);
            let width = span[0].hypot(span[1]);
            (width > DXF_GEOMETRY_EPSILON_MM).then(|| {
                text.rotation_rad = span[1].atan2(span[0]);
                text.fit = if horizontal == 3 {
                    TextFit::Scaled(width)
                } else {
                    TextFit::Width(width)
                };
            })
        }
        _ => None,
    });
    if placed.is_none() {
        unsupported_justification(format!("{kind} {horizontal}/{vertical}"), diagnostics)?;
    }
    Ok(())
}

/// A text drawn left on its baseline instead of as its alignment asks.
fn unsupported_justification(
    subject: String,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.text-justification-unsupported",
        Some(subject),
        1,
    )
}

/// Every text is drawn in the program's font: its style's font, and any font the text
/// switches to, is named as substituted.
fn report_style(
    kind: &str,
    record: &[DxfPair<'_>],
    fonts: &[String],
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    let style = parse_optional_text(record, 7)?.unwrap_or("STANDARD");
    for font in std::iter::once(format!("style {style}"))
        .chain(fonts.iter().map(|font| format!("{kind} font {font}")))
    {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.text-font-substituted",
            Some(font),
            1,
        )?;
    }
    Ok(())
}

/// A field (`%<\AcVar …>%`) is drawn as its code: no drawing data evaluates it here.
fn report_field(
    kind: &str,
    raw: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    if raw.contains("%<\\") {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.text-field-not-evaluated",
            Some(kind.to_owned()),
            1,
        )?;
    }
    Ok(())
}

fn positive(value: f64) -> Result<f64, DxfImportError> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(DxfImportError::DegenerateGeometry)
    }
}

/// The text of an MTEXT without its formatting codes: `\P` breaks a line, a stacked
/// fraction `\S1^2;` reads `1/2`, `\U+00B0` is its character, and font, height, color,
/// width, spacing and underline codes and `{}` groups are taken out. The fonts it
/// switches to are added to `fonts`.
fn mtext_plain(raw: &str, fonts: &mut Vec<String>) -> String {
    let mut plain = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let parameter = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        let mut value = String::new();
        for next in chars.by_ref() {
            if next == ';' {
                break;
            }
            value.push(next);
        }
        value
    };
    while let Some(next) = chars.next() {
        match next {
            '{' | '}' => {}
            '^' if chars.peek() == Some(&'J') => {
                chars.next();
                plain.push('\n');
            }
            '^' if chars.peek() == Some(&'I') => {
                chars.next();
                plain.push(' ');
            }
            '\\' => match chars.next() {
                Some('P' | 'X') => plain.push('\n'),
                Some('~') => plain.push(' '),
                Some(literal @ ('\\' | '{' | '}')) => plain.push(literal),
                Some('S') => plain.push_str(&parameter(&mut chars).replace(['^', '#'], "/")),
                Some('U') if chars.peek() == Some(&'+') => {
                    chars.next();
                    let code = chars.by_ref().take(4).collect::<String>();
                    plain.push(
                        u32::from_str_radix(&code, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or(char::REPLACEMENT_CHARACTER),
                    );
                }
                Some('M') if chars.peek() == Some(&'+') => {
                    // A character in a code page by number: drawn as unknown.
                    chars.by_ref().take(6).for_each(drop);
                    plain.push(char::REPLACEMENT_CHARACTER);
                }
                Some('F' | 'f') => {
                    let font = parameter(&mut chars);
                    let name = font.split('|').next().unwrap_or_default().to_owned();
                    if !name.is_empty() && !fonts.contains(&name) {
                        fonts.push(name);
                    }
                }
                Some('A' | 'C' | 'c' | 'H' | 'h' | 'Q' | 'q' | 'T' | 't' | 'W' | 'w' | 'p') => {
                    parameter(&mut chars);
                }
                Some(_) | None => {}
            },
            other => plain.push(other),
        }
    }
    plain
}

/// The special characters of a text: `%%c` ⌀, `%%d` °, `%%p` ±, `%%%` %, `%%nnn` the
/// character of that number; the underline and overline switches `%%u` and `%%o` are
/// taken out.
fn special_characters(raw: &str) -> String {
    if !raw.contains("%%") {
        return raw.to_owned();
    }
    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find("%%") {
        text.push_str(&rest[..at]);
        let code = &rest[at + 2..];
        let digits = code.bytes().take(3).take_while(u8::is_ascii_digit).count();
        let (replacement, used) = match code.chars().next() {
            Some('c' | 'C') => (Some('\u{2300}'), 1),
            Some('d' | 'D') => (Some('°'), 1),
            Some('p' | 'P') => (Some('±'), 1),
            Some('%') => (Some('%'), 1),
            Some('u' | 'U' | 'o' | 'O') => (None, 1),
            _ if digits == 3 => (code[..3].parse::<u32>().ok().and_then(char::from_u32), 3),
            _ => (Some('%'), 0),
        };
        text.extend(replacement);
        if used == 0 {
            text.push('%');
        }
        rest = &code[used..];
    }
    text.push_str(rest);
    text
}

/// The texts of a block placed by an INSERT or a DIMENSION: moved, turned and scaled as
/// its lines are; a text on layer 0 takes the layer of what places it.
pub(super) fn place_texts(
    texts: &[OrderedText],
    layer: &str,
    base_mm: [f64; 2],
    insertion_mm: [f64; 2],
    scale: [f64; 2],
    rotation_sin: f64,
    rotation_cos: f64,
) -> Result<Vec<OrderedText>, DxfImportError> {
    texts
        .iter()
        .map(|ordered| {
            let text = &ordered.text;
            let along = [
                text.rotation_rad.cos() * scale[0],
                text.rotation_rad.sin() * scale[1],
            ];
            let direction = [
                along[0] * rotation_cos - along[1] * rotation_sin,
                along[0] * rotation_sin + along[1] * rotation_cos,
            ];
            let stretch = direction[0].hypot(direction[1]);
            let height = scale[1].abs();
            let fit = match text.fit {
                TextFit::None => TextFit::None,
                TextFit::Width(width) => TextFit::Width(width * stretch),
                TextFit::Scaled(width) => TextFit::Scaled(width * stretch),
            };
            Ok(OrderedText {
                layer: if ordered.layer == "0" {
                    layer.to_owned()
                } else {
                    ordered.layer.clone()
                },
                text: UnderlayText {
                    anchor_mm: place_point(
                        text.anchor_mm,
                        base_mm,
                        insertion_mm,
                        scale,
                        rotation_sin,
                        rotation_cos,
                    )?,
                    height_mm: text.height_mm * height,
                    rotation_rad: direction[1].atan2(direction[0]),
                    width_factor: text.width_factor * stretch / height,
                    fit,
                    wrap_width_mm: text.wrap_width_mm * stretch,
                    line_pitch_mm: text.line_pitch_mm * height,
                    ..text.clone()
                },
            })
        })
        .collect()
}
