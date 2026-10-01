//! Sweep paths: tangent-continuous chains of lines and circular arcs in a
//! part's frame, and where a profile swept along one reaches.
//!
//! The profile's `(u, v)` plane sits at the path start, square to the start
//! tangent `t`: `u = t × z` (or `t × y` when the path starts along ±z) and
//! `v = u × t`, both normalised. Going horizontally, `u` points to the right
//! of the direction of travel and `v` up. The exact kernel carries this
//! frame along the path without twist: unchanged along lines, turned about
//! the axis of each arc.

use crate::model::{ProgramPathArc, ProgramPathSegment, ProgramProfileSegment};
use ketchup_geometry::linalg::{
    add, circumcenter, cross, dot, length, normalize_within, scale, sub,
};
use ketchup_model::tolerance::{APPROXIMATION, ROUNDING};
use std::f64::consts::TAU;

/// The exact kernel's limits: joins must be tangent to within this angle
/// (radians) and arc geometry must agree to within this many millimetres.
const KERNEL_EPSILON: f64 = ROUNDING;
pub const MAX_PATH_SEGMENTS: usize = 64;

type Vec3 = [f64; 3];

fn unit(a: Vec3) -> Option<Vec3> {
    normalize_within(a, f64::EPSILON)
}

/// `vector` turned by `angle` radians about the unit `axis` (right hand).
fn rotate(vector: Vec3, axis: Vec3, angle: f64) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    add(
        add(scale(vector, cos), scale(cross(axis, vector), sin)),
        scale(axis, dot(axis, vector) * (1.0 - cos)),
    )
}

impl ProgramPathSegment {
    #[must_use]
    pub fn line(start_mm: Vec3, end_mm: Vec3) -> Self {
        Self {
            start_mm,
            end_mm,
            arc: None,
        }
    }

    /// Turn of an arc in radians, in `(0, 2π)`.
    fn angle(&self) -> f64 {
        let Some(arc) = self.arc else { return 0.0 };
        let (from, to) = (
            sub(self.start_mm, arc.center_mm),
            sub(self.end_mm, arc.center_mm),
        );
        dot(arc.normal, cross(from, to))
            .atan2(dot(from, to))
            .rem_euclid(TAU)
    }

    fn start_tangent(&self) -> Option<Vec3> {
        match self.arc {
            None => unit(sub(self.end_mm, self.start_mm)),
            Some(arc) => unit(cross(arc.normal, sub(self.start_mm, arc.center_mm))),
        }
    }

    fn end_tangent(&self) -> Option<Vec3> {
        match self.arc {
            None => self.start_tangent(),
            Some(arc) => unit(cross(arc.normal, sub(self.end_mm, arc.center_mm))),
        }
    }

    #[must_use]
    pub fn length(&self) -> f64 {
        match self.arc {
            None => length(sub(self.end_mm, self.start_mm)),
            Some(arc) => length(sub(self.start_mm, arc.center_mm)) * self.angle(),
        }
    }
}

/// The arc from `start` to `end` passing through `through`.
pub fn arc_through(start: Vec3, end: Vec3, through: Vec3) -> Result<ProgramPathArc, String> {
    let (a, b) = (sub(through, start), sub(end, start));
    let normal = unit(cross(a, b))
        .filter(|_| length(cross(a, b)) > APPROXIMATION * length(a) * length(b))
        .ok_or("the through point lies on the line from start to end; use a line")?;
    let center_mm = circumcenter(start, through, end)
        .ok_or("the through point lies on the line from start to end; use a line")?;
    Ok(ProgramPathArc {
        center_mm,
        // Start, through, end run counter-clockwise about a × b.
        normal,
    })
}

/// The arc about `center`, counter-clockwise about `normal`.
pub fn arc_about(
    start: Vec3,
    end: Vec3,
    center: Vec3,
    normal: Vec3,
) -> Result<ProgramPathArc, String> {
    let normal = unit(normal).ok_or("the arc normal must be a non-zero direction")?;
    let (from, to) = (sub(start, center), sub(end, center));
    let (r_start, r_end) = (length(from), length(to));
    if (r_start - r_end).abs() > KERNEL_EPSILON.max(ROUNDING * r_start) {
        return Err(format!(
            "start and end must lie equally far from the arc center ({r_start} mm vs {r_end} mm)"
        ));
    }
    for (point, radius) in [("start", from), ("end", to)] {
        let off_plane = dot(radius, normal);
        if off_plane.abs() > KERNEL_EPSILON.max(ROUNDING * r_start) {
            return Err(format!(
                "the arc {point} lies {off_plane} mm off the plane through the center square to the normal"
            ));
        }
    }
    Ok(ProgramPathArc {
        center_mm: center,
        normal,
    })
}

