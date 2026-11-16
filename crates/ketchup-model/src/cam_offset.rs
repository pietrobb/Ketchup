//! Closed loops of lines and circular arcs in a plane: offsetting by a cutter
//! radius, intersections, and scanline crossings. Every result is exact
//! line/arc geometry; nothing is sampled into polylines.
use crate::cam::CamPlannerError;
use crate::tolerance::{APPROXIMATION, NEGLIGIBLE, ROUNDING};
use std::f64::consts::TAU;

pub(crate) type Point = [f64; 2];

/// Two points closer than this are the same point.
pub(crate) const LOOP_TOLERANCE_MM: f64 = APPROXIMATION;
const PARALLEL: f64 = NEGLIGIBLE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Segment {
    Line {
        start: Point,
        end: Point,
    },
    Arc {
        start: Point,
        end: Point,
        center: Point,
        ccw: bool,
    },
}

fn refused() -> CamPlannerError {
    CamPlannerError::UnsupportedRegion
}

pub(crate) fn distance(left: Point, right: Point) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

fn sub(left: Point, right: Point) -> Point {
    [left[0] - right[0], left[1] - right[1]]
}

fn cross(left: Point, right: Point) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn dot(left: Point, right: Point) -> f64 {
    left[0] * right[0] + left[1] * right[1]
}

fn unit(vector: Point) -> Point {
    let length = vector[0].hypot(vector[1]);
    [vector[0] / length, vector[1] / length]
}

/// Counter-clockwise angle from `from` to `to` about `center`, in `[0, TAU)`.
fn ccw_angle(center: Point, from: Point, to: Point) -> f64 {
    let start = (from[1] - center[1]).atan2(from[0] - center[0]);
    let end = (to[1] - center[1]).atan2(to[0] - center[0]);
    (end - start).rem_euclid(TAU)
}

impl Segment {
    pub(crate) const fn start(&self) -> Point {
        match self {
            Self::Line { start, .. } | Self::Arc { start, .. } => *start,
        }
    }

    pub(crate) const fn end(&self) -> Point {
        match self {
            Self::Line { end, .. } | Self::Arc { end, .. } => *end,
        }
    }

    fn with_ends(self, start: Point, end: Point) -> Self {
        match self {
            Self::Line { .. } => Self::Line { start, end },
            Self::Arc { center, ccw, .. } => Self::Arc {
                start,
                end,
                center,
                ccw,
            },
        }
    }

    pub(crate) fn reversed(self) -> Self {
        match self {
            Self::Line { start, end } => Self::Line {
                start: end,
                end: start,
            },
            Self::Arc {
                start,
                end,
                center,
                ccw,
            } => Self::Arc {
                start: end,
                end: start,
                center,
                ccw: !ccw,
            },
        }
    }

    /// Angle the arc turns through, in `(0, TAU]`; a closed arc turns a full circle.
    fn sweep(&self) -> f64 {
        match self {
            Self::Line { .. } => 0.0,
            Self::Arc {
                start,
                end,
                center,
                ccw,
            } => {
                let angle = if *ccw {
                    ccw_angle(*center, *start, *end)
                } else {
                    ccw_angle(*center, *end, *start)
                };
                if angle <= LOOP_TOLERANCE_MM / distance(*start, *center) {
                    TAU
                } else {
                    angle
                }
            }
        }
    }

    fn tangent_at(&self, point: Point) -> Point {
        match self {
            Self::Line { start, end } => unit(sub(*end, *start)),
            Self::Arc { center, ccw, .. } => {
                let radial = unit(sub(point, *center));
                if *ccw {
                    [-radial[1], radial[0]]
                } else {
                    [radial[1], -radial[0]]
                }
            }
        }
    }

    pub(crate) fn length(&self) -> f64 {
        match self {
            Self::Line { start, end } => distance(*start, *end),
            Self::Arc { start, center, .. } => distance(*start, *center) * self.sweep(),
        }
    }

