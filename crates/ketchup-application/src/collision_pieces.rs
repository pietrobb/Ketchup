//! Analytic pair answers for straight extrusions of line profiles, such as
//! notched, housed or sloped parts. The profile is split into
//! convex trapezoids, so the solid is a union of convex prisms that only share
//! faces. Cuts after the extrusion only remove material, so the prisms still
//! enclose the solid and prove separation; without cuts they are the solid and
//! also measure its touching faces.
use ketchup_geometry::contact_patch::ContactPatch;
use ketchup_geometry::linalg::{cross, dot};
use ketchup_model::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepPlanarLoop, ExactBRepPlanarSegment,
};
use ketchup_model::tolerance::{COINCIDENT_SLAB_TOLERANCES, CONTACT_FIT_MM, ROUNDING};

const PARALLEL: f64 = 1.0 - ROUNDING;
/// Most prisms one solid is split into; a more detailed profile stays exact-only.
const MAX_PIECES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Relation {
    /// Farther apart than the contact tolerance.
    Separated,
    /// No common volume; `area_mm2` is the shared face area, on A's face with
    /// outward unit `normal` (zero without area).
    Touching { area_mm2: f64, normal: [f64; 3] },
    /// Common volume; only exact geometry can decide (e.g. pin in its hole).
    Overlapping,
}

/// An axis-aligned straight extrusion in definition-local coordinates.
struct Extrusion {
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    direction: [f64; 3],
    interval: [f64; 2],
}

impl Extrusion {
    /// The profile point `[a, b]` swept `t` along the extrusion.
    fn point(&self, [a, b]: [f64; 2], t: f64) -> [f64; 3] {
        [0, 1, 2].map(|axis| {
            self.origin[axis] + self.u[axis] * a + self.v[axis] * b + self.direction[axis] * t
        })
    }
}

fn axis_aligned(v: [f64; 3]) -> bool {
    let mut ones = 0;
    for c in v {
        if c.abs() == 1.0 {
            ones += 1;
        } else if c != 0.0 {
            return false;
        }
    }
    ones == 1
}

/// Boxes of what each cut of a solid may remove; `None` when one is unknown.
type CutBounds = Option<Vec<[[f64; 3]; 2]>>;

/// The extrusion the producer starts from, if it is followed only by
/// subtractive cuts on that same solid, so anything enclosing it encloses the
/// producer; and the bounds of what its cuts may remove.
fn base_extrusion(
    graph: &ExactBRepGraph,
) -> Option<(Extrusion, &ExactBRepPlanarGeometry, CutBounds)> {
    let mut node = graph.nodes.len().checked_sub(1)?;
    let mut cuts = Vec::new();
    loop {
        let operation = &graph.nodes[node].operation;
        match operation {
            ExactBRepOperation::ProfileCut { target, .. }
            | ExactBRepOperation::Boolean {
                operation: ExactBRepBooleanOperation::Cut,
                target,
                ..
            } if (target.0 as usize) < node => {
                cuts.push(graph.removed_bounds_mm(operation));
                node = target.0 as usize;
            }
            ExactBRepOperation::Extrude {
                profile, interval, ..
            } => {
                let profile = graph.profiles.get(profile.0 as usize)?;
                let [origin, u, v, normal] = profile.frame().to_vectors();
                let direction = interval.direction();
                if !axis_aligned(u)
                    || !axis_aligned(v)
                    || !axis_aligned(normal)
                    || dot(direction, normal).abs() != 1.0
                    || dot(u, v) != 0.0
                {
                    return None;
                }
                let extrusion = Extrusion {
                    origin,
                    u,
                    v,
                    direction,
                    interval: [interval.start_mm(), interval.end_mm()],
                };
                return Some((extrusion, &profile.geometry, cuts.into_iter().collect()));
            }
            _ => return None,
        }
    }
}

