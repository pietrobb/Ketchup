//! One comparison of an object of the underlay with the object of its entity as the file
//! reads now: its layer, lines, texts and fills, with every object it placed (review
//! 2026-11-04 r19 CD-161: only the points of lines were compared, so a text or a fill
//! changed in the file since the import was moved or taken out unseen).

use std::collections::BTreeSet;

use super::place;
use crate::tolerance::{DEFAULT_ANGULAR_TOLERANCE_RAD, DEFAULT_LINEAR_TOLERANCE_MM, ROUNDING};
use crate::underlay::UnderlayLines;
use crate::underlay::fill::UnderlayFill;
use crate::underlay::text::UnderlayText;

/// What an object draws in the file now unlike what it drew when imported.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Difference {
    /// It, or an object it placed, is on another layer.
    Layer,
    /// Its lines lie elsewhere or run otherwise.
    Lines,
    /// A text of it lies elsewhere, reads otherwise or is set otherwise.
    Text,
    /// A fill of it covers another area.
    Fill,
}

/// What one object draws itself: its layer's name, its runs of points, its texts (their
/// style, an index among the drawing's, left out) and its fills' rings.
#[derive(PartialEq)]
struct Drawn<'a> {
    layer: &'a str,
    lines: Vec<&'a [[f64; 2]]>,
    texts: Vec<UnderlayText>,
    fills: Vec<Vec<&'a [[f64; 2]]>>,
}

impl Drawn<'_> {
    fn is_empty(&self) -> bool {
        self.lines.iter().all(|run| run.is_empty())
            && self.texts.is_empty()
            && self.fills.is_empty()
    }
}

/// What object `id` of `lines` draws, and each object it placed, which follow it.
fn drawn(lines: &UnderlayLines, id: usize) -> Vec<Drawn<'_>> {
    let objects = lines.objects();
    let mut placed = BTreeSet::from([id]);
    let mut drawn = Vec::new();
    for (at, object) in objects.objects.iter().enumerate().skip(id) {
        let parent = object
            .parent
            .and_then(|parent| usize::try_from(parent).ok());
        if at != id && !parent.is_some_and(|parent| placed.contains(&parent)) {
            break;
        }
        placed.insert(at);
        let mut own = Drawn {
            layer: objects.layer_name(object),
            lines: Vec::new(),
            texts: Vec::new(),
            fills: Vec::new(),
        };
        for part in &object.parts {
            let Some(layer) = lines.layers().get(place(part.layer)) else {
                continue;
            };
            let range = |range: &std::ops::Range<u32>| place(range.start)..place(range.end);
            own.lines.extend(layer.points_mm().get(range(&part.points)));
            own.texts.extend(
                layer
                    .texts()
                    .get(range(&part.texts))
                    .into_iter()
                    .flatten()
                    .map(|text| UnderlayText {
                        style: 0,
                        ..text.clone()
                    }),
            );
            own.fills.extend(
                layer
                    .fills()
                    .get(range(&part.fills))
                    .into_iter()
                    .flatten()
                    .map(|fill: &UnderlayFill| fill.rings().collect()),
            );
        }
        drawn.push(own);
    }
    drawn
}

/// How object `found` of `now` differs from object `id` of `then`, the first difference
/// found; `None` where it draws alike.
pub(super) fn difference(
    then: &UnderlayLines,
    id: usize,
    now: &UnderlayLines,
    found: usize,
) -> Option<Difference> {
    let (then, now) = (drawn(then, id), drawn(now, found));
    if then.len() != now.len() {
        return Some(Difference::Lines);
    }
    then.iter().zip(&now).find_map(|(then, now)| {
        if then.layer != now.layer {
            Some(Difference::Layer)
        } else if !runs_alike(&then.lines, &now.lines) {
            Some(Difference::Lines)
        } else if !all_alike(&then.texts, &now.texts, texts_alike) {
            Some(Difference::Text)
        } else if !all_alike(&then.fills, &now.fills, |a, b| runs_alike(a, b)) {
            Some(Difference::Fill)
        } else {
            None
        }
    })
}

/// Whether `a` and `b` pair up one by one, each pair `alike`.
fn all_alike<T>(a: &[T], b: &[T], alike: impl Fn(&T, &T) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| alike(a, b))
}

/// Two lengths in millimetres within the model's linear tolerance: a CAD writing a
/// coordinate back with its own rounding (1e-10 mm off) changes nothing (review
/// 2026-11-06 P3: it was refused as a line changed in the file).
fn close_mm(a: f64, b: f64) -> bool {
    (a - b).abs() <= DEFAULT_LINEAR_TOLERANCE_MM
}

fn runs_alike(a: &[&[[f64; 2]]], b: &[&[[f64; 2]]]) -> bool {
    all_alike(a, b, |a, b| {
        all_alike(a, b, |p, q| close_mm(p[0], q[0]) && close_mm(p[1], q[1]))
    })
}

fn texts_alike(a: &UnderlayText, b: &UnderlayText) -> bool {
    a.text == b.text
        && a.horizontal == b.horizontal
        && a.vertical == b.vertical
        && a.fit == b.fit
        && close_mm(a.anchor_mm[0], b.anchor_mm[0])
        && close_mm(a.anchor_mm[1], b.anchor_mm[1])
        && close_mm(a.height_mm, b.height_mm)
        && close_mm(a.wrap_width_mm, b.wrap_width_mm)
        && close_mm(a.line_pitch_mm, b.line_pitch_mm)
        && (a.rotation_rad - b.rotation_rad).abs() <= DEFAULT_ANGULAR_TOLERANCE_RAD
        && (a.width_factor - b.width_factor).abs()
            <= ROUNDING * a.width_factor.abs().max(b.width_factor.abs()).max(1.0)
}

/// Where object `id` of `lines` lies, to the millimetre: the anchor of its first text, or
/// else the first point of its lines, or of its fills; `None` where it draws nothing.
pub(super) fn place_mm(lines: &UnderlayLines, id: usize) -> Option<[i64; 2]> {
    let drawn = drawn(lines, id);
    let first = |pick: fn(&Drawn<'_>) -> Option<[f64; 2]>| drawn.iter().find_map(pick);
    let point = first(|drawn| drawn.texts.first().map(|text| text.anchor_mm))
        .or_else(|| first(|drawn| drawn.lines.iter().find_map(|run| run.first().copied())))
        .or_else(|| {
            first(|drawn| {
                drawn
                    .fills
                    .iter()
                    .flatten()
                    .find_map(|ring| ring.first().copied())
            })
        })?;
    let [x, y] = point.map(millimetre);
    Some([x?, y?])
}

/// `value` rounded to the millimetre, through its decimal text: one too large for an
/// `i64` is none, not saturated as a cast would.
fn millimetre(value: f64) -> Option<i64> {
    format!("{value:.0}").parse().ok()
}

/// Whether object `id` of `lines`, or an object it placed, draws anything.
pub(super) fn draws(lines: &UnderlayLines, id: usize) -> bool {
    drawn(lines, id).iter().any(|drawn| !drawn.is_empty())
}
