//! Sweep paths: tangent-continuous chains of lines, circular arcs and cubic
//! Bezier curves in a part's frame, and where a profile swept along one
//! reaches.
//!
//! The profile's `(u, v)` plane sits at the path start, square to the start
//! tangent `t`: `u = t × z` (or `t × y` when the path starts along ±z) and
//! `v = u × t`, both normalised. Going horizontally, `u` points to the right
//! of the direction of travel and `v` up. The exact kernel carries this
//! frame along the path without twist: unchanged along lines, turned about
//! the axis of each arc, and along a curve rotation-minimising (here
//! followed by double reflection between close samples).
//!
//! A sweep given a fixed `up` instead keeps `v = up` and `u = t × up`
//! (normalised) everywhere, so the profile plane holds `up`: swept along a
//! helix about `up` the profile stays in the axial section.

use crate::model::{ProgramPathArc, ProgramPathSegment, ProgramProfileSegment};
use ketchup_geometry::linalg::{
    CubicBezier, add, circumcenter, cross, dot, length, normalize_within, scale, sub,
};
use ketchup_model::tolerance::{APPROXIMATION, ROUNDING};
use ketchup_tolerance::limits;
use std::f64::consts::TAU;

/// The exact kernel's limits: joins must be tangent to within this angle
/// (radians) and arc geometry must agree to within this many millimetres.
const KERNEL_EPSILON: f64 = ROUNDING;
/// Samples per curve piece for its length and for following the profile
/// frame along it.
const CURVE_SAMPLES: usize = 256;

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
            bezier: None,
        }
    }

    /// A cubic Bezier piece.
    #[must_use]
    pub fn curve(curve: CubicBezier<3>) -> Self {
        let [start_mm, control_1, control_2, end_mm] = curve.points;
        Self {
            start_mm,
            end_mm,
            arc: None,
            bezier: Some([control_1, control_2]),
        }
    }

    fn bezier_curve(&self) -> Option<CubicBezier<3>> {
        self.bezier
            .map(|[first, second]| CubicBezier::new([self.start_mm, first, second, self.end_mm]))
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
        match (self.arc, self.bezier) {
            (Some(arc), _) => unit(cross(arc.normal, sub(self.start_mm, arc.center_mm))),
            (None, Some([first, _])) => unit(sub(first, self.start_mm)),
            (None, None) => unit(sub(self.end_mm, self.start_mm)),
        }
    }

    fn end_tangent(&self) -> Option<Vec3> {
        match (self.arc, self.bezier) {
            (Some(arc), _) => unit(cross(arc.normal, sub(self.end_mm, arc.center_mm))),
            (None, Some([_, second])) => unit(sub(self.end_mm, second)),
            (None, None) => self.start_tangent(),
        }
    }

    /// The point and unit tangent at `s` in `[0, 1]` along the piece.
    fn point_and_tangent(&self, s: f64) -> (Vec3, Option<Vec3>) {
        match (self.arc, self.bezier_curve()) {
            (Some(arc), _) => {
                let radius = rotate(
                    sub(self.start_mm, arc.center_mm),
                    arc.normal,
                    s * self.angle(),
                );
                (add(arc.center_mm, radius), unit(cross(arc.normal, radius)))
            }
            (None, Some(curve)) => (curve.eval(s), unit(curve.derivative(s))),
            (None, None) => (
                add(self.start_mm, scale(sub(self.end_mm, self.start_mm), s)),
                self.start_tangent(),
            ),
        }
    }

    #[must_use]
    pub fn length(&self) -> f64 {
        match (self.arc, self.bezier_curve()) {
            (Some(arc), _) => length(sub(self.start_mm, arc.center_mm)) * self.angle(),
            (None, Some(curve)) => (0..CURVE_SAMPLES)
                .map(|index| {
                    let at = |index: usize| curve.eval(index as f64 / CURVE_SAMPLES as f64);
                    length(sub(at(index + 1), at(index)))
                })
                .sum(),
            (None, None) => length(sub(self.end_mm, self.start_mm)),
        }
    }
}