/// Why a point list cannot become a sweep path.
#[derive(Debug, Clone, PartialEq)]
pub enum PolylineError {
    TooFewPoints,
    /// Inner corners left sharp because no bend radius was given.
    SharpCorners {
        corners: usize,
    },
    /// The path turns back on itself at this 1-based point.
    Reverses {
        point: usize,
        at: Vec3,
    },
    /// The roundings at both ends of a side need more than the side's length.
    BendDoesNotFit {
        bend_mm: f64,
        from_point: usize,
        needed_mm: f64,
        side_mm: f64,
    },
}

impl std::fmt::Display for PolylineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooFewPoints => f.write_str("a path needs at least two distinct points"),
            Self::SharpCorners { corners } => write!(
                f,
                "the path turns at {corners} corner(s), but a sweep path must be smooth; pass bend=<radius> to round them"
            ),
            Self::Reverses { point, at } => write!(
                f,
                "the path reverses at point {point} {at:?}; a sweep cannot turn back on itself"
            ),
            Self::BendDoesNotFit {
                bend_mm,
                from_point,
                needed_mm,
                side_mm,
            } => write!(
                f,
                "bend={bend_mm} does not fit between points {from_point} and {}: the roundings need {needed_mm} mm of the {side_mm} mm side",
                from_point + 1
            ),
        }
    }
}

impl std::error::Error for PolylineError {}