    /// Whether `point`, known to lie on this segment's line or circle, lies
    /// within the segment.
    fn spans(&self, point: Point) -> bool {
        self.spans_with(point, LOOP_TOLERANCE_MM)
    }

    /// [`Self::spans`] allowing `slack_mm` past either end.
    fn spans_with(&self, point: Point, slack_mm: f64) -> bool {
        match self {
            Self::Line { start, end } => {
                let along = sub(*end, *start);
                let length = along[0].hypot(along[1]);
                let t = dot(sub(point, *start), along) / (length * length);
                let slack = slack_mm / length;
                (-slack..=1.0 + slack).contains(&t)
            }
            Self::Arc {
                start, center, ccw, ..
            } => {
                let radius = distance(*start, *center);
                let angle = if *ccw {
                    ccw_angle(*center, *start, point)
                } else {
                    ccw_angle(*center, point, *start)
                };
                let slack = slack_mm / radius;
                angle <= self.sweep() + slack || angle >= TAU - slack
            }
        }
    }

    /// Distance from `point` to the nearest point of this segment.
    pub(crate) fn distance_to(&self, point: Point) -> f64 {
        match self {
            Self::Line { start, end } => {
                let along = sub(*end, *start);
                let t = (dot(sub(point, *start), along) / dot(along, along)).clamp(0.0, 1.0);
                distance(point, [start[0] + along[0] * t, start[1] + along[1] * t])
            }
            Self::Arc {
                start, end, center, ..
            } => {
                let radius = distance(*start, *center);
                let from_center = distance(point, *center);
                if from_center > LOOP_TOLERANCE_MM {
                    let foot = [
                        center[0] + (point[0] - center[0]) * radius / from_center,
                        center[1] + (point[1] - center[1]) * radius / from_center,
                    ];
                    if self.spans(foot) {
                        return (from_center - radius).abs();
                    }
                }
                distance(point, *start).min(distance(point, *end))
            }
        }
    }

    /// Bounding box, counting the axis extremes an arc passes through.
    pub(crate) fn bounds(&self) -> [Point; 2] {
        let mut minimum = [
            self.start()[0].min(self.end()[0]),
            self.start()[1].min(self.end()[1]),
        ];
        let mut maximum = [
            self.start()[0].max(self.end()[0]),
            self.start()[1].max(self.end()[1]),
        ];
        if let Self::Arc { start, center, .. } = self {
            let radius = distance(*start, *center);
            for extreme in [
                [center[0] + radius, center[1]],
                [center[0] - radius, center[1]],
                [center[0], center[1] + radius],
                [center[0], center[1] - radius],
            ] {
                if self.spans(extreme) {
                    for axis in 0..2 {
                        minimum[axis] = minimum[axis].min(extreme[axis]);
                        maximum[axis] = maximum[axis].max(extreme[axis]);
                    }
                }
            }
        }
        [minimum, maximum]
    }
}

/// Twice the signed area a closed loop encloses: positive when counter-clockwise.
pub(crate) fn signed_area(segments: &[Segment]) -> f64 {
    segments
        .iter()
        .map(|segment| {
            let chord = cross(segment.start(), segment.end());
            match segment {
                Segment::Line { .. } => chord,
                Segment::Arc {
                    start, center, ccw, ..
                } => {
                    let radius = distance(*start, *center);
                    let sweep = segment.sweep();
                    let bulge = radius * radius * (sweep - sweep.sin());
                    if *ccw { chord + bulge } else { chord - bulge }
                }
            }
        })
        .sum()
}

pub(crate) fn reversed(segments: &[Segment]) -> Vec<Segment> {
    segments
        .iter()
        .rev()
        .map(|segment| segment.reversed())
        .collect()
}

/// The loop oriented counter-clockwise (`ccw`) or clockwise.
pub(crate) fn oriented(segments: &[Segment], ccw: bool) -> Vec<Segment> {
    if (signed_area(segments) > 0.0) == ccw {
        segments.to_vec()
    } else {
        reversed(segments)
    }
}