/// The rotation's columns of a rigid (rotation + translation) placement.
fn rigid_axes(matrix: &[f64; 16]) -> Option<[[f64; 3]; 3]> {
    if !matrix.iter().all(|x| x.is_finite()) || matrix[12..] != [0.0, 0.0, 0.0, 1.0] {
        return None;
    }
    let axes = [0, 1, 2].map(|column| [matrix[column], matrix[4 + column], matrix[8 + column]]);
    for i in 0..3 {
        for j in 0..3 {
            let expected = if i == j { 1.0 } else { 0.0 };
            if (dot(axes[i], axes[j]) - expected).abs() > ROUNDING {
                return None;
            }
        }
    }
    Some(axes)
}

/// A solid's convex profile pieces and the boxes its cuts may remove, in
/// definition-local coordinates.
pub(super) struct LocalPieces {
    extrusion: Extrusion,
    polygons: Vec<Vec<[f64; 2]>>,
    cuts: CutBounds,
}

/// A placed solid: prisms that enclose it, and boxes enclosing everything its
/// cuts remove (`None` when a cut's reach is unknown). Away from those boxes
/// the solid is its prisms.
pub(super) struct Hull {
    pieces: Vec<Piece>,
    cuts: Option<Vec<Piece>>,
}

/// One convex prism in world coordinates.
#[derive(Clone, Debug)]
pub(super) struct Piece {
    vertices: Vec<[f64; 3]>,
    faces: Vec<Face>,
    edges: Vec<[f64; 3]>,
}

#[derive(Clone, Debug)]
struct Face {
    /// Outward unit normal.
    normal: [f64; 3],
    /// Convex, in order around the face.
    points: Vec<[f64; 3]>,
}

pub(super) fn local_pieces(graph: &ExactBRepGraph) -> Option<LocalPieces> {
    let (extrusion, geometry, cuts) = base_extrusion(graph)?;
    Some(LocalPieces {
        extrusion,
        polygons: convex_pieces(geometry)?,
        cuts,
    })
}

/// `p` placed by `matrix`, translated when `point`.
fn place(matrix: &[f64; 16], p: [f64; 3], point: bool) -> [f64; 3] {
    [0, 1, 2].map(|row| {
        matrix[4 * row] * p[0]
            + matrix[4 * row + 1] * p[1]
            + matrix[4 * row + 2] * p[2]
            + if point { matrix[4 * row + 3] } else { 0.0 }
    })
}

impl LocalPieces {
    pub(super) fn hull(&self, matrix: &[f64; 16]) -> Option<Hull> {
        let pieces = self.world(matrix)?;
        let up = place(matrix, [0.0, 0.0, 1.0], false);
        let cuts = self.cuts.as_ref().and_then(|cuts| {
            cuts.iter()
                .map(|[low, high]| {
                    let rectangle = |z: f64| {
                        [
                            [low[0], low[1]],
                            [high[0], low[1]],
                            [high[0], high[1]],
                            [low[0], high[1]],
                        ]
                        .map(|[x, y]| place(matrix, [x, y, z], true))
                        .to_vec()
                    };
                    Piece::prism(rectangle(low[2]), rectangle(high[2]), up)
                })
                .collect()
        });
        Some(Hull { pieces, cuts })
    }

    /// Rigid placements only; anything else stays exact-only.
    pub(super) fn world(&self, matrix: &[f64; 16]) -> Option<Vec<Piece>> {
        rigid_axes(matrix)?;
        let direction = place(matrix, self.extrusion.direction, false);
        let [start, end] = self.extrusion.interval;
        self.polygons
            .iter()
            .map(|polygon| {
                let bottom = polygon
                    .iter()
                    .map(|p| place(matrix, self.extrusion.point(*p, start), true))
                    .collect::<Vec<_>>();
                let top = polygon
                    .iter()
                    .map(|p| place(matrix, self.extrusion.point(*p, end), true))
                    .collect::<Vec<_>>();
                Piece::prism(bottom, top, direction)
            })
            .collect()
    }
}

