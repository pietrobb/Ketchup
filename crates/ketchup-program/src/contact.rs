//! Polygonal material shared by opposite coplanar faces.
use crate::eval::TOLERANCE_MM;
use crate::faces::{FaceFrame, FaceKind};
use crate::model::Part;
use ketchup_geometry::linalg::dot;
use ketchup_model::tolerance::ROUNDING;

#[path = "contact_polygon.rs"]
pub(crate) mod polygon;

/// A shared face patch of positive area between two parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub axis: usize,
    pub face_a: String,
    pub face_b: String,
    pub min_mm: [f64; 3],
    pub max_mm: [f64; 3],
    pub normal: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    /// Bounding rectangle, not necessarily contained in the material.
    pub origin_mm: [f64; 3],
    pub size_mm: [f64; 2],
    /// Boundary walk, possibly concave. Holes/disconnected rings are joined
    /// by doubled, zero-area connectors; these are not material edges.
    pub points_mm: Vec<[f64; 3]>,
}

/// Largest total overlap of an opposing planar face pair. Unsupported surfaces
/// are not replaced with their bounding boxes. Curves use the face tessellation.
#[must_use]
pub fn contact(a: &Part, b: &Part) -> Option<Contact> {
    if a.obb().separation(&b.obb()) > TOLERANCE_MM {
        return None;
    }
    contact_with_faces(a, b, &planar_faces(a), &planar_faces(b))
}
fn contact_with_faces(
    a: &Part,
    b: &Part,
    local_faces_a: &[FaceFrame],
    local_faces_b: &[FaceFrame],
) -> Option<Contact> {
    let faces_b: Vec<_> = local_faces_b
        .iter()
        // The cached frames stay part-local.
        .map(|f| b.world_face(f))
        .collect();
    let mut best: Option<(f64, Contact)> = None;
    for local_a in local_faces_a {
        let face_a = a.world_face(local_a);
        for face_b in &faces_b {
            if dot(face_a.normal, face_b.normal) > ROUNDING - 1.0
                || dot(
                    std::array::from_fn(|i| face_b.origin_mm[i] - face_a.origin_mm[i]),
                    face_a.normal,
                )
                .abs()
                    > TOLERANCE_MM
            {
                continue;
            }
            let other = face_b
                .planar_rings()
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|p| face_a.coordinates(face_b.point(*p)))
                        .collect()
                })
                .collect();
            let patch = polygon::boolean(&face_a.planar_rings(), &other, false);
            let area = polygon::area(&patch);
            let points = polygon::walk(&patch);
            let (min, max) = points.iter().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(lo, hi), p| {
                    (
                        std::array::from_fn(|i| lo[i].min(p[i])),
                        std::array::from_fn(|i| hi[i].max(p[i])),
                    )
                },
            );
            let longest = (max[0] - min[0]).max(max[1] - min[1]);
            if longest <= TOLERANCE_MM
                || area <= TOLERANCE_MM * longest
                || best.as_ref().is_some_and(|(previous, _)| *previous >= area)
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
            let points_mm: Vec<_> = points.iter().map(|p| face_a.point(*p)).collect();
            let (min_mm, max_mm) = points_mm.iter().fold(
                ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
                |(lo, hi), p| {
                    (
                        std::array::from_fn(|i| lo[i].min(p[i])),
                        std::array::from_fn(|i| hi[i].max(p[i])),
                    )
                },
            );
            best = Some((
                area,
                Contact {
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
                },
            ));
        }
    }
    best.map(|(_, found)| found)
}

#[derive(Default)]
pub(crate) struct ContactFaces<'a> {
    faces: std::collections::BTreeMap<&'a str, Vec<FaceFrame>>,
}
impl<'a> ContactFaces<'a> {
    pub fn contact(&mut self, a: &'a Part, b: &'a Part) -> Option<Contact> {
        if a.obb().separation(&b.obb()) > TOLERANCE_MM {
            return None;
        }
        for part in [a, b] {
            self.faces
                .entry(&part.name)
                .or_insert_with(|| planar_faces(part));
        }
        contact_with_faces(
            a,
            b,
            &self.faces[a.name.as_str()],
            &self.faces[b.name.as_str()],
        )
    }
}
fn planar_faces(part: &Part) -> Vec<FaceFrame> {
    part.face_frames()
        .into_iter()
        .filter(|face| face.kind == FaceKind::Planar)
        .collect()
}
