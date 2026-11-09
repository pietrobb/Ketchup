//! One comparison of an object of the underlay with the object of its entity as the file
//! reads now: its layer, lines, texts and fills, with every object it placed (review
//! 2026-11-04 r19 CD-161: only the points of lines were compared, so a text or a fill
//! changed in the file since the import was moved or taken out unseen).

use std::collections::BTreeSet;

use super::place;
use crate::tolerance::{DEFAULT_ANGULAR_TOLERANCE_RAD, ROUNDING, same_coordinate_mm};
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
pub(super) struct Drawn<'a> {
    pub(super) layer: &'a str,
    pub(super) lines: Vec<&'a [[f64; 2]]>,
    pub(super) texts: Vec<UnderlayText>,
    pub(super) fills: Vec<Vec<&'a [[f64; 2]]>>,
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
    drawn_runs(lines, id, false)
}

/// [`drawn`] with its lines parted into their polylines, as an overlay draws them.
pub(super) fn drawn_parts(lines: &UnderlayLines, id: usize) -> Vec<Drawn<'_>> {
    drawn_runs(lines, id, true)
}

/// The polylines of `points`, a range of the points of `layer`, each cut to the range.
fn polylines_in(
    layer: &crate::underlay::LayerLines,
    points: std::ops::Range<usize>,
) -> Vec<&[[f64; 2]]> {
    let mut start = 0;
    let mut runs = Vec::new();
    for polyline in layer.polylines() {
        let end = start + polyline.len();
        let (from, to) = (start.max(points.start), end.min(points.end));
        if from < to {
            runs.push(&layer.points_mm()[from..to]);
        }
        start = end;
    }
    runs
}

fn drawn_runs(lines: &UnderlayLines, id: usize, split: bool) -> Vec<Drawn<'_>> {
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
            if split {
                own.lines.extend(polylines_in(layer, range(&part.points)));
            } else {
                own.lines.extend(layer.points_mm().get(range(&part.points)));
            }
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
/// found; `None` where it draws alike. The points of both lie `offset_mm` from where the
/// file has them.
pub(super) fn difference(
    then: &UnderlayLines,
    id: usize,
    now: &UnderlayLines,
    found: usize,
    offset_mm: [f64; 2],
) -> Option<Difference> {
    difference_between(then, id, offset_mm, now, found, offset_mm)
}

/// [`difference`] of two drawings whose lines were moved apart: `then`'s points lie
/// `then_offset_mm` from where its file has them, `now`'s `now_offset_mm`.
pub(super) fn difference_between(
    then: &UnderlayLines,
    id: usize,
    then_offset_mm: [f64; 2],
    now: &UnderlayLines,
    found: usize,
    now_offset_mm: [f64; 2],
) -> Option<Difference> {
    let (then, now) = (drawn(then, id), drawn(now, found));
    if then.len() != now.len() {
        return Some(Difference::Lines);
    }
    let alike = Alike {
        then_mm: then_offset_mm,
        now_mm: now_offset_mm,
    };
    then.iter().zip(&now).find_map(|(then, now)| {
        if then.layer != now.layer {
            Some(Difference::Layer)
        } else if !alike.runs(&then.lines, &now.lines) {
            Some(Difference::Lines)
        } else if !all_alike(&then.texts, &now.texts, |a, b| alike.texts(a, b)) {
            Some(Difference::Text)
        } else if !all_alike(&then.fills, &now.fills, |a, b| alike.runs(a, b)) {
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

/// Points and lengths in millimetres alike as a drawing's are: a CAD writing a coordinate
/// back with its own rounding (far below a micrometre off) changes nothing (review
/// 2026-11-06 P3: it was refused as a line changed in the file), however far from the
/// origin the file has it (r23 CD-202: a geodetic drawing, 5e9 mm out, was refused).
struct Alike {
    /// Where the file's origin lies against the points compared, of the one first named
    /// and of the other.
    then_mm: [f64; 2],
    now_mm: [f64; 2],
}

impl Alike {
    fn point(&self, p: [f64; 2], q: [f64; 2]) -> bool {
        (0..2).all(|axis| {
            same_coordinate_mm(p[axis] + self.then_mm[axis], q[axis] + self.now_mm[axis])
        })
    }

    fn runs(&self, a: &[&[[f64; 2]]], b: &[&[[f64; 2]]]) -> bool {
        all_alike(a, b, |a, b| all_alike(a, b, |p, q| self.point(*p, *q)))
    }

    fn texts(&self, a: &UnderlayText, b: &UnderlayText) -> bool {
        texts_alike(a, b) && self.point(a.anchor_mm, b.anchor_mm)
    }
}

/// Whether two texts are alike but for where they lie.
fn texts_alike(a: &UnderlayText, b: &UnderlayText) -> bool {
    a.text == b.text
        && a.horizontal == b.horizontal
        && a.vertical == b.vertical
        && a.fit == b.fit
        && same_coordinate_mm(a.height_mm, b.height_mm)
        && same_coordinate_mm(a.wrap_width_mm, b.wrap_width_mm)
        && same_coordinate_mm(a.line_pitch_mm, b.line_pitch_mm)
        && (a.rotation_rad - b.rotation_rad).abs() <= DEFAULT_ANGULAR_TOLERANCE_RAD
        && (a.width_factor - b.width_factor).abs()
            <= ROUNDING * a.width_factor.abs().max(b.width_factor.abs()).max(1.0)
}

/// Where object `id` of `lines` lies, to the millimetre, where the file has it, `offset_mm`
/// from the lines (review 2026-11-06 r23 CD-202: a geodetic drawing's object was said to
/// lie at -877, -877 mm): the anchor of its first text, or else the first point of its
/// lines, or of its fills; `None` where it draws nothing.
pub(super) fn place_mm(lines: &UnderlayLines, id: usize, offset_mm: [f64; 2]) -> Option<[i64; 2]> {
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
    let [x, y] = [point[0] + offset_mm[0], point[1] + offset_mm[1]].map(millimetre);
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