/// Offsets a closed loop by `distance_mm` to the right of its direction of
/// travel: a counter-clockwise loop grows, a clockwise loop shrinks. Corners
/// the offset opens get an arc of `distance_mm` round the corner, as the cutter
/// rolls round it; corners it closes are trimmed where the offset curves meet.
/// Refuses a loop the offset would turn inside out or make cross itself,
/// which is where the cutter does not fit.
pub(crate) fn offset_right(
    segments: &[Segment],
    distance_mm: f64,
) -> Result<Vec<Segment>, CamPlannerError> {
    let count = segments.len();
    let mut shifted = Vec::with_capacity(count);
    for segment in segments {
        shifted.push(shift_right(*segment, distance_mm)?);
    }
    // Where each shifted segment starts and ends once its corners are joined,
    // and the arc, if any, that rolls round the corner after it.
    let mut starts: Vec<Point> = shifted.iter().map(Segment::start).collect();
    let mut ends: Vec<Point> = shifted.iter().map(Segment::end).collect();
    let mut corner_arcs = vec![None; count];
    for index in 0..count {
        let next = (index + 1) % count;
        let vertex = segments[index].end();
        let incoming = segments[index].tangent_at(vertex);
        let outgoing = segments[next].tangent_at(vertex);
        let turn = cross(incoming, outgoing);
        if distance(ends[index], starts[next]) <= LOOP_TOLERANCE_MM {
            if dot(incoming, outgoing) <= 0.0 {
                return Err(refused());
            }
            starts[next] = ends[index];
        } else if turn > 0.0 {
            corner_arcs[index] = Some(Segment::Arc {
                start: ends[index],
                end: starts[next],
                center: vertex,
                ccw: true,
            });
        } else {
            let meeting = curve_intersections(&shifted[index], &shifted[next])
                .into_iter()
                .min_by(|left, right| {
                    let cost = |point: &Point| {
                        distance(*point, ends[index]) + distance(*point, starts[next])
                    };
                    cost(left).total_cmp(&cost(right))
                })
                .ok_or_else(refused)?;
            ends[index] = meeting;
            starts[next] = meeting;
        }
    }
    let mut offset = Vec::with_capacity(count * 2);
    for index in 0..count {
        let trimmed = shifted[index].with_ends(starts[index], ends[index]);
        if !keeps_direction(&shifted[index], &trimmed) {
            return Err(refused());
        }
        offset.push(trimmed);
        offset.extend(corner_arcs[index]);
    }
    if self_intersects(&offset) {
        return Err(refused());
    }
    Ok(offset)
}

fn shift_right(segment: Segment, distance_mm: f64) -> Result<Segment, CamPlannerError> {
    match segment {
        Segment::Line { start, end } => {
            let direction = unit(sub(end, start));
            let normal = [direction[1] * distance_mm, -direction[0] * distance_mm];
            Ok(Segment::Line {
                start: [start[0] + normal[0], start[1] + normal[1]],
                end: [end[0] + normal[0], end[1] + normal[1]],
            })
        }
        Segment::Arc {
            start,
            end,
            center,
            ccw,
        } => {
            let radius = distance(start, center);
            // Right of a counter-clockwise arc is away from its centre.
            let shifted = if ccw {
                radius + distance_mm
            } else {
                radius - distance_mm
            };
            if shifted <= LOOP_TOLERANCE_MM {
                return Err(refused());
            }
            let scale = |point: Point| {
                [
                    center[0] + (point[0] - center[0]) * shifted / radius,
                    center[1] + (point[1] - center[1]) * shifted / radius,
                ]
            };
            Ok(Segment::Arc {
                start: scale(start),
                end: scale(end),
                center,
                ccw,
            })
        }
    }
}

