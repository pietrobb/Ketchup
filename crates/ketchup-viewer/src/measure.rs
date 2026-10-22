//! The customer's point-to-point measurement. The pointer snaps to a vertex of
//! the display mesh near it, else to the nearest point of a model edge near it,
//! else to the face under it. A point on a curved face is only as exact as the
//! flat triangles the package stores for that face, and says so.

use crate::{
    camera::Camera,
    model::{Model, indexed_point, source_point},
};
use ketchup_geometry::linalg::{Affine3, Vec3};
use ketchup_view_format::Section;
use num_traits::ToPrimitive;
use std::collections::{HashMap, HashSet};

/// How near the pointer, in screen points, a vertex or an edge catches it.
pub const SNAP_PX: f32 = 12.0;
/// Candidates of one kind checked for being in sight before the next kind is tried.
const SIGHT_CHECKS: usize = 4;

/// What a measured point lies on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapKind {
    Vertex,
    Edge,
    Face,
}

/// A point the pointer snapped to, in source millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub point: [f64; 3],
    pub kind: SnapKind,
    /// It lies on a curved face, which the package stores only as flat triangles.
    pub approximate: bool,
}

/// Two measured points, A first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    pub a: Snap,
    pub b: Snap,
}

impl Measured {
    /// B − A per axis, in mm.
    pub fn delta(&self) -> [f64; 3] {
        std::array::from_fn(|i| self.b.point[i] - self.a.point[i])
    }

    /// The straight distance from A to B, in mm.
    pub fn distance(&self) -> f64 {
        Vec3::from(self.delta()).length()
    }

    /// Either point lies on a curved face.
    pub fn approximate(&self) -> bool {
        self.a.approximate || self.b.approximate
    }
}

/// Where the pointer is in the 3D view and what the view shows there.
pub struct Pointer<'a> {
    pub ndc: [f32; 2],
    /// The view's width and height in screen points.
    pub size: [f32; 2],
    pub visible: &'a [bool],
    pub section: Option<&'a Section>,
}

/// The Measure mode: the first point while the second is awaited, the last
/// finished measurement and what the pointer would snap to now. View state
/// only; it is never written to a file or a comment.
#[derive(Debug, Default)]
pub struct Measure {
    pub first: Option<Snap>,
    pub done: Option<Measured>,
    pub hover: Option<Snap>,
    curvature: HashMap<usize, MeshCurvature>,
}

impl Measure {
    /// A click or tap on `snap`: point A, then point B; the next click starts
    /// a new measurement that replaces the finished one.
    pub fn place(&mut self, snap: Snap) {
        match self.first.take() {
            Some(a) => self.done = Some(Measured { a, b: snap }),
            None => {
                self.first = Some(snap);
                self.done = None;
            }
        }
    }

