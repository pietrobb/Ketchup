//! Analytic pair answers for straight extrusions of line profiles, such as
//! notched, housed or sloped parts. The profile is split into
//! convex trapezoids, so the solid is a union of convex prisms that only share
//! faces. Cuts after the extrusion only remove material, so the prisms still
//! enclose the solid and prove separation; without cuts they are the solid and
//! also measure its touching faces.
use super::hull::{self, COINCIDENT_SLAB_TOLERANCES, Extrusion, HullRelation, PARALLEL};
use ketchup_geometry::contact_patch::ContactPatch;
use ketchup_geometry::linalg::{cross, dot};
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry, ExactBRepPlanarLoop,
    ExactBRepPlanarSegment,
};
use ketchup_model::tolerance::ROUNDING;

/// Most prisms one solid is split into; a more detailed profile stays exact-only.
const MAX_PIECES: usize = 64;

/// A solid's convex profile pieces, in definition-local coordinates.
pub(super) struct LocalPieces {
    extrusion: Extrusion,
    polygons: Vec<Vec<[f64; 2]>>,
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
    let (extrusion, geometry) = hull::base_extrusion(graph)?;
    Some(LocalPieces {
        extrusion,
        polygons: convex_pieces(geometry)?,
    })
}

impl LocalPieces {
    /// Rigid placements only; anything else stays exact-only.
    pub(super) fn world(&self, matrix: &[f64; 16]) -> Option<Vec<Piece>> {
        let axes = hull::rigid_axes(matrix)?;
        let place = |p: [f64; 3]| {
            [0, 1, 2].map(|row| {
                matrix[4 * row] * p[0]
                    + matrix[4 * row + 1] * p[1]
                    + matrix[4 * row + 2] * p[2]
                    + matrix[4 * row + 3]
            })
        };
        let local_direction = self.extrusion.direction();
        let direction = [0, 1, 2].map(|row| {
            (0..3)
                .map(|k| axes[k][row] * local_direction[k])
                .sum::<f64>()
        });
        let [start, end] = self.extrusion.interval;
        self.polygons
            .iter()
            .map(|polygon| {
                let bottom = polygon
                    .iter()
                    .map(|p| place(self.extrusion.point(*p, start)))
                    .collect::<Vec<_>>();
                let top = polygon
                    .iter()
                    .map(|p| place(self.extrusion.point(*p, end)))
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

/// `a` and `b` are each one solid's pieces. Separation has no gap, so with
/// `exact_gap` (program checks read it) a separated pair is left to OCCT.
pub(super) fn measure(
    a: &[Piece],
    b: &[Piece],
    graphs: [&ExactBRepGraph; 2],
    tolerance_mm: f64,
    exact_gap: bool,
) -> Option<(Vec<ContactPatch>, Option<f64>)> {
    match relate_solids(a, b, tolerance_mm)? {
        None => (!exact_gap).then(|| (Vec::new(), None)),
        Some(patches) => graphs
            .iter()
            .all(|graph| {
                matches!(
                    graph.nodes.last().map(|node| &node.operation),
                    Some(ExactBRepOperation::Extrude { .. })
                )
            })
            .then_some((patches, Some(0.0))),
    }
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
                HullRelation::Overlapping => return None,
                HullRelation::Separated => {}
                HullRelation::Touching { area_mm2, normal } => {
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

/// Separating-axis classification of two convex prisms: their face normals and
/// the crosses of their edge directions decide separation and penetration depth.
fn relate(a: &Piece, b: &Piece, tolerance_mm: f64) -> HullRelation {
    let mut axes = a
        .faces
        .iter()
        .chain(&b.faces)
        .map(|face| face.normal)
        .collect::<Vec<_>>();
    for left in &a.edges {
        for right in &b.edges {
            if let Some(axis) = unit(cross(*left, *right)) {
                axes.push(axis);
            }
        }
    }
    let mut min_overlap = f64::INFINITY;
    for axis in axes {
        let ((a_low, a_high), (b_low, b_high)) = (a.extent(axis), b.extent(axis));
        let overlap = a_high.min(b_high) - a_low.max(b_low);
        if overlap < -tolerance_mm {
            return HullRelation::Separated;
        }
        min_overlap = min_overlap.min(overlap);
    }
    let coincident_slab_mm = COINCIDENT_SLAB_TOLERANCES * tolerance_mm;
    if min_overlap > coincident_slab_mm {
        return HullRelation::Overlapping;
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
    HullRelation::Touching {
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