/// Whether trimming left the segment running the way it ran, with length.
fn keeps_direction(original: &Segment, trimmed: &Segment) -> bool {
    match (original, trimmed) {
        (
            Segment::Line { start, end },
            Segment::Line {
                start: from,
                end: to,
            },
        ) => {
            distance(*from, *to) > LOOP_TOLERANCE_MM
                && dot(sub(*end, *start), sub(*to, *from)) > 0.0
        }
        (Segment::Arc { .. }, Segment::Arc { .. }) => {
            trimmed.length() > LOOP_TOLERANCE_MM
                && trimmed.sweep() <= original.sweep() + LOOP_TOLERANCE_MM
        }
        _ => false,
    }
}

/// Where the full line or circle of `left` meets that of `right`.
fn curve_intersections(left: &Segment, right: &Segment) -> Vec<Point> {
    match (left, right) {
        (
            Segment::Line { start, end },
            Segment::Line {
                start: from,
                end: to,
            },
        ) => {
            let along = sub(*end, *start);
            let other = sub(*to, *from);
            let denominator = cross(along, other);
            if denominator.abs() <= PARALLEL * dot(along, along).sqrt() * dot(other, other).sqrt() {
                return Vec::new();
            }
            let t = cross(sub(*from, *start), other) / denominator;
            vec![[start[0] + along[0] * t, start[1] + along[1] * t]]
        }
        (Segment::Line { start, end }, Segment::Arc { center, .. }) => {
            line_circle(*start, *end, *center, distance(right.start(), *center))
        }
        (Segment::Arc { .. }, Segment::Line { .. }) => curve_intersections(right, left),
        (
            Segment::Arc { start, center, .. },
            Segment::Arc {
                start: other_start,
                center: other_center,
                ..
            },
        ) => circle_circle(
            *center,
            distance(*start, *center),
            *other_center,
            distance(*other_start, *other_center),
        ),
    }
}

fn line_circle(start: Point, end: Point, center: Point, radius: f64) -> Vec<Point> {
    let direction = unit(sub(end, start));
    let to_center = sub(center, start);
    let along = dot(to_center, direction);
    let foot = [
        start[0] + direction[0] * along,
        start[1] + direction[1] * along,
    ];
    let offset = distance(foot, center);
    if offset > radius + LOOP_TOLERANCE_MM {
        return Vec::new();
    }
    let half = (radius * radius - offset * offset).max(0.0).sqrt();
    if half <= LOOP_TOLERANCE_MM {
        return vec![foot];
    }
    vec![
        [foot[0] - direction[0] * half, foot[1] - direction[1] * half],
        [foot[0] + direction[0] * half, foot[1] + direction[1] * half],
    ]
}

fn circle_circle(center: Point, radius: f64, other: Point, other_radius: f64) -> Vec<Point> {
    let gap = distance(center, other);
    if gap <= LOOP_TOLERANCE_MM
        || gap > radius + other_radius + LOOP_TOLERANCE_MM
        || gap < (radius - other_radius).abs() - LOOP_TOLERANCE_MM
    {
        return Vec::new();
    }
    let along = (gap * gap + radius * radius - other_radius * other_radius) / (2.0 * gap);
    let half = (radius * radius - along * along).max(0.0).sqrt();
    let direction = unit(sub(other, center));
    let foot = [
        center[0] + direction[0] * along,
        center[1] + direction[1] * along,
    ];
    if half <= LOOP_TOLERANCE_MM {
        return vec![foot];
    }
    vec![
        [foot[0] - direction[1] * half, foot[1] + direction[0] * half],
        [foot[0] + direction[1] * half, foot[1] - direction[0] * half],
    ]
}