    /// What the pointer snaps to: a mesh vertex within [`SNAP_PX`], else the
    /// nearest point of a model edge within it, else the face under it; only
    /// what is in sight and not cut away by the section.
    pub fn snap(&mut self, model: &Model, camera: &Camera, pointer: &Pointer<'_>) -> Option<Snap> {
        let aspect = pointer.size[0].max(1.0) / pointer.size[1].max(1.0);
        let screen = |point: [f64; 3]| screen_point(camera, aspect, pointer.size, point);
        let at = [
            (pointer.ndc[0] + 1.0) * 0.5 * pointer.size[0],
            (1.0 - pointer.ndc[1]) * 0.5 * pointer.size[1],
        ];
        let cut = |point: [f64; 3]| {
            pointer
                .section
                .is_some_and(|s| Vec3::from(s.normal).dot(Vec3::from(point)) > s.offset_mm)
        };
        let (origin, ray) = camera.ray(pointer.ndc, aspect);
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let consider = |list: &mut Vec<Candidate>, point: [f64; 3], body: usize, keys: [Key; 2]| {
            if let Some((px, depth)) = screen(point) {
                let off = ((px[0] - at[0]).powi(2) + (px[1] - at[1]).powi(2)).sqrt();
                if off <= SNAP_PX && !cut(point) {
                    list.push(Candidate {
                        off,
                        depth,
                        point,
                        body,
                        keys,
                    });
                }
            }
        };
        for (index, body) in model.bodies.iter().enumerate() {
            if !pointer.visible[body.occurrence] || !near(&screen, body.bounds_mm, at) {
                continue;
            }
            let placement = Affine3::from_column_major(body.world_matrix_f64_mm);
            let world = |local: [f32; 3]| -> [f64; 3] {
                placement
                    .transform_point(Vec3::from(source_point(local)))
                    .into()
            };
            for (positions, _) in model.triangle_buffers(body.mesh) {
                for chunk in positions.chunks_exact(12) {
                    let local = std::array::from_fn(|k| {
                        f32::from_le_bytes(chunk[k * 4..k * 4 + 4].try_into().expect("point"))
                    });
                    consider(&mut vertices, world(local), index, [key(local); 2]);
                }
            }
            for (positions, indices) in model.line_buffers(body.mesh) {
                for segment in 0..indices.len() / 8 {
                    let ends =
                        [0, 1].map(|end| indexed_point(positions, indices, segment * 2 + end));
                    let point = closest_to_ray(origin, ray, ends.map(world));
                    consider(&mut edges, point, index, ends.map(key));
                }
            }
        }
        for (kind, mut candidates) in [(SnapKind::Vertex, vertices), (SnapKind::Edge, edges)] {
            // Of candidates equally near the pointer, the one nearest the viewer
            // first: in a top view a box's top corner hides its bottom one.
            candidates.sort_by(|a, b| {
                (a.off * 2.0)
                    .round()
                    .total_cmp(&(b.off * 2.0).round())
                    .then(a.depth.total_cmp(&b.depth))
            });
            for candidate in candidates.iter().take(SIGHT_CHECKS) {
                if !in_sight(model, camera, aspect, pointer, candidate.point) {
                    continue;
                }
                let curvature = self.curvature(model, model.bodies[candidate.body].mesh);
                let approximate = match kind {
                    SnapKind::Vertex => curvature.vertices.contains(&candidate.keys[0]),
                    _ => curvature.edges.contains(&edge_key(candidate.keys)),
                };
                return Some(Snap {
                    point: candidate.point,
                    kind,
                    approximate,
                });
            }
        }
        let hit = camera.pick_hit(model, pointer.ndc, aspect, pointer.visible, pointer.section)?;
        let curvature = self.curvature(model, model.bodies[hit.body].mesh);
        Some(Snap {
            point: hit.point,
            kind: SnapKind::Face,
            approximate: curvature.triangles[hit.primitive][hit.triangle],
        })
    }

    fn curvature(&mut self, model: &Model, mesh: usize) -> &MeshCurvature {
        self.curvature
            .entry(mesh)
            .or_insert_with(|| MeshCurvature::of(model, mesh))
    }
}

/// A vertex or an edge point near the pointer: how far from it on screen,
/// its depth, the world point, its body and the vertex keys it stands on.
struct Candidate {
    off: f32,
    depth: f32,
    point: [f64; 3],
    body: usize,
    keys: [Key; 2],
}

/// `point` on the view in screen points from its top left, with its depth;
/// `None` behind the camera.
fn screen_point(
    camera: &Camera,
    aspect: f32,
    size: [f32; 2],
    point: [f64; 3],
) -> Option<([f32; 2], f32)> {
    let p = camera.project(point.map(|v| v.to_f32().unwrap_or(f32::MAX)), aspect);
    ((0.0..=1.0).contains(&p[2]) && p[0].is_finite() && p[1].is_finite()).then(|| {
        (
            [(p[0] + 1.0) * 0.5 * size[0], (1.0 - p[1]) * 0.5 * size[1]],
            p[2],
        )
    })
}

/// Whether `bounds` come within [`SNAP_PX`] of the pointer `at` on screen;
/// bounds reaching behind the camera always do.
fn near(
    screen: &impl Fn([f64; 3]) -> Option<([f32; 2], f32)>,
    bounds: [[f64; 3]; 2],
    at: [f32; 2],
) -> bool {
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for bits in 0..8 {
        let corner = std::array::from_fn(|i| bounds[(bits >> i) & 1][i]);
        let Some((px, _)) = screen(corner) else {
            return true;
        };
        for i in 0..2 {
            low[i] = low[i].min(px[i]);
            high[i] = high[i].max(px[i]);
        }
    }
    (0..2).all(|i| at[i] >= low[i] - SNAP_PX && at[i] <= high[i] + SNAP_PX)
}

/// The point of segment `ends` nearest the ray from `origin` along `ray`.
fn closest_to_ray(origin: Vec3, ray: Vec3, ends: [[f64; 3]; 2]) -> [f64; 3] {
    let start = Vec3::from(ends[0]);
    let along = Vec3::from(ends[1]) - start;
    let between = start - origin;
    let (a, b, c) = (along.dot(along), along.dot(ray), ray.dot(ray));
    let (d, e) = (along.dot(between), ray.dot(between));
    let denominator = a * c - b * b;
    let s = if denominator <= a * c * ketchup_tolerance::NEGLIGIBLE {
        0.0
    } else {
        ((b * e - c * d) / denominator).clamp(0.0, 1.0)
    };
    (start + along * s).into()
}