/// Why arc or path points cannot become an exact sweep path.
#[derive(Debug, Clone, PartialEq)]
pub enum PathError {
    /// The through point of an arc lies on the line from its start to its end.
    ThroughOnLine,
    ZeroArcNormal,
    /// Start and end lie at different distances from the arc center.
    UnequalRadii {
        start_mm: f64,
        end_mm: f64,
    },
    /// An arc end lies off the plane through the center square to the normal.
    OffArcPlane {
        end: ArcEnd,
        off_mm: f64,
    },
    SegmentCount {
        count: usize,
    },
    /// The 1-based `segment` has no length or no direction.
    NoLength {
        segment: usize,
    },
    /// Segment `segment` (1-based) does not end where the next one starts.
    Disconnected {
        segment: usize,
        end: Vec3,
        next_start: Vec3,
    },
    /// Segments `segment` and `segment + 1` meet at a corner.
    Corner {
        segment: usize,
        degrees: f64,
        at: Vec3,
    },
    /// A fixed sweep `up` runs along the 1-based `segment` at `at`.
    UpAlongPath {
        segment: usize,
        up: Vec3,
        at: Vec3,
    },
}

/// Which end of an arc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcEnd {
    Start,
    End,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThroughOnLine => {
                f.write_str("the through point lies on the line from start to end; use a line")
            }
            Self::ZeroArcNormal => f.write_str("the arc normal must be a non-zero direction"),
            Self::UnequalRadii { start_mm, end_mm } => write!(
                f,
                "start and end must lie equally far from the arc center ({start_mm} mm vs {end_mm} mm)"
            ),
            Self::OffArcPlane { end, off_mm } => write!(
                f,
                "the arc {} lies {off_mm} mm off the plane through the center square to the normal",
                match end {
                    ArcEnd::Start => "start",
                    ArcEnd::End => "end",
                }
            ),
            Self::SegmentCount { count } => write!(
                f,
                "a path has 1 to {} segments, got {count}",
                limits::PATH_SEGMENTS
            ),
            Self::NoLength { segment } => write!(f, "path segment {segment} has no length"),
            Self::Disconnected {
                segment,
                end,
                next_start,
            } => write!(
                f,
                "path segment {segment} ends at {end:?} but segment {} starts at {next_start:?}",
                segment + 1
            ),
            Self::Corner {
                segment,
                degrees,
                at,
            } => write!(
                f,
                "path segments {segment} and {} meet at a {degrees:.6}° corner at {at:?}; a sweep path must be smooth: give the points with bend=<radius>, or make the arc tangent",
                segment + 1
            ),
            Self::UpAlongPath { segment, up, at } => write!(
                f,
                "path segment {segment} runs along up {up:?} at {at:?}; up must stay off the path direction"
            ),
        }
    }
}

impl std::error::Error for PathError {}

/// The arc from `start` to `end` passing through `through`.
///
/// # Errors
/// [`PathError::ThroughOnLine`] when the three points are collinear.
pub fn arc_through(start: Vec3, end: Vec3, through: Vec3) -> Result<ProgramPathArc, PathError> {
    let (a, b) = (sub(through, start), sub(end, start));
    let normal = unit(cross(a, b))
        .filter(|_| length(cross(a, b)) > APPROXIMATION * length(a) * length(b))
        .ok_or(PathError::ThroughOnLine)?;
    let center_mm = circumcenter(start, through, end).ok_or(PathError::ThroughOnLine)?;
    Ok(ProgramPathArc {
        center_mm,
        // Start, through, end run counter-clockwise about a × b.
        normal,
    })
}

/// The arc about `center`, counter-clockwise about `normal`.
///
/// # Errors
/// When the normal is zero or start and end do not lie on one circle about
/// `center` square to it.
pub fn arc_about(
    start: Vec3,
    end: Vec3,
    center: Vec3,
    normal: Vec3,
) -> Result<ProgramPathArc, PathError> {
    let normal = unit(normal).ok_or(PathError::ZeroArcNormal)?;
    let (from, to) = (sub(start, center), sub(end, center));
    let (r_start, r_end) = (length(from), length(to));
    if (r_start - r_end).abs() > KERNEL_EPSILON.max(ROUNDING * r_start) {
        return Err(PathError::UnequalRadii {
            start_mm: r_start,
            end_mm: r_end,
        });
    }
    for (end, radius) in [(ArcEnd::Start, from), (ArcEnd::End, to)] {
        let off_mm = dot(radius, normal);
        if off_mm.abs() > KERNEL_EPSILON.max(ROUNDING * r_start) {
            return Err(PathError::OffArcPlane { end, off_mm });
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
                bezier: None,
            });
            cursor = arc_end;
        }
    }
    Ok(segments)
}