/// Points where two segments meet, ends included. Overlapping collinear lines
/// or coincident arcs report the ends of their overlap.
pub(crate) fn intersections(left: &Segment, right: &Segment) -> Vec<Point> {
    let mut points: Vec<Point> = if same_curve(left, right) {
        [left.start(), left.end(), right.start(), right.end()]
            .into_iter()
            .filter(|point| left.spans(*point) && right.spans(*point))
            .collect()
    } else {
        curve_intersections(left, right)
            .into_iter()
            .filter(|point| left.spans(*point) && right.spans(*point))
            .collect()
    };
    points.dedup_by(|left, right| distance(*left, *right) <= LOOP_TOLERANCE_MM);
    points
}

fn same_curve(left: &Segment, right: &Segment) -> bool {
    match (left, right) {
        (
            Segment::Line { start, end },
            Segment::Line {
                start: from,
                end: to,
            },
        ) => {
            let along = unit(sub(*end, *start));
            cross(along, sub(*from, *start)).abs() <= LOOP_TOLERANCE_MM
                && cross(along, sub(*to, *start)).abs() <= LOOP_TOLERANCE_MM
        }
        (
            Segment::Arc { start, center, .. },
            Segment::Arc {
                start: other_start,
                center: other_center,
                ..
            },
        ) => {
            distance(*center, *other_center) <= LOOP_TOLERANCE_MM
                && (distance(*start, *center) - distance(*other_start, *other_center)).abs()
                    <= LOOP_TOLERANCE_MM
        }
        _ => false,
    }
}

/// Whether a closed loop crosses or touches itself anywhere but where
/// consecutive segments join.
pub(crate) fn self_intersects(segments: &[Segment]) -> bool {
    let count = segments.len();
    (0..count).any(|first| {
        (first + 1..count).any(|second| {
            let joints: Vec<Point> = [
                (second == first + 1).then(|| segments[first].end()),
                (first == 0 && second == count - 1).then(|| segments[second].end()),
            ]
            .into_iter()
            .flatten()
            .collect();
            intersections(&segments[first], &segments[second])
                .into_iter()
                .any(|point| {
                    joints
                        .iter()
                        .all(|joint| distance(point, *joint) > LOOP_TOLERANCE_MM)
                })
        })
    })
}

/// Whether any segment of one loop meets any segment of the other.
pub(crate) fn loops_meet(left: &[Segment], right: &[Segment]) -> bool {
    left.iter().any(|first| {
        right
            .iter()
            .any(|second| !intersections(first, second).is_empty())
    })
}

/// Whether the horizontal line `y` passes a loop vertex or the top or bottom
/// of an arc's circle, where counting crossings is ambiguous.
pub(crate) fn degenerate_lane(loops: &[Vec<Segment>], y: f64) -> bool {
    loops.iter().flatten().any(|segment| {
        let near = |value: f64| (value - y).abs() <= ROUNDING;
        near(segment.start()[1])
            || near(segment.end()[1])
            || matches!(segment, Segment::Arc { start, center, .. }
                if near(center[1] + distance(*start, *center))
                    || near(center[1] - distance(*start, *center)))
    })
}

/// X coordinates where the horizontal line `y` crosses a loop. `y` must not
/// be degenerate for the loop (see [`degenerate_lane`]).
pub(crate) fn lane_crossings(segments: &[Segment], y: f64) -> Vec<f64> {
    let mut crossings = Vec::new();
    for segment in segments {
        match segment {
            Segment::Line { start, end } => {
                if (start[1] - y) * (end[1] - y) < 0.0 {
                    crossings.push(
                        start[0] + (y - start[1]) * (end[0] - start[0]) / (end[1] - start[1]),
                    );
                }
            }
            Segment::Arc { start, center, .. } => {
                let radius = distance(*start, *center);
                let rise = y - center[1];
                if rise.abs() < radius {
                    let half = (radius * radius - rise * rise).sqrt();
                    for x in [center[0] - half, center[0] + half] {
                        // No slack: a lane just past where two arcs meet crosses only one.
                        if segment.spans_with([x, y], 0.0) {
                            crossings.push(x);
                        }
                    }
                }
            }
        }
    }
    crossings
}
