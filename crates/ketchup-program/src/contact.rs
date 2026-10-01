//! Where two parts touch face to face: a flat face of one lying against a
//! flat face of the other, whatever the parts' bodies, operations and frames.

use crate::eval::TOLERANCE_MM;
use crate::faces::{FaceFrame, FaceKind, PANEL_FACES};
use crate::model::Part;
use ketchup_geometry::linalg::dot;
use ketchup_model::tolerance::ROUNDING;

/// A shared face patch of positive area between two parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    /// Local axis of the first part nearest the normal of `face_a`.
    pub axis: usize,
    /// Face of the first part that touches the second (a [`FaceFrame`] name).
    pub face_a: String,
    /// Face of the second part that touches the first.
    pub face_b: String,
    /// World bounds of the patch.
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
    /// World unit normal pointing out of the first part.
    pub normal: [f64; 3],
    /// World directions of `face_a`'s (u, v) axes; the patch's bounding
    /// rectangle along them starts at `origin_mm` and measures `size_mm`.
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub origin_mm: [f64; 3],
    pub size_mm: [f64; 2],
    /// World corners of the convex patch.
    pub points_mm: Vec<[f64; 3]>,
}

/// A half-plane `a·s + b·t <= c` in a face's (s, t) coordinates.
type HalfPlane = ([f64; 2], f64);

/// Finds the largest patch where a flat face of `a` lies against a flat
/// face of `b`: the faces are coplanar with opposite normals, and the patch
/// is where their rectangles overlap inside both parts' boxes. A body
/// without flat or cylindrical faces (a sweep, a loft) touches through the
/// faces of its box.
#[must_use]
pub fn contact(a: &Part, b: &Part) -> Option<Contact> {
    let (box_a, box_b) = (a.obb(), b.obb());
    if box_a.separation(&box_b) > TOLERANCE_MM {
        return None;
    }
    let bounds: Vec<_> = box_a.planes().into_iter().chain(box_b.planes()).collect();
    let faces_b: Vec<_> = planar_faces(b)
        .into_iter()
        .map(|face| b.world_face(&face))
        .collect();
    let mut best: Option<(f64, Contact)> = None;
    for local_a in planar_faces(a) {
        let face_a = a.world_face(&local_a);
        for face_b in &faces_b {
            if dot(face_a.normal, face_b.normal) > ROUNDING - 1.0
                || dot(minus(face_b.origin_mm, face_a.origin_mm), face_a.normal).abs()
                    > TOLERANCE_MM
            {
                continue;
            }
            let limits = rectangle(&face_a, face_b).into_iter().chain(
                bounds
                    .iter()
                    .map(|&(normal, offset)| in_face(&face_a, normal, offset)),
            );
            let patch = limits.fold(corners(&face_a), clip);
            let area = area(&patch);
            let (min, max) = bounds_2d(&patch);
            let longest = (max[0] - min[0]).max(max[1] - min[1]);
            if longest <= TOLERANCE_MM
                || area <= TOLERANCE_MM * longest
                || best.as_ref().is_some_and(|(best, _)| *best >= area)
            {
                continue;
            }
            let axis = (0..3)
                .max_by(|i, j| {
                    local_a.normal[*i]
                        .abs()
                        .total_cmp(&local_a.normal[*j].abs())
                })
                .unwrap_or(2);
            let points_mm: Vec<_> = patch.iter().map(|at| face_a.point(*at)).collect();
            let (min_mm, max_mm) = points_mm.iter().fold(
                ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
                |(low, high), point| {
                    (
                        std::array::from_fn(|i| low[i].min(point[i])),
                        std::array::from_fn(|i| high[i].max(point[i])),
                    )
                },
            );
            let found = Contact {
                axis,
                face_a: face_a.name.clone(),
                face_b: face_b.name.clone(),
                min_mm,
                max_mm,
                normal: face_a.normal,
                u: face_a.u,
                v: face_a.v,
                origin_mm: face_a.point(min),
                size_mm: [max[0] - min[0], max[1] - min[1]],
                points_mm,
            };
            best = Some((area, found));
        }
    }
    best.map(|(_, found)| found)
}