/// Checks a path the exact kernel will accept: connected, at most 64
/// non-degenerate pieces, tangent-continuous joins.
///
/// # Errors
/// The first rule the path breaks.
pub fn validate(path: &[ProgramPathSegment]) -> Result<(), PathError> {
    if !(1..=limits::PATH_SEGMENTS).contains(&path.len()) {
        return Err(PathError::SegmentCount { count: path.len() });
    }
    for (index, segment) in path.iter().enumerate() {
        if segment.length() <= APPROXIMATION || segment.start_tangent().is_none() {
            return Err(PathError::NoLength { segment: index + 1 });
        }
    }
    for (index, pair) in path.windows(2).enumerate() {
        if pair[0].end_mm != pair[1].start_mm {
            return Err(PathError::Disconnected {
                segment: index + 1,
                end: pair[0].end_mm,
                next_start: pair[1].start_mm,
            });
        }
        let (out, into) = (
            pair[0].end_tangent().expect("validated"),
            pair[1].start_tangent().expect("validated"),
        );
        if dot(out, into) < 1.0 - KERNEL_EPSILON || length(cross(out, into)) > KERNEL_EPSILON {
            return Err(PathError::Corner {
                segment: index + 1,
                degrees: length(cross(out, into)).atan2(dot(out, into)).to_degrees(),
                at: pair[0].end_mm,
            });
        }
    }
    Ok(())
}

/// Samples per path piece at which a fixed `up` must stay off the tangent.
const UP_SAMPLES: usize = 64;

/// Checks that a fixed sweep `up` (unit) never runs along `path`, where the
/// profile's u = t × up would vanish.
///
/// # Errors
/// [`PathError::UpAlongPath`] at the first sample where it does.
pub fn validate_up(path: &[ProgramPathSegment], up: Vec3) -> Result<(), PathError> {
    for (index, segment) in path.iter().enumerate() {
        for step in 0..=UP_SAMPLES {
            let s = step as f64 / UP_SAMPLES as f64;
            if let (at, Some(tangent)) = segment.point_and_tangent(s)
                && length(cross(tangent, up)) <= KERNEL_EPSILON
            {
                return Err(PathError::UpAlongPath {
                    segment: index + 1,
                    up,
                    at,
                });
            }
        }
    }
    Ok(())
}

/// The profile frame `(u, v)` a fixed `up` gives where the path runs along
/// `tangent`.
fn up_frame(tangent: Vec3, up: Vec3) -> (Vec3, Vec3) {
    (unit(cross(tangent, up)).unwrap_or(up), up)
}

