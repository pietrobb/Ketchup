//! Underlay texts as lines: every letter in egui's own font, laid out at the text's anchor,
//! height, rotation and alignment in the XY plane. A glyph's outline is made once, in font
//! units; each letter only says where it stands and how large and turned it is, so many
//! texts cost little memory. They are not underlay lines, so nothing snaps to them.

use ab_glyph::{Font as _, FontArc, GlyphId, OutlineCurve};
use eframe::egui::{FontDefinitions, FontFamily};
use ketchup_model::underlay::text::{TextFit, TextHorizontal, TextVertical, UnderlayText};
use std::sync::OnceLock;

/// Straight pieces a quadratic letter curve is drawn with, and a cubic one.
const QUAD_PIECES: u16 = 3;
const CUBIC_PIECES: u16 = 4;

/// The font texts are drawn in, with the heights its letters are measured by.
struct TextFont {
    font: FontArc,
    /// The height of a capital letter, in font units: a text's height is this high.
    cap_units: f64,
    /// How far the descenders reach below the baseline, in font units.
    descent_units: f64,
}

fn text_font() -> &'static TextFont {
    static FONT: OnceLock<TextFont> = OnceLock::new();
    FONT.get_or_init(|| {
        let definitions = FontDefinitions::default();
        let data = definitions
            .families
            .get(&FontFamily::Proportional)
            .and_then(|names| names.first())
            .and_then(|name| definitions.font_data.get(name))
            .expect("egui's default fonts have a proportional font");
        let font = FontArc::try_from_vec(data.font.to_vec()).expect("egui's own font parses");
        let cap_units =
            font.outline(font.glyph_id('H'))
                .map_or(f64::from(font.ascent_unscaled()), |outline| {
                    // ab_glyph's outline bounds are flipped in y; the curves are y-up.
                    f64::from(outline.bounds.min.y.max(outline.bounds.max.y))
                });
        let descent_units = f64::from(-font.descent_unscaled());
        TextFont {
            font,
            cap_units,
            descent_units,
        }
    })
}

/// One letter of a text where the drawing puts it: its glyph's outline, in font units, is
/// drawn from `origin_mm` along `x_axis` and `y_axis`, in millimetres per font unit;
/// `advance` font units along `x_axis` is where the next letter starts.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedLetter {
    pub glyph: u16,
    pub origin_mm: [f64; 2],
    pub x_axis: [f64; 2],
    pub y_axis: [f64; 2],
    pub advance: f64,
}

/// The height of a capital letter in font units: a letter is this many `y_axis` high.
pub(super) fn cap_units() -> f64 {
    text_font().cap_units
}

/// Every letter of `text`, spaces too, in reading order.
pub(super) fn lay_out(text: &UnderlayText, letter: &mut impl FnMut(PlacedLetter)) {
    let font = text_font();
    let mut height_scale = text.height_mm / font.cap_units;
    let mut width_scale = height_scale * text.width_factor;
    let first_line = text.text.lines().next().unwrap_or_default();
    let natural_units = line_units(&font.font, first_line);
    match text.fit {
        TextFit::Width(width) if natural_units > 0.0 => width_scale = width / natural_units,
        TextFit::Scaled(width) if natural_units > 0.0 => {
            width_scale = width / natural_units;
            height_scale = width_scale / text.width_factor;
        }
        _ => {}
    }
    let lines = text
        .text
        .lines()
        .flat_map(|line| wrap(&font.font, line, text.wrap_width_mm / width_scale))
        .collect::<Vec<_>>();
    let height_mm = font.cap_units * height_scale;
    let pitch_mm = text.line_pitch_mm * height_scale / (text.height_mm / font.cap_units);
    let below_first = lines.iter().skip(1).map(|_| pitch_mm).sum::<f64>();
    let first_baseline = match text.vertical {
        TextVertical::Baseline => below_first,
        TextVertical::Bottom => below_first + font.descent_units * height_scale,
        TextVertical::Middle => (height_mm + below_first) / 2.0 - height_mm,
        TextVertical::Top => -height_mm,
    };
    let (sin, cos) = text.rotation_rad.sin_cos();
    let place = |[x, y]: [f64; 2]| {
        [
            text.anchor_mm[0] + x * cos - y * sin,
            text.anchor_mm[1] + x * sin + y * cos,
        ]
    };
    let x_axis = [width_scale * cos, width_scale * sin];
    let y_axis = [-height_scale * sin, height_scale * cos];
    let mut baseline = first_baseline;
    for line in &lines {
        let width_mm = line_units(&font.font, line) * width_scale;
        let start = match text.horizontal {
            TextHorizontal::Left => 0.0,
            TextHorizontal::Center => -width_mm / 2.0,
            TextHorizontal::Right => -width_mm,
        };
        let mut pen = start;
        let mut previous = None;
        for character in line.chars() {
            let glyph = font.font.glyph_id(character);
            if let Some(previous) = previous {
                pen += f64::from(font.font.kern_unscaled(previous, glyph)) * width_scale;
            }
            let advance = f64::from(font.font.h_advance_unscaled(glyph));
            letter(PlacedLetter {
                glyph: glyph.0,
                origin_mm: place([pen, baseline]),
                x_axis,
                y_axis,
                advance,
            });
            pen += advance * width_scale;
            previous = Some(glyph);
        }
        baseline -= pitch_mm;
    }
}