impl Piece {
    fn prism(bottom: Vec<[f64; 3]>, top: Vec<[f64; 3]>, direction: [f64; 3]) -> Option<Self> {
        let n = bottom.len();
        let vertices = bottom.iter().chain(&top).copied().collect::<Vec<_>>();
        let count = f64::from(u32::try_from(vertices.len()).ok()?);
        let centroid = [0, 1, 2].map(|k| vertices.iter().map(|p| p[k]).sum::<f64>() / count);
        let outward = |normal: [f64; 3], on_face: [f64; 3]| {
            let offset = [0, 1, 2].map(|k| on_face[k] - centroid[k]);
            if dot(normal, offset) < 0.0 {
                normal.map(|x| -x)
            } else {
                normal
            }
        };
        let mut faces = vec![
            Face {
                normal: outward(direction, top[0]),
                points: top.clone(),
            },
            Face {
                normal: outward(direction, bottom[0]),
                points: bottom.clone(),
            },
        ];
        let mut edges = vec![direction];
        for i in 0..n {
            let (a, b) = (bottom[i], bottom[(i + 1) % n]);
            let edge = unit([0, 1, 2].map(|k| b[k] - a[k]))?;
            let normal = unit(cross(edge, direction))?;
            faces.push(Face {
                normal: outward(normal, a),
                points: vec![a, b, top[(i + 1) % n], top[i]],
            });
            edges.push(edge);
        }
        vertices
            .iter()
            .flatten()
            .all(|x| x.is_finite())
            .then_some(Self {
                vertices,
                faces,
                edges,
            })
    }

    fn extent(&self, axis: [f64; 3]) -> (f64, f64) {
        self.vertices
            .iter()
            .map(|p| dot(*p, axis))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), x| {
                (low.min(x), high.max(x))
            })
    }
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let norm = dot(v, v).sqrt();
    (norm > ROUNDING && norm.is_finite()).then(|| v.map(|x| x / norm))
}

/// The answer for two solids, if their hulls decide it. Separation has no
/// distance, so with `exact_gap` (program checks read it) a separated pair is
/// left to OCCT. Touching faces count only where no cut comes within
/// [`CONTACT_FIT_MM`] of the other solid, for a cut may remove the face.
pub(super) fn measure(
    a: &Hull,
    b: &Hull,
    tolerance_mm: f64,
    exact_gap: bool,
) -> Option<(Vec<ContactPatch>, Option<f64>)> {
    match relate_solids(&a.pieces, &b.pieces, tolerance_mm)? {
        None => (!exact_gap).then(|| (Vec::new(), None)),
        Some(patches) => [(a, b), (b, a)]
            .iter()
            .all(|(cut, other)| {
                cut.cuts
                    .as_ref()
                    .is_some_and(|cuts| apart(cuts, &other.pieces, CONTACT_FIT_MM))
            })
            .then_some((patches, Some(0.0))),
    }
}

/// Whether every piece of `a` is farther than `gap_mm` from every piece of `b`.
fn apart(a: &[Piece], b: &[Piece], gap_mm: f64) -> bool {
    a.iter().all(|left| {
        b.iter()
            .all(|right| least_overlap(left, [0.0; 3], 0.0, right, gap_mm).is_none())
    })
}

/// None when two pieces penetrate; otherwise the shared faces by A's outward
/// normal if any pieces touch. Pieces of one solid only share faces, so a face
/// of B touching an inner face of A would penetrate A's piece behind it: every
/// touching face counted here is an outer face of both solids.
fn relate_solids(a: &[Piece], b: &[Piece], tolerance_mm: f64) -> Option<Option<Vec<ContactPatch>>> {
    let mut touching = false;
    let mut patches: Vec<ContactPatch> = Vec::new();
    for left in a {
        for right in b {
            match relate(left, right, tolerance_mm) {
                Relation::Overlapping => return None,
                Relation::Separated => {}
                Relation::Touching { area_mm2, normal } => {
                    touching = true;
                    if area_mm2 > 0.0 {
                        match patches
                            .iter_mut()
                            .find(|patch| dot(patch.normal, normal) >= PARALLEL)
                        {
                            Some(patch) => patch.area_mm2 += area_mm2,
                            None => patches.push(ContactPatch { normal, area_mm2 }),
                        }
                    }
                }
            }
        }
    }
    Some(touching.then_some(patches))
}

