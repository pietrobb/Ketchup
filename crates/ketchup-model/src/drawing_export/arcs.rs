//! Exact circles and arcs for the DXF of a drawing. The tessellated lines of a
//! circle the view faces become one CIRCLE, or one ARC per unbroken run, on the
//! layer of their visibility; centre and radius come from the exact circle, the
//! run ends from where hidden-line removal split the lines. Lines that are not
//! chords of an exact circle stay lines.

use super::LineStyle;
use crate::drawing::{
    OrthographicView, ProjectedCircle, ProjectedVisibleLine, projected_edge_id as edge_id,
};
use crate::tolerance::{ACCUMULATED_ROUNDING, DEFAULT_LINEAR_TOLERANCE_MM};
use ketchup_geometry::linalg::dot2;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{FRAC_PI_4, PI, TAU};

/// Widest angle one tessellated line of a circle may span. Display meshes step
/// a circle by at most 0.35 rad, while a straight edge that only touches a
/// circle at both ends (a square inscribed in it spans 90 degrees) stays a line.
const MAX_CHORD_SPAN_RAD: f64 = FRAC_PI_4;

/// A circle (`span_rad` = TAU) or a counter-clockwise arc, in view millimetres.
pub(super) struct ModelArc {
    pub(super) id: String,
    pub(super) center: [f64; 2],
    pub(super) radius: f64,
    pub(super) start_rad: f64,
    pub(super) span_rad: f64,
    pub(super) style: LineStyle,
}

pub(super) struct ViewArcs {
    pub(super) arcs: Vec<ModelArc>,
    /// Ids of the view lines the arcs replace.
    pub(super) replaced_lines: BTreeSet<String>,
}

pub(super) fn view_arcs(view: &OrthographicView) -> ViewArcs {
    let circles = distinct_circles(&view.circles);
    let mut edges = BTreeMap::<&str, Vec<(usize, &ProjectedVisibleLine)>>::new();
    for (style, lines) in [(0, &view.visible_lines), (1, &view.hidden_lines)] {
        for line in lines {
            edges
                .entry(edge_id(&line.stable_line_id))
                .or_default()
                .push((style, line));
        }
    }
    let mut runs = vec![[Vec::new(), Vec::new()]; circles.len()];
    let mut replaced_lines = BTreeSet::new();
    for pieces in edges.values() {
        let Some(index) = chord_circle(&circles, pieces) else {
            continue;
        };
        let center = circles[index].circle.center_mm;
        for (style, line) in pieces {
            runs[index][*style].push(minor_arc(center, line.start_mm, line.end_mm));
            replaced_lines.insert(line.stable_line_id.clone());
        }
    }
    let mut arcs = Vec::new();
    for (view_circle, styles) in circles.iter().zip(runs) {
        let circle = view_circle.circle;
        for (style, intervals) in [LineStyle::Visible, LineStyle::Hidden]
            .into_iter()
            .zip(styles)
        {
            let gap = tolerance(circle) / circle.radius_mm;
            let merged = merge(intervals, gap);
            let name = match style {
                LineStyle::Hidden => "hidden",
                _ => "visible",
            };
            let full = merged.len() == 1 && merged[0].1 >= TAU;
            for (ordinal, (start_rad, span_rad)) in merged.into_iter().enumerate() {
                arcs.push(ModelArc {
                    id: if full {
                        format!("{}/{name}", circle.stable_circle_id)
                    } else {
                        format!("{}/{name}-arc-{ordinal}", circle.stable_circle_id)
                    },
                    center: circle.center_mm,
                    radius: circle.radius_mm,
                    start_rad,
                    span_rad,
                    style,
                });
            }
        }
    }
    ViewArcs {
        arcs,
        replaced_lines,
    }
}

/// One curve of the view and the parts of it that exact edges cover.
struct ViewCircle<'a> {
    circle: &'a ProjectedCircle,
    /// (start, counter-clockwise span) of every edge on the curve.
    extents: Vec<(f64, f64)>,
}

/// Coincident circles (the two rims of a hole seen along it) are one curve of
/// the view; the first by id names it, and it covers what any of them covers.
fn distinct_circles(circles: &[ProjectedCircle]) -> Vec<ViewCircle<'_>> {
    let mut distinct = Vec::<ViewCircle<'_>>::new();
    for circle in circles {
        let usable = circle.radius_mm.is_finite()
            && circle.radius_mm > 0.0
            && circle.center_mm.iter().all(|value| value.is_finite())
            && circle.start_rad.is_finite()
            && circle.span_rad.is_finite()
            && circle.span_rad > 0.0;
        if !usable {
            continue;
        }
        let extent = (circle.start_rad, circle.span_rad);
        match distinct.iter_mut().find(|known| {
            let tolerance = tolerance(known.circle);
            (known.circle.radius_mm - circle.radius_mm).abs() <= tolerance
                && distance(known.circle.center_mm, circle.center_mm) <= tolerance
        }) {
            Some(known) => known.extents.push(extent),
            None => distinct.push(ViewCircle {
                circle,
                extents: vec![extent],
            }),
        }
    }
    distinct
}

