//! Names a face of a part in the program's own words, from a point on it and
//! its outward normal in the part's frame, so a face or edge picked in the
//! window can be used directly in `on`, `hole`, `push_pull` or `fillet`.
use crate::model::{Part, ProgramPartBody, ProgramProfileSegment};
use ketchup_model::tolerance::{APPROXIMATION, ROUNDING};
use std::f64::consts::TAU;

const AXES: [char; 3] = ['x', 'y', 'z'];

impl Part {
    /// Program name of the face through `point` with outward unit `normal`,
    /// both in the part's own frame: "x-" ... "z+" on a box, "start", "end"
    /// or a segment name on a profile part. `role` is the name the solid's
    /// topology gave the face, used when it names a face of the part. `None`
    /// when the face is not one the program can name (a cut, a hole wall, a
    /// fillet).
    #[must_use]
    pub fn face_at(&self, point: [f64; 3], normal: [f64; 3], role: Option<&str>) -> Option<String> {
        let (min, max) = self.local_bounds();
        let extent = (0..3).map(|i| max[i] - min[i]).fold(1.0, f64::max);
        let on_plane = |value: f64, plane: f64| (value - plane).abs() <= APPROXIMATION * extent;
        let cap = |axis: usize| {
            (normal[axis].abs() > 1.0 - APPROXIMATION).then(|| {
                if normal[axis] > 0.0 {
                    (on_plane(point[axis], max[axis]), "+")
                } else {
                    (on_plane(point[axis], min[axis]), "-")
                }
            })
        };
        let segments = match &self.body {
            ProgramPartBody::Panel => {
                return (0..3).find_map(|axis| match cap(axis) {
                    Some((true, sign)) => Some(format!("{}{sign}", AXES[axis])),
                    _ => None,
                });
            }
            ProgramPartBody::Extrusion { segments, .. }
            | ProgramPartBody::Revolve { segments, .. }
            | ProgramPartBody::Sweep { segments, .. } => segments.as_slice(),
            ProgramPartBody::Loft { sections } => sections
                .first()
                .map_or(&[][..], |section| section.segments.as_slice()),
        };
        if let Some(role) = role {
            let base = role.split('#').next().unwrap_or_default();
            let cut_face = base.split_once('.').is_some_and(|(cut, face)| {
                self.cuts()
                    .any(|c| c.name == cut && c.segments.iter().any(|s| s.name == face))
            });
            if base == "start"
                || base == "end"
                || cut_face
                || segments.iter().any(|s| s.name == base)
            {
                return Some(role.to_owned());
            }
        }
        let tolerance = (0.01 * extent).max(0.1);
        match &self.body {
            ProgramPartBody::Extrusion { .. } => {
                if let Some((true, sign)) = cap(2) {
                    return Some(if sign == "+" { "end" } else { "start" }.to_owned());
                }
                if normal[2].abs() > APPROXIMATION {
                    return None;
                }
                closest(segments, [point[0], point[1]], tolerance)
            }
            ProgramPartBody::Revolve {
                axis_start_mm,
                axis_end_mm,
                ..
            } if axis_start_mm[0].abs() <= f64::EPSILON && axis_end_mm[0].abs() <= f64::EPSILON => {
                closest(segments, [point[0].hypot(point[2]), point[1]], tolerance)
            }
            _ => None,
        }
    }
}

/// Name of the segment nearest `point`, when it lies within `tolerance`.
fn closest(segments: &[ProgramProfileSegment], point: [f64; 2], tolerance: f64) -> Option<String> {
    segments
        .iter()
        .map(|segment| (distance(segment, point), segment))
        .filter(|(distance, _)| *distance <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, segment)| segment.name.clone())
}

fn distance(segment: &ProgramProfileSegment, p: [f64; 2]) -> f64 {
    let [a, b] = [segment.start_mm, segment.end_mm];
    let length = |v: [f64; 2]| v[0].hypot(v[1]);
    let minus = |u: [f64; 2], v: [f64; 2]| [u[0] - v[0], u[1] - v[1]];
    if segment.bezier.is_some() {
        const SAMPLES: u32 = 256;
        return (0..=SAMPLES)
            .map(|index| {
                length(minus(
                    p,
                    segment.bezier_point(f64::from(index) / f64::from(SAMPLES)),
                ))
            })
            .fold(f64::INFINITY, f64::min);
    }
    let Some(arc) = segment.arc else {
        let d = minus(b, a);
        let squared = ketchup_geometry::linalg::dot2(d, d);
        let t = if squared > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        return length(minus(p, [a[0] + t * d[0], a[1] + t * d[1]]));
    };
    let c = arc.center_mm;
    let angle = |q: [f64; 2]| (q[1] - c[1]).atan2(q[0] - c[0]);
    // Counter-clockwise from `from` to `to`; a clockwise arc is the
    // counter-clockwise one from its end back to its start.
    let (from, to) = if arc.clockwise { (b, a) } else { (a, b) };
    let span = (angle(to) - angle(from)).rem_euclid(TAU);
    let span = if span <= ROUNDING { TAU } else { span };
    if (angle(p) - angle(from)).rem_euclid(TAU) <= span {
        (length(minus(p, c)) - length(minus(a, c))).abs()
    } else {
        length(minus(p, a)).min(length(minus(p, b)))
    }
}