/// True when every position of `a` along the straight translation `sweep`
/// stays apart from or only touches `b`, within the contact tolerance.
pub(super) fn sweep_clear(a: &[Piece], sweep: [f64; 3], b: &[Piece], tolerance_mm: f64) -> bool {
    let scale = a
        .iter()
        .flat_map(|piece| &piece.vertices)
        .flat_map(|p| [0, 1, 2].map(|k| p[k].abs().max((p[k] + sweep[k]).abs())))
        .fold(1.0, f64::max);
    // Rounding of the swept extents, so a clear answer never rests on it.
    let margin = 64.0 * ROUNDING * scale;
    a.iter().all(|left| {
        b.iter().all(|right| {
            least_overlap(left, sweep, margin, right, tolerance_mm)
                .is_none_or(|overlap| overlap <= COINCIDENT_SLAB_TOLERANCES * tolerance_mm)
        })
    })
}

/// Separating-axis test of `a`, swept along `sweep` and grown by `margin`, against
/// `b`: the least overlap over all axes, or None when farther apart than tolerance.
/// A swept convex prism is convex, with faces along A's normals and across A's
/// edges and the sweep, and edges along A's edges and the sweep.
fn least_overlap(
    a: &Piece,
    sweep: [f64; 3],
    margin: f64,
    b: &Piece,
    tolerance_mm: f64,
) -> Option<f64> {
    let moving = sweep != [0.0; 3];
    let mut axes = a
        .faces
        .iter()
        .chain(&b.faces)
        .map(|face| face.normal)
        .collect::<Vec<_>>();
    let a_edges = || a.edges.iter().copied().chain(moving.then_some(sweep));
    if moving {
        axes.extend(a.edges.iter().filter_map(|edge| unit(cross(*edge, sweep))));
    }
    for left in a_edges() {
        for right in &b.edges {
            if let Some(axis) = unit(cross(left, *right)) {
                axes.push(axis);
            }
        }
    }
    let mut min_overlap = f64::INFINITY;
    for axis in axes {
        let ((a_low, a_high), (b_low, b_high)) = (a.extent(axis), b.extent(axis));
        let shift = dot(sweep, axis);
        let (a_low, a_high) = (
            a_low + shift.min(0.0) - margin,
            a_high + shift.max(0.0) + margin,
        );
        let overlap = a_high.min(b_high) - a_low.max(b_low);
        if overlap < -tolerance_mm {
            return None;
        }
        min_overlap = min_overlap.min(overlap);
    }
    Some(min_overlap)
}

/// Separating-axis classification of two convex prisms: their face normals and
/// the crosses of their edge directions decide separation and penetration depth.
fn relate(a: &Piece, b: &Piece, tolerance_mm: f64) -> Relation {
    let Some(min_overlap) = least_overlap(a, [0.0; 3], 0.0, b, tolerance_mm) else {
        return Relation::Separated;
    };
    let coincident_slab_mm = COINCIDENT_SLAB_TOLERANCES * tolerance_mm;
    if min_overlap > coincident_slab_mm {
        return Relation::Overlapping;
    }
    // Positive area needs a face of A in one plane with an opposite face of B.
    let mut area_mm2: f64 = 0.0;
    let mut outward = [0.0; 3];
    for face in &a.faces {
        for other in &b.faces {
            if dot(face.normal, other.normal) > -PARALLEL {
                continue;
            }
            let gap = [0, 1, 2].map(|k| other.points[0][k] - face.points[0][k]);
            if dot(face.normal, gap).abs() > coincident_slab_mm {
                continue;
            }
            let shared = shared_area(face, other);
            if shared > area_mm2 {
                area_mm2 = shared;
                outward = face.normal;
            }
        }
    }
    Relation::Touching {
        area_mm2,
        normal: outward,
    }
}