/// Layout-cell centres of visible letters, using the same alignment, wrapping and rotation as rendering.
pub(crate) fn letter_centres(text: &UnderlayText) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    lay_out(text, &mut |letter| {
        if text_font().font.outline(GlyphId(letter.glyph)).is_some() {
            points.push([0, 1].map(|axis| {
                letter.origin_mm[axis]
                    + letter.x_axis[axis] * letter.advance / 2.0
                    + letter.y_axis[axis] * cap_units() / 2.0
            }));
        }
    });
    points
}

/// The outline of a glyph in font units, two points per straight piece; empty for one
/// that draws nothing, such as a space.
pub(super) fn glyph_outline(glyph: u16) -> Vec<[f32; 2]> {
    let mut points = Vec::new();
    if let Some(outline) = text_font().font.outline(GlyphId(glyph)) {
        for curve in &outline.curves {
            push_curve(curve, &mut points);
        }
    }
    points
}

/// The straight pieces of one curve of a letter's outline.
fn push_curve(curve: &OutlineCurve, points: &mut Vec<[f32; 2]>) {
    let mut polyline = |samples: &mut dyn Iterator<Item = ab_glyph::Point>| {
        let mut last = None;
        for point in samples {
            let point = [point.x, point.y];
            if let Some(last) = last {
                points.extend([last, point]);
            }
            last = Some(point);
        }
    };
    match *curve {
        OutlineCurve::Line(from, to) => polyline(&mut [from, to].into_iter()),
        OutlineCurve::Quad(from, control, to) => polyline(&mut (0..=QUAD_PIECES).map(|step| {
            let t = f32::from(step) / f32::from(QUAD_PIECES);
            let u = 1.0 - t;
            blend(&[from, control, to], [u * u, 2.0 * u * t, t * t])
        })),
        OutlineCurve::Cubic(from, first, second, to) => {
            polyline(&mut (0..=CUBIC_PIECES).map(|step| {
                let t = f32::from(step) / f32::from(CUBIC_PIECES);
                let u = 1.0 - t;
                blend(
                    &[from, first, second, to],
                    [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t],
                )
            }));
        }
    }
}

/// The point of a Bézier curve with these control points and Bernstein weights.
fn blend<const N: usize>(controls: &[ab_glyph::Point; N], weights: [f32; N]) -> ab_glyph::Point {
    let mut point = ab_glyph::point(0.0, 0.0);
    for (control, weight) in controls.iter().zip(weights) {
        point.x += control.x * weight;
        point.y += control.y * weight;
    }
    point
}

/// The width of one line of text in font units, its kerning included.
fn line_units(font: &FontArc, line: &str) -> f64 {
    let mut width = 0.0;
    let mut previous: Option<GlyphId> = None;
    for character in line.chars() {
        let glyph = font.glyph_id(character);
        if let Some(previous) = previous {
            width += f64::from(font.kern_unscaled(previous, glyph));
        }
        width += f64::from(font.h_advance_unscaled(glyph));
        previous = Some(glyph);
    }
    width
}