/// Path start and the profile frame `(u, v)` there.
#[must_use]
pub fn start_frame(path: &[ProgramPathSegment], up: Option<Vec3>) -> (Vec3, Vec3, Vec3) {
    let tangent = path[0].start_tangent().expect("validated path");
    if let Some(up) = up {
        let (u, v) = up_frame(tangent, up);
        return (path[0].start_mm, u, v);
    }
    let reference = if length(cross(tangent, [0.0, 0.0, 1.0])) <= ROUNDING {
        [0.0, 1.0, 0.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    let u = unit(cross(tangent, reference)).expect("reference is not parallel");
    let v = unit(cross(u, tangent)).expect("u is square to the tangent");
    (path[0].start_mm, u, v)
}

/// The profile frame `(u, v)` at `frame.0 = curve(t[0])` carried along the
/// curve to `t[1]` without twist: the double reflection of Wang et al.
/// (2008), exact for rotation-minimising frames up to the sample spacing.
#[allow(clippy::similar_names)]
fn carry_frame(
    curve: &CubicBezier<3>,
    (point, u, v): (Vec3, Vec3, Vec3),
    t: [f64; 2],
) -> (Vec3, Vec3, Vec3) {
    let next = curve.eval(t[1]);
    let reflect = |vector: Vec3, normal: Vec3| {
        let square = dot(normal, normal);
        if square <= f64::EPSILON * f64::EPSILON {
            vector
        } else {
            sub(vector, scale(normal, 2.0 * dot(normal, vector) / square))
        }
    };
    let (Some(tangent), Some(next_tangent)) =
        (unit(curve.derivative(t[0])), unit(curve.derivative(t[1])))
    else {
        return (next, u, v);
    };
    let chord = sub(next, point);
    let reflected_tangent = reflect(tangent, chord);
    let second = sub(next_tangent, reflected_tangent);
    let carry = |vector: Vec3| reflect(reflect(vector, chord), second);
    (next, carry(u), carry(v))
}

/// The largest `f` over `[0, 1]`: `samples` even samples, then a
/// golden-section refinement between the neighbours of the best.
fn maximize(f: impl Fn(f64) -> f64, samples: usize) -> f64 {
    let step = 1.0 / samples as f64;
    let (best_index, best_value) = (0..=samples)
        .map(|index| f(step * index as f64))
        .enumerate()
        .fold((0, f64::NEG_INFINITY), |best, (index, value)| {
            if value > best.1 { (index, value) } else { best }
        });
    let (mut low, mut high) = (
        (step * best_index as f64 - step).max(0.0),
        (step * best_index as f64 + step).min(1.0),
    );
    let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
    for _ in 0..60 {
        let (left, right) = (high - ratio * (high - low), low + ratio * (high - low));
        if f(left) < f(right) {
            low = left;
        } else {
            high = right;
        }
    }
    best_value.max(f((low + high) / 2.0))
}

/// The largest `direction · p` over the body a closed profile sweeps along
/// `path`. With a fixed `up` the frame follows the tangent alone, so every
/// piece is sampled and refined. Without, along a line the frame is fixed,
/// so the ends decide; along an arc every point turns about the arc axis,
/// and along a curve the frame is carried sample by sample; the largest
/// value over either is found by sampling and a golden-section refinement.
#[must_use]
pub fn sweep_support(
    profile: &[ProgramProfileSegment],
    path: &[ProgramPathSegment],
    up: Option<Vec3>,
    direction: Vec3,
) -> f64 {
    let profile_support = |u: Vec3, v: Vec3, d: Vec3| {
        profile
            .iter()
            .map(|segment| segment.support([dot(d, u), dot(d, v)]))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    if let Some(up) = up {
        return path
            .iter()
            .map(|segment| {
                maximize(
                    |s| {
                        let (point, tangent) = segment.point_and_tangent(s);
                        let (u, v) = up_frame(tangent.unwrap_or(up), up);
                        dot(direction, point) + profile_support(u, v, direction)
                    },
                    CURVE_SAMPLES,
                )
            })
            .fold(f64::NEG_INFINITY, f64::max);
    }
    let (_, mut u, mut v) = start_frame(path, None);
    let mut best = f64::NEG_INFINITY;
    for segment in path {
        if let Some(curve) = segment.bezier_curve() {
            let reach = |(point, u, v): (Vec3, Vec3, Vec3)| {
                dot(direction, point) + profile_support(u, v, direction)
            };
            // The frame at every sample, carried from the one before it.
            let mut frames = vec![(curve.eval(0.0), u, v)];
            for index in 1..=CURVE_SAMPLES {
                let previous = frames[index - 1];
                let t = [(index - 1) as f64, index as f64].map(|i| i / CURVE_SAMPLES as f64);
                frames.push(carry_frame(&curve, previous, t));
            }
            let (best_index, best_value) = frames
                .iter()
                .map(|frame| reach(*frame))
                .enumerate()
                .fold((0, f64::NEG_INFINITY), |best, (index, value)| {
                    if value > best.1 { (index, value) } else { best }
                });
            // Refine between the samples next to the best one.
            let step = 1.0 / CURVE_SAMPLES as f64;
            let at = |t: f64| {
                let from = ((t / step).floor() as usize).min(CURVE_SAMPLES - 1);
                reach(carry_frame(&curve, frames[from], [from as f64 * step, t]))
            };
            let (mut low, mut high) = (
                (step * best_index as f64 - step).max(0.0),
                (step * best_index as f64 + step).min(1.0),
            );
            let ratio = (5.0_f64.sqrt() - 1.0) / 2.0;
            for _ in 0..60 {
                let (left, right) = (high - ratio * (high - low), low + ratio * (high - low));
                if at(left) < at(right) {
                    low = left;
                } else {
                    high = right;
                }
            }
            best = best.max(best_value).max(at((low + high) / 2.0));
            (_, u, v) = frames[CURVE_SAMPLES];
            continue;
        }
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