/// The part's flat faces in its own frame; the faces of its box when its
/// body has no faces with frames.
fn planar_faces(part: &Part) -> Vec<FaceFrame> {
    let faces = part.face_frames();
    if faces.is_empty() {
        let (min, max) = part.local_bounds();
        return PANEL_FACES
            .iter()
            .map(|&(name, axis, far)| {
                let (u_axis, v_axis) = match axis {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let unit = |index: usize| -> [f64; 3] {
                    std::array::from_fn(|i| if i == index { 1.0 } else { 0.0 })
                };
                let mut origin = min;
                origin[axis] = if far { max[axis] } else { min[axis] };
                FaceFrame {
                    name: name.to_owned(),
                    kind: FaceKind::Planar,
                    origin_mm: origin,
                    normal: unit(axis).map(|value| if far { value } else { -value }),
                    u: unit(u_axis),
                    v: unit(v_axis),
                    min: [0.0; 2],
                    max: [max[u_axis] - min[u_axis], max[v_axis] - min[v_axis]],
                }
            })
            .collect();
    }
    faces
        .into_iter()
        .filter(|face| face.kind == FaceKind::Planar)
        .collect()
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

/// The world half-space `normal · x <= offset` on the plane of `face`.
fn in_face(face: &FaceFrame, normal: [f64; 3], offset: f64) -> HalfPlane {
    (
        [dot(normal, face.u), dot(normal, face.v)],
        offset - dot(normal, face.origin_mm),
    )
}

/// The rectangle `other` spans, as half-planes on the plane of `face`.
fn rectangle(face: &FaceFrame, other: &FaceFrame) -> [HalfPlane; 4] {
    let along = |direction: [f64; 3], low: f64, high: f64| {
        let start = dot(direction, other.origin_mm);
        [
            in_face(face, direction, start + high),
            in_face(face, direction.map(|value| -value), -(start + low)),
        ]
    };
    let [u_high, u_low] = along(other.u, other.min[0], other.max[0]);
    let [v_high, v_low] = along(other.v, other.min[1], other.max[1]);
    [u_high, u_low, v_high, v_low]
}

/// The rectangle `face` spans, counter-clockwise in its coordinates.
fn corners(face: &FaceFrame) -> Vec<[f64; 2]> {
    let ([s0, t0], [s1, t1]) = (face.min, face.max);
    vec![[s0, t0], [s1, t0], [s1, t1], [s0, t1]]
}

/// A convex polygon cut down to a half-plane (Sutherland–Hodgman); points
/// within [`TOLERANCE_MM`] outside it stay.
fn clip(polygon: Vec<[f64; 2]>, (coefficients, limit): HalfPlane) -> Vec<[f64; 2]> {
    let value = |point: [f64; 2]| ketchup_geometry::linalg::dot2(coefficients, point);
    let inside = |point: [f64; 2]| value(point) <= limit + TOLERANCE_MM;
    let mut clipped = Vec::with_capacity(polygon.len() + 1);
    for (index, &current) in polygon.iter().enumerate() {
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        if inside(current) != inside(previous) {
            let t =
                ((limit - value(previous)) / (value(current) - value(previous))).clamp(0.0, 1.0);
            clipped.push(std::array::from_fn(|i| {
                previous[i] + (current[i] - previous[i]) * t
            }));
        }
        if inside(current) {
            clipped.push(current);
        }
    }
    clipped
}

fn bounds_2d(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    points.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(min, max), point| {
            (
                std::array::from_fn(|i| min[i].min(point[i])),
                std::array::from_fn(|i| max[i].max(point[i])),
            )
        },
    )
}

fn area(points: &[[f64; 2]]) -> f64 {
    let twice: f64 = (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum();
    twice.abs() * 0.5
}