/// Whether nothing visible stands between the viewer and `point`: the first
/// face on the ray through it lies no nearer than the point itself, up to the
/// single precision the faces are tested in.
fn in_sight(
    model: &Model,
    camera: &Camera,
    aspect: f32,
    pointer: &Pointer<'_>,
    point: [f64; 3],
) -> bool {
    let p = camera.project(point.map(|v| v.to_f32().unwrap_or(f32::MAX)), aspect);
    let ndc = [p[0], p[1]];
    let Some((_, hit)) = camera.pick_point(model, ndc, aspect, pointer.visible, pointer.section)
    else {
        return true;
    };
    let (origin, ray) = camera.ray(ndc, aspect);
    let depth = |q: [f64; 3]| (Vec3::from(q) - origin).dot(ray) / ray.length();
    let magnitude = model
        .bounds_mm
        .iter()
        .flatten()
        .fold(1.0_f64, |m, v| m.max(v.abs()));
    depth(point) <= depth(hit) + magnitude * f64::from(f32::EPSILON) * 64.0
}

/// A mesh vertex by its exact single-precision position; `-0.0` is `0.0`.
type Key = [u32; 3];

fn key(point: [f32; 3]) -> Key {
    point.map(|v| (v + 0.0).to_bits())
}

fn edge_key(keys: [Key; 2]) -> [Key; 2] {
    let [a, b] = keys;
    if a <= b { [a, b] } else { [b, a] }
}

/// Which triangles of one mesh lie on curved faces, and their vertices and
/// sides. Two triangles sharing a side that is not a presentation edge belong
/// to one face; when they do not lie in one plane that face is curved.
#[derive(Debug)]
struct MeshCurvature {
    /// Per triangle primitive, per triangle.
    triangles: Vec<Vec<bool>>,
    vertices: HashSet<Key>,
    edges: HashSet<[Key; 2]>,
}

impl MeshCurvature {
    fn of(model: &Model, mesh: usize) -> Self {
        let features: HashSet<[Key; 2]> = model
            .line_buffers(mesh)
            .flat_map(|(positions, indices)| {
                (0..indices.len() / 8).map(move |segment| {
                    edge_key(
                        [0, 1].map(|end| key(indexed_point(positions, indices, segment * 2 + end))),
                    )
                })
            })
            .collect();
        let mut corners: Vec<Vec<[Key; 3]>> = Vec::new();
        let mut normals: Vec<Vec<Option<Vec3>>> = Vec::new();
        let mut sides: HashMap<[Key; 2], Vec<(usize, usize)>> = HashMap::new();
        for (primitive, (positions, indices)) in model.triangle_buffers(mesh).enumerate() {
            let count = indices.len() / 12;
            let (mut keys, mut planes) = (Vec::with_capacity(count), Vec::with_capacity(count));
            for triangle in 0..count {
                let points: [[f32; 3]; 3] =
                    std::array::from_fn(|k| indexed_point(positions, indices, triangle * 3 + k));
                let corner = points.map(key);
                let [a, b, c] = points.map(|p| Vec3::from(p.map(f64::from)));
                planes.push((b - a).cross(c - a).normalized());
                for (from, to) in [(0, 1), (1, 2), (2, 0)] {
                    if corner[from] != corner[to] {
                        sides
                            .entry(edge_key([corner[from], corner[to]]))
                            .or_default()
                            .push((primitive, triangle));
                    }
                }
                keys.push(corner);
            }
            corners.push(keys);
            normals.push(planes);
        }
        let mut triangles: Vec<Vec<bool>> = normals.iter().map(|n| vec![false; n.len()]).collect();
        let flat = ketchup_tolerance::MESH_FACET_ANGLE_RAD.cos();
        for (side, users) in &sides {
            if features.contains(side) {
                continue;
            }
            for (i, &(p, t)) in users.iter().enumerate() {
                for &(q, u) in &users[i + 1..] {
                    // A flipped neighbour lying in the same plane is not a bend.
                    if let (Some(n), Some(m)) = (normals[p][t], normals[q][u])
                        && n.dot(m).abs() < flat
                    {
                        triangles[p][t] = true;
                        triangles[q][u] = true;
                    }
                }
            }
        }
        let mut vertices = HashSet::new();
        let mut edges = HashSet::new();
        for (p, flags) in triangles.iter().enumerate() {
            for (t, _) in flags.iter().enumerate().filter(|(_, curved)| **curved) {
                let [a, b, c] = corners[p][t];
                vertices.extend([a, b, c]);
                edges.extend([edge_key([a, b]), edge_key([b, c]), edge_key([c, a])]);
            }
        }
        Self {
            triangles,
            vertices,
            edges,
        }
    }
}