/// `line` broken at spaces into lines at most `width_units` wide; a word wider than that
/// stays whole on its own line. No wrapping when `width_units` is not positive.
fn wrap<'a>(font: &FontArc, line: &'a str, width_units: f64) -> Vec<&'a str> {
    if width_units <= 0.0 || line_units(font, line) <= width_units {
        return vec![line];
    }
    let mut lines = Vec::new();
    let mut start = 0;
    let mut last_break = None;
    for (at, character) in line.char_indices() {
        if character != ' ' {
            continue;
        }
        if line_units(font, &line[start..at]) > width_units
            && let Some(previous) = last_break
        {
            lines.push(&line[start..previous]);
            start = previous + 1;
        }
        last_break = Some(at);
    }
    if line_units(font, &line[start..]) > width_units
        && let Some(previous) = last_break.filter(|previous| *previous >= start)
    {
        lines.push(&line[start..previous]);
        start = previous + 1;
    }
    lines.push(&line[start..]);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(points: &[[f64; 2]]) -> [[f64; 2]; 2] {
        points.iter().fold(
            [[f64::MAX, f64::MAX], [f64::MIN, f64::MIN]],
            |[low, high], point| {
                [
                    [low[0].min(point[0]), low[1].min(point[1])],
                    [high[0].max(point[0]), high[1].max(point[1])],
                ]
            },
        )
    }

    fn drawn(text: &UnderlayText) -> [[f64; 2]; 2] {
        let mut points = Vec::new();
        lay_out(text, &mut |letter| {
            for [x, y] in glyph_outline(letter.glyph)
                .into_iter()
                .map(|p| p.map(f64::from))
            {
                points.push([0, 1].map(|axis| {
                    letter.origin_mm[axis] + letter.x_axis[axis] * x + letter.y_axis[axis] * y
                }));
            }
        });
        assert!(!points.is_empty() && points.len() % 2 == 0);
        bounds(&points)
    }

    #[test]
    fn a_capital_is_as_high_as_the_text_and_starts_at_its_anchor() {
        let [low, high] = drawn(&UnderlayText::new("H", [100.0, 50.0], 10.0));
        assert!((high[1] - 60.0).abs() < 0.01, "{high:?}");
        assert!((low[1] - 50.0).abs() < 0.01, "{low:?}");
        assert!(low[0] >= 100.0 && low[0] < 103.0, "{low:?}");
    }

    #[test]
    fn alignment_rotation_and_fit_place_the_letters() {
        let mut text = UnderlayText::new("HH", [0.0, 0.0], 10.0);
        let left = drawn(&text);
        text.horizontal = TextHorizontal::Right;
        text.vertical = TextVertical::Top;
        let [low, high] = drawn(&text);
        // Right and top: the letters end at the anchor and hang below it.
        assert!(high[0] <= 0.01 && high[0] > -3.0, "{high:?}");
        assert!(
            (high[1]).abs() < 0.01 && (low[1] + 10.0).abs() < 0.01,
            "{low:?} {high:?}"
        );

        text.horizontal = TextHorizontal::Left;
        text.vertical = TextVertical::Baseline;
        text.rotation_rad = std::f64::consts::FRAC_PI_2;
        let [low, high] = drawn(&text);
        // Turned a quarter: the letters run up +Y and stand to the left of the anchor.
        assert!(
            (low[0] + 10.0).abs() < 0.01 && high[0].abs() < 0.01,
            "{low:?} {high:?}"
        );
        assert!((high[1] - low[1] - (left[1][0] - left[0][0])).abs() < 0.01);

        text.rotation_rad = 0.0;
        text.fit = TextFit::Width(100.0);
        let [low, high] = drawn(&text);
        // Stretched to 100 mm: as high as before, about 100 mm wide.
        assert!((high[1] - 10.0).abs() < 0.01, "{high:?}");
        assert!(
            high[0] - low[0] > 80.0 && high[0] <= 100.01,
            "{low:?} {high:?}"
        );
    }

    #[test]
    fn lines_break_at_line_feeds_and_wrap_at_spaces() {
        let mut text = UnderlayText::new("HH\nHH", [0.0, 0.0], 10.0);
        let [low, high] = drawn(&text);
        // Two lines on baselines one pitch apart, the last on the anchor.
        assert!((low[1]).abs() < 0.01, "{low:?}");
        assert!(
            (high[1] - 10.0 - text.line_pitch_mm).abs() < 0.01,
            "{high:?}"
        );
        text.text = "HH HH".to_owned();
        let one_line = drawn(&text);
        text.wrap_width_mm = (one_line[1][0] - one_line[0][0]) * 0.7;
        let [low, high] = drawn(&text);
        assert!(
            (high[1] - 10.0 - text.line_pitch_mm).abs() < 0.01,
            "{low:?} {high:?}"
        );
        assert!(high[0] - low[0] < one_line[1][0] - one_line[0][0]);
    }
}