/// Area of `other` projected into `face`'s plane and clipped to `face`.
fn shared_area(face: &Face, other: &Face) -> f64 {
    let origin = face.points[0];
    let Some(u) = unit([0, 1, 2].map(|k| face.points[1][k] - origin[k])) else {
        return 0.0;
    };
    let v = cross(face.normal, u);
    let flat = |points: &[[f64; 3]]| {
        let mut flat = points
            .iter()
            .map(|p| {
                let d = [0, 1, 2].map(|k| p[k] - origin[k]);
                [dot(d, u), dot(d, v)]
            })
            .collect::<Vec<_>>();
        if signed_area(&flat) < 0.0 {
            flat.reverse();
        }
        flat
    };
    let clip = flat(&face.points);
    let mut polygon = flat(&other.points);
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let side = |p: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        let mut clipped = Vec::with_capacity(polygon.len() + 2);
        for j in 0..polygon.len() {
            let (p, q) = (polygon[j], polygon[(j + 1) % polygon.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                clipped.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                clipped.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        polygon = clipped;
        if polygon.len() < 3 {
            return 0.0;
        }
    }
    signed_area(&polygon).abs()
}

fn signed_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    0.5 * (0..n)
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
}

/// Convex trapezoids (counter-clockwise) whose union is a simple closed loop of
/// lines, split at every vertex abscissa; neighbours between the same two
/// edges are one trapezoid.
fn convex_pieces(geometry: &ExactBRepPlanarGeometry) -> Option<Vec<Vec<[f64; 2]>>> {
    let segments = match geometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => segments,
        ExactBRepPlanarGeometry::Region {
            outer: ExactBRepPlanarLoop::Boundary { segments },
            holes,
        } if holes.is_empty() => segments,
        _ => return None,
    };
    if segments.len() < 3 || segments.len() > 2 * MAX_PIECES {
        return None;
    }
    let mut edges = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        // One loop: each edge starts where the previous one ends.
        let ExactBRepPlanarSegment::Line {
            start_bits: next_start,
            ..
        } = &segments[(index + 1) % segments.len()]
        else {
            return None;
        };
        let (start, end) = (start_bits.map(f64::from_bits), end_bits.map(f64::from_bits));
        if end_bits != next_start
            || start == end
            || !start.iter().chain(&end).all(|x| x.is_finite())
        {
            return None;
        }
        edges.push((start, end));
    }
    // Exact at an edge's own end points, so neighbouring trapezoids share corners.
    let y_at = |(start, end): ([f64; 2], [f64; 2]), x: f64| {
        if x == start[0] {
            start[1]
        } else if x == end[0] {
            end[1]
        } else {
            start[1] + (end[1] - start[1]) * (x - start[0]) / (end[0] - start[0])
        }
    };
    let mut abscissas = edges.iter().map(|(start, _)| start[0]).collect::<Vec<_>>();
    abscissas.sort_by(f64::total_cmp);
    abscissas.dedup();
    // (lower edge, upper edge, from x, to x)
    let mut trapezoids: Vec<(usize, usize, f64, f64)> = Vec::new();
    for slab in abscissas.windows(2) {
        let middle = 0.5 * (slab[0] + slab[1]);
        let mut crossing = (0..edges.len())
            .filter(|&i| {
                let (start, end) = edges[i];
                start[0].min(end[0]) < middle && middle < start[0].max(end[0])
            })
            .collect::<Vec<_>>();
        if crossing.len() % 2 != 0 {
            return None;
        }
        crossing.sort_by(|&i, &j| y_at(edges[i], middle).total_cmp(&y_at(edges[j], middle)));
        for pair in crossing.chunks(2) {
            let (lower, upper) = (pair[0], pair[1]);
            match trapezoids
                .iter_mut()
                .find(|t| t.0 == lower && t.1 == upper && t.3 == slab[0])
            {
                Some(trapezoid) => trapezoid.3 = slab[1],
                None => trapezoids.push((lower, upper, slab[0], slab[1])),
            }
        }
    }
    if trapezoids.is_empty() || trapezoids.len() > MAX_PIECES {
        return None;
    }
    let mut polygons = Vec::with_capacity(trapezoids.len());
    let mut covered = 0.0;
    for (lower, upper, from, to) in trapezoids {
        let corners = [
            [from, y_at(edges[lower], from)],
            [to, y_at(edges[lower], to)],
            [to, y_at(edges[upper], to)],
            [from, y_at(edges[upper], from)],
        ];
        // Crossing edges would put the upper one below the lower one.
        if corners[0][1] > corners[3][1] || corners[1][1] > corners[2][1] {
            return None;
        }
        let mut polygon: Vec<[f64; 2]> = Vec::with_capacity(4);
        for corner in corners {
            if polygon.last() != Some(&corner) && polygon.first() != Some(&corner) {
                polygon.push(corner);
            }
        }
        if polygon.len() < 3 {
            return None;
        }
        covered += signed_area(&polygon);
        polygons.push(polygon);
    }
    // A simple loop's area is its pieces' area; a self-crossing one's is not.
    let loop_area = edges
        .iter()
        .map(|(start, end)| start[0] * end[1] - end[0] * start[1])
        .sum::<f64>()
        .abs()
        * 0.5;
    ((covered - loop_area).abs() <= ROUNDING * loop_area).then_some(polygons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_loop(points: &[[f64; 2]]) -> ExactBRepPlanarGeometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: (0..points.len())
                .map(|i| ExactBRepPlanarSegment::Line {
                    start_bits: points[i].map(f64::to_bits),
                    end_bits: points[(i + 1) % points.len()].map(f64::to_bits),
                })
                .collect(),
        }
    }

    /// A prism in world coordinates: `profile` in the XZ plane, swept along +Y.
    fn prism(profile: &[[f64; 2]], y: [f64; 2]) -> Vec<Piece> {
        convex_pieces(&line_loop(profile))
            .unwrap()
            .iter()
            .map(|polygon| {
                let at = |y: f64| polygon.iter().map(|p| [p[0], y, p[1]]).collect::<Vec<_>>();
                Piece::prism(at(y[0]), at(y[1]), [0.0, 1.0, 0.0]).unwrap()
            })
            .collect()
    }

    fn area(polygons: &[Vec<[f64; 2]>]) -> f64 {
        polygons.iter().map(|p| signed_area(p)).sum()
    }

    // A 60 mm stud with 10 mm deep housings at 754..814 and 1568..1628.
    const STUD: [[f64; 2]; 12] = [
        [0.0, 60.0],
        [60.0, 60.0],
        [60.0, 754.0],
        [50.0, 754.0],
        [50.0, 814.0],
        [60.0, 814.0],
        [60.0, 1568.0],
        [50.0, 1568.0],
        [50.0, 1628.0],
        [60.0, 1628.0],
        [60.0, 2502.0],
        [0.0, 2502.0],
    ];

    #[test]
    fn a_housed_stud_and_a_sloped_board_split_into_trapezoids_of_their_area() {
        let stud = convex_pieces(&line_loop(&STUD)).unwrap();
        // The full-width web plus the three solid stretches beside the housings.
        assert_eq!(stud.len(), 4, "{stud:?}");
        let expected = 60.0 * 2442.0 - 2.0 * 10.0 * 60.0;
        assert!((area(&stud) - expected).abs() < 1e-9, "{}", area(&stud));
        let gable = [
            [140.0, 0.0],
            [1467.5, 0.0],
            [1467.5, 2274.97],
            [140.0, 1161.06],
        ];
        let board = convex_pieces(&line_loop(&gable)).unwrap();
        assert_eq!(board.len(), 1);
        let expected = 1327.5 * (2274.97 + 1161.06) / 2.0;
        assert!((area(&board) - expected).abs() < 1e-6);
    }

    #[test]
    fn curved_or_self_crossing_profiles_are_not_split() {
        let bow_tie = [[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
        assert!(convex_pieces(&line_loop(&bow_tie)).is_none());
        let arc = ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: vec![
                ExactBRepPlanarSegment::Line {
                    start_bits: [0.0f64, 0.0].map(f64::to_bits),
                    end_bits: [10.0f64, 0.0].map(f64::to_bits),
                },
                ExactBRepPlanarSegment::CircularArc {
                    start_bits: [10.0f64, 0.0].map(f64::to_bits),
                    end_bits: [0.0f64, 0.0].map(f64::to_bits),
                    center_bits: [5.0f64, 0.0].map(f64::to_bits),
                    clockwise: false,
                },
            ],
        };
        assert!(convex_pieces(&arc).is_none());
    }

    fn total(patches: &[ContactPatch]) -> f64 {
        patches.iter().map(|patch| patch.area_mm2).sum()
    }

    #[test]
    fn a_noggin_in_a_housing_bears_on_three_faces_and_one_too_deep_penetrates() {
        let stud = prism(&STUD, [0.0, 140.0]);
        // Seated: fills the housing 50..60 and continues to x = 400.
        let noggin = prism(
            &[[50.0, 754.0], [400.0, 754.0], [400.0, 814.0], [50.0, 814.0]],
            [0.0, 140.0],
        );
        let patches = relate_solids(&stud, &noggin, 1e-7).unwrap().unwrap();
        // Housing floor and ceiling (10 x 140 each) and its back (60 x 140).
        assert!(
            (total(&patches) - (2.0 * 10.0 + 60.0) * 140.0).abs() < 1e-6,
            "{patches:?}"
        );
        let up = patches.iter().find(|p| p.normal[2] > 0.5).unwrap();
        assert!((up.area_mm2 - 1400.0).abs() < 1e-6, "{patches:?}");
        // 1 mm deeper than the housing: common volume, so exact geometry decides.
        let deep = prism(
            &[[49.0, 754.0], [400.0, 754.0], [400.0, 814.0], [49.0, 814.0]],
            [0.0, 140.0],
        );
        assert!(relate_solids(&stud, &deep, 1e-7).is_none());
        // Beside the stud with a gap: separated.
        let apart = prism(
            &[[61.0, 0.0], [400.0, 0.0], [400.0, 60.0], [61.0, 60.0]],
            [0.0, 140.0],
        );
        assert!(matches!(relate_solids(&stud, &apart, 1e-7), Some(None)));
    }

    /// The native contact tolerance the motion check runs with.
    const CONTACT_MM: f64 = 0.001;

    #[test]
    fn a_block_sliding_on_a_base_may_touch_it_but_not_pass_through_a_stop() {
        let base = prism(
            &[[0.0, 0.0], [2000.0, 0.0], [2000.0, 18.0], [0.0, 18.0]],
            [0.0, 500.0],
        );
        let stop = prism(
            &[[800.0, 18.0], [900.0, 18.0], [900.0, 118.0], [800.0, 118.0]],
            [0.0, 500.0],
        );
        let block = prism(
            &[[0.0, 18.0], [100.0, 18.0], [100.0, 118.0], [0.0, 118.0]],
            [0.0, 100.0],
        );
        assert!(sweep_clear(&block, [500.0, 0.0, 0.0], &base, CONTACT_MM));
        assert!(sweep_clear(&block, [700.0, 0.0, 0.0], &stop, CONTACT_MM));
        assert!(!sweep_clear(&block, [750.0, 0.0, 0.0], &stop, CONTACT_MM));
        // Both ends are clear of the stop, but the way between them is not.
        assert!(!sweep_clear(&block, [1000.0, 0.0, 0.0], &stop, CONTACT_MM));
        // Lifted 1 mm, a diagonal pass over the stop's corner is clear.
        let over = prism(
            &[[0.0, 119.0], [100.0, 119.0], [100.0, 219.0], [0.0, 219.0]],
            [0.0, 100.0],
        );
        assert!(sweep_clear(&over, [1000.0, 300.0, 0.0], &stop, CONTACT_MM));
    }

    #[test]
    fn a_plate_on_a_sloped_stud_top_shares_the_slope() {
        // Stud whose top runs from (0, 1000) to (60, 1030); plate 40 mm thick on it.
        let stud = prism(
            &[[0.0, 0.0], [60.0, 0.0], [60.0, 1030.0], [0.0, 1000.0]],
            [0.0, 140.0],
        );
        let plate = prism(
            &[[0.0, 1000.0], [60.0, 1030.0], [60.0, 1070.0], [0.0, 1040.0]],
            [0.0, 140.0],
        );
        let patches = relate_solids(&stud, &plate, 1e-7).unwrap().unwrap();
        let slope = (60.0f64 * 60.0 + 30.0 * 30.0).sqrt();
        assert!(
            (total(&patches) - slope * 140.0).abs() < 1e-6,
            "{patches:?}"
        );
        assert!(
            patches[0].normal[2] > 0.0 && patches[0].normal[0] < 0.0,
            "{patches:?}"
        );
    }
}