/// A polyline through `points`, every inner corner rounded by a tangent arc
/// of radius `bend_mm` (0 keeps corners, which the sweep then rejects).
pub fn polyline(points: &[Vec3], bend_mm: f64) -> Result<Vec<ProgramPathSegment>, PolylineError> {
    let mut points = points.to_vec();
    points.dedup_by(|next, previous| length(sub(*next, *previous)) <= KERNEL_EPSILON);
    if points.len() < 2 {
        return Err(PolylineError::TooFewPoints);
    }
    let direction = |from: Vec3, to: Vec3| unit(sub(to, from)).expect("deduplicated points");
    // Drop inner points that do not turn the path.
    let mut index = 1;
    while index + 1 < points.len() {
        let (a, b) = (
            direction(points[index - 1], points[index]),
            direction(points[index], points[index + 1]),
        );
        if length(cross(a, b)) <= KERNEL_EPSILON && dot(a, b) > 0.0 {
            points.remove(index);
        } else {
            index += 1;
        }
    }
    let corners = points.len() - 2;
    if corners > 0 && bend_mm <= 0.0 {
        return Err(PolylineError::SharpCorners { corners });
    }
    // Tangent length taken from each side of every inner corner.
    let trims = (1..points.len() - 1)
        .map(|index| {
            let (a, b) = (
                direction(points[index - 1], points[index]),
                direction(points[index], points[index + 1]),
            );
            let turn = dot(a, b).clamp(-1.0, 1.0).acos();
            if turn >= std::f64::consts::PI - APPROXIMATION {
                return Err(PolylineError::Reverses {
                    point: index + 1,
                    at: points[index],
                });
            }
            Ok(bend_mm * (turn / 2.0).tan())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let trim = |corner: usize| {
        if corner == 0 || corner > trims.len() {
            0.0
        } else {
            trims[corner - 1]
        }
    };
    let mut segments = Vec::new();
    let mut cursor = points[0];
    for index in 1..points.len() {
        let (from, to) = (points[index - 1], points[index]);
        let span = length(sub(to, from));
        let (before, after) = (trim(index - 1), trim(index));
        if before + after > span + KERNEL_EPSILON {
            return Err(PolylineError::BendDoesNotFit {
                bend_mm,
                from_point: index,
                needed_mm: before + after,
                side_mm: span,
            });
        }
        let a = direction(from, to);
        let line_end = sub(to, scale(a, after));
        if length(sub(line_end, cursor)) > KERNEL_EPSILON {
            segments.push(ProgramPathSegment::line(cursor, line_end));
        }
        cursor = line_end;
        if index + 1 < points.len() {
            let b = direction(to, points[index + 1]);
            let arc_end = add(to, scale(b, after));
            let inward = unit(sub(b, scale(a, dot(a, b)))).expect("a turning corner");
            segments.push(ProgramPathSegment {
                start_mm: cursor,
                end_mm: arc_end,
                arc: Some(ProgramPathArc {
                    center_mm: add(cursor, scale(inward, bend_mm)),
                    normal: unit(cross(a, b)).expect("a turning corner"),
                }),
            });
            cursor = arc_end;
        }
    }
    Ok(segments)
}

/// Checks a path the exact kernel will accept: connected, at most 64
/// non-degenerate pieces, tangent-continuous joins.
pub fn validate(path: &[ProgramPathSegment]) -> Result<(), String> {
    if !(1..=MAX_PATH_SEGMENTS).contains(&path.len()) {
        return Err(format!(
            "a path has 1 to {MAX_PATH_SEGMENTS} segments, got {}",
            path.len()
        ));
    }
    for (index, segment) in path.iter().enumerate() {
        if segment.length() <= APPROXIMATION || segment.start_tangent().is_none() {
            return Err(format!("path segment {} has no length", index + 1));
        }
    }
    for (index, pair) in path.windows(2).enumerate() {
        if pair[0].end_mm != pair[1].start_mm {
            return Err(format!(
                "path segment {} ends at {:?} but segment {} starts at {:?}",
                index + 1,
                pair[0].end_mm,
                index + 2,
                pair[1].start_mm
            ));
        }
        let (out, into) = (
            pair[0].end_tangent().expect("validated"),
            pair[1].start_tangent().expect("validated"),
        );
        if dot(out, into) < 1.0 - KERNEL_EPSILON || length(cross(out, into)) > KERNEL_EPSILON {
            let degrees = length(cross(out, into)).atan2(dot(out, into)).to_degrees();
            return Err(format!(
                "path segments {} and {} meet at a {degrees:.6}° corner at {:?}; a sweep path must be smooth: give the points with bend=<radius>, or make the arc tangent",
                index + 1,
                index + 2,
                pair[0].end_mm
            ));
        }
    }
    Ok(())
}

/// Path start and the profile frame `(u, v)` there.
#[must_use]
pub fn start_frame(path: &[ProgramPathSegment]) -> (Vec3, Vec3, Vec3) {
    let tangent = path[0].start_tangent().expect("validated path");
    let reference = if length(cross(tangent, [0.0, 0.0, 1.0])) <= ROUNDING {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let u = unit(cross(tangent, reference)).expect("reference is not parallel");
    let v = unit(cross(u, tangent)).expect("u is square to the tangent");
    (path[0].start_mm, u, v)
}

/// The largest `direction · p` over the body a closed profile sweeps along
/// `path`. Along a line the frame is fixed, so the ends decide; along an arc
/// every point turns about the arc axis and the largest value over the turn
/// is found by sampling and a golden-section refinement.
#[must_use]
pub fn sweep_support(
    profile: &[ProgramProfileSegment],
    path: &[ProgramPathSegment],
    direction: Vec3,
) -> f64 {
    let profile_support = |u: Vec3, v: Vec3, d: Vec3| {
        profile
            .iter()
            .map(|segment| segment.support([dot(d, u), dot(d, v)]))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    let (_, mut u, mut v) = start_frame(path);
    let mut best = f64::NEG_INFINITY;
    for segment in path {
        let Some(arc) = segment.arc else {
            let across = profile_support(u, v, direction);
            best = best
                .max(dot(direction, segment.start_mm) + across)
                .max(dot(direction, segment.end_mm) + across);
            continue;
        };
        let turn = segment.angle();
        let radius = sub(segment.start_mm, arc.center_mm);
        // Turning the body by φ equals turning the direction by -φ.
        let at = |phi: f64| {
            let d = rotate(direction, arc.normal, -phi);
            dot(direction, arc.center_mm) + dot(d, radius) + profile_support(u, v, d)
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let samples = ((turn / TAU) * 512.0).ceil().max(16.0) as usize;
        let step = turn / samples as f64;
        let (mut best_index, mut best_value) = (0, f64::NEG_INFINITY);
        for index in 0..=samples {
            let value = at(step * index as f64);
            if value > best_value {
                (best_index, best_value) = (index, value);
            }
        }
        let (mut low, mut high) = (
            (step * best_index as f64 - step).max(0.0),
            (step * best_index as f64 + step).min(turn),
        );
        let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
        for _ in 0..80 {
            let (left, right) = (high - ratio * (high - low), low + ratio * (high - low));
            if at(left) < at(right) {
                low = left;
            } else {
                high = right;
            }
        }
        best = best.max(best_value).max(at((low + high) / 2.0));
        u = rotate(u, arc.normal, turn);
        v = rotate(v, arc.normal, turn);
    }
    best
}