/// Whether the counter-clockwise interval `(start, span)` lies on one of
/// `extents`, within `gap` at either end.
fn covered(extents: &[(f64, f64)], (start, span): (f64, f64), gap: f64) -> bool {
    extents.iter().any(|&(from, width)| {
        width >= TAU || (start - from + gap).rem_euclid(TAU) + span <= width + 2.0 * gap
    })
}

fn tolerance(circle: &ProjectedCircle) -> f64 {
    let magnitude = circle.center_mm[0].abs().max(circle.center_mm[1].abs()) + circle.radius_mm;
    DEFAULT_LINEAR_TOLERANCE_MM + ACCUMULATED_ROUNDING * magnitude.max(1.0)
}

/// The circle whose chord the pieces of one edge form: both ends of the edge
/// on the circle, every piece on the chord, the chord no wider than one step
/// and on the part of the circle an exact edge covers (a flat whose ends lie on
/// the circle is no chord of it).
fn chord_circle(
    circles: &[ViewCircle<'_>],
    pieces: &[(usize, &ProjectedVisibleLine)],
) -> Option<usize> {
    let (_, first) = pieces.first()?;
    let direction = subtract(first.end_mm, first.start_mm);
    let length = direction[0].hypot(direction[1]);
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let direction = [direction[0] / length, direction[1] / length];
    let points = pieces
        .iter()
        .flat_map(|(_, line)| [line.start_mm, line.end_mm])
        .collect::<Vec<_>>();
    let along = |point: [f64; 2]| dot2(subtract(point, first.start_mm), direction);
    let start = *points
        .iter()
        .min_by(|left, right| along(**left).total_cmp(&along(**right)))?;
    let end = *points
        .iter()
        .max_by(|left, right| along(**left).total_cmp(&along(**right)))?;
    circles.iter().position(|view_circle| {
        let circle = view_circle.circle;
        let tolerance = tolerance(circle);
        let on_circle =
            |point| (distance(point, circle.center_mm) - circle.radius_mm).abs() <= tolerance;
        let across = |point: [f64; 2]| {
            let offset = subtract(point, first.start_mm);
            (offset[0] * direction[1] - offset[1] * direction[0]).abs()
        };
        let from = subtract(start, circle.center_mm);
        let to = subtract(end, circle.center_mm);
        let span = (from[0] * to[1] - from[1] * to[0])
            .abs()
            .atan2(dot2(from, to));
        on_circle(start)
            && on_circle(end)
            && span > tolerance / circle.radius_mm
            && span <= MAX_CHORD_SPAN_RAD
            && points.iter().all(|point| across(*point) <= tolerance)
            && covered(
                &view_circle.extents,
                minor_arc(circle.center_mm, start, end),
                tolerance / circle.radius_mm,
            )
    })
}

/// The shorter arc between the central projections of a chord's ends, as
/// (start in [0, TAU), counter-clockwise span).
fn minor_arc(center: [f64; 2], start: [f64; 2], end: [f64; 2]) -> (f64, f64) {
    let angle = |point: [f64; 2]| {
        let offset = subtract(point, center);
        offset[1].atan2(offset[0]).rem_euclid(TAU)
    };
    let (from, to) = (angle(start), angle(end));
    let span = (to - from).rem_euclid(TAU);
    if span <= PI {
        (from, span)
    } else {
        (to, TAU - span)
    }
}

/// Unbroken runs of the intervals; a run closing on itself is the full circle
/// (0, TAU).
fn merge(mut intervals: Vec<(f64, f64)>, gap: f64) -> Vec<(f64, f64)> {
    intervals.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut runs = Vec::<(f64, f64)>::new();
    for (start, span) in intervals {
        match runs.last_mut() {
            Some(last) if start <= last.1 + gap => last.1 = last.1.max(start + span),
            _ => runs.push((start, start + span)),
        }
    }
    if runs.len() > 1 {
        let first = runs[0];
        let last = runs.len() - 1;
        if runs[last].1 + gap >= first.0 + TAU {
            runs[last].1 = runs[last].1.max(first.1 + TAU);
            runs.remove(0);
        }
    }
    runs.into_iter()
        .map(|(start, end)| {
            if end - start + gap >= TAU {
                (0.0, TAU)
            } else {
                (start, end - start)
            }
        })
        .collect()
}

fn subtract(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    let delta = subtract(left, right);
    delta[0].hypot(delta[1])
}

#[cfg(test)]
#[path = "arcs_tests.rs"]
mod tests;
