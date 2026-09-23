//! Analytic panel hulls: a rectangle extrusion, optionally reduced by cuts, is
//! always contained in its rectangular box. Two such boxes that do not
//! penetrate prove the solids do not penetrate either, at any rotation.
use ketchup_core::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepPlanarLoop, ExactBRepPlanarSegment,
};

/// Same slab as the native pair query: an apparent overlap up to this depth is
/// shared faces within OCCT's coincidence tolerance, never common volume.
const COINCIDENT_SLAB_MM: f64 = 3.0e-7;
const PARALLEL: f64 = 1.0 - 1.0e-12;

/// Box in definition-local coordinates enclosing the whole solid.
#[derive(Clone, Copy, Debug)]
pub(super) struct LocalHull {
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct WorldHull {
    center: [f64; 3],
    axes: [[f64; 3]; 3],
    half: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum HullRelation {
    /// Farther apart than the contact tolerance.
    Separated,
    /// No common volume; `area_mm2` is the shared outer face area. Cuts can only
    /// remove material, so for cut panels it is an upper bound (e.g. dowel holes).
    Touching { area_mm2: f64 },
    /// Hulls penetrate; only exact geometry can decide (e.g. dowel in its hole).
    Overlapping,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
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

/// Axis-aligned rectangle `[min, max]` in profile coordinates, or None.
fn rectangle(geometry: &ExactBRepPlanarGeometry) -> Option<([f64; 2], [f64; 2])> {
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
    if segments.len() != 4 {
        return None;
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for segment in segments {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let (start, end) = (start_bits.map(f64::from_bits), end_bits.map(f64::from_bits));
        // Every edge must be horizontal or vertical and non-degenerate.
        if (start[0] == end[0]) == (start[1] == end[1]) {
            return None;
        }
        for point in [start, end] {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
    }
    // Four axis-parallel edges spanning exactly two x and two y values form the rectangle.
    let on_frame = segments.iter().all(|segment| {
        let ExactBRepPlanarSegment::Line { start_bits, .. } = segment else {
            return false;
        };
        let p = start_bits.map(f64::from_bits);
        (p[0] == min[0] || p[0] == max[0]) && (p[1] == min[1] || p[1] == max[1])
    });
    (on_frame
        && min.iter().chain(&max).all(|x| x.is_finite())
        && min[0] < max[0]
        && min[1] < max[1])
        .then_some((min, max))
}

/// The producer must be a rectangle extrusion, optionally followed only by
/// subtractive cuts on that same solid, so the extrusion box encloses it.
pub(super) fn local_hull(graph: &ExactBRepGraph) -> Option<LocalHull> {
    let mut node = graph.nodes.len().checked_sub(1)?;
    loop {
        match &graph.nodes[node].operation {
            ExactBRepOperation::ProfileCut { target, .. }
            | ExactBRepOperation::Boolean {
                operation: ExactBRepBooleanOperation::Cut,
                target,
                ..
            } if (target.0 as usize) < node => node = target.0 as usize,
            ExactBRepOperation::Extrude {
                profile, interval, ..
            } => {
                let profile = graph.profiles.get(profile.0 as usize)?;
                let frame = profile.frame_bits.map(f64::from_bits);
                let (origin, u, v, normal) = (
                    [frame[0], frame[1], frame[2]],
                    [frame[3], frame[4], frame[5]],
                    [frame[6], frame[7], frame[8]],
                    [frame[9], frame[10], frame[11]],
                );
                let direction = interval.direction();
                if !axis_aligned(u)
                    || !axis_aligned(v)
                    || !axis_aligned(normal)
                    || dot(direction, normal).abs() != 1.0
                    || dot(u, v) != 0.0
                {
                    return None;
                }
                let (low, high) = rectangle(&profile.geometry)?;
                let (start, end) = (interval.start_mm(), interval.end_mm());
                let mut min = [f64::INFINITY; 3];
                let mut max = [f64::NEG_INFINITY; 3];
                for a in [low[0], high[0]] {
                    for b in [low[1], high[1]] {
                        for t in [start, end] {
                            for axis in 0..3 {
                                let value =
                                    origin[axis] + u[axis] * a + v[axis] * b + direction[axis] * t;
                                min[axis] = min[axis].min(value);
                                max[axis] = max[axis].max(value);
                            }
                        }
                    }
                }
                return ((0..3).all(|axis| {
                    min[axis].is_finite() && max[axis].is_finite() && min[axis] < max[axis]
                }))
                .then_some(LocalHull { min, max });
            }
            _ => return None,
        }
    }
}

impl LocalHull {
    /// Rigid (rotation + translation) placements only; anything else stays exact-only.
    pub(super) fn world(self, matrix: [f64; 16]) -> Option<WorldHull> {
        if !matrix.iter().all(|x| x.is_finite()) || matrix[12..] != [0.0, 0.0, 0.0, 1.0] {
            return None;
        }
        let axes = [0, 1, 2].map(|column| [matrix[column], matrix[4 + column], matrix[8 + column]]);
        for i in 0..3 {
            for j in 0..3 {
                let expected = if i == j { 1.0 } else { 0.0 };
                if (dot(axes[i], axes[j]) - expected).abs() > 1.0e-12 {
                    return None;
                }
            }
        }
        let local_center = [0, 1, 2].map(|axis| 0.5 * (self.min[axis] + self.max[axis]));
        let center = [0, 1, 2].map(|row| {
            matrix[4 * row] * local_center[0]
                + matrix[4 * row + 1] * local_center[1]
                + matrix[4 * row + 2] * local_center[2]
                + matrix[4 * row + 3]
        });
        let half = [0, 1, 2].map(|axis| 0.5 * (self.max[axis] - self.min[axis]));
        Some(WorldHull { center, axes, half })
    }
}

impl WorldHull {
    fn radius(&self, axis: [f64; 3]) -> f64 {
        (0..3)
            .map(|i| self.half[i] * dot(self.axes[i], axis).abs())
            .sum()
    }

    /// Outer face `sign * axes[index]` of this box, as 4 corners.
    fn face(&self, index: usize, sign: f64) -> [[f64; 3]; 4] {
        let (a, b) = ((index + 1) % 3, (index + 2) % 3);
        let center =
            [0, 1, 2].map(|k| self.center[k] + sign * self.half[index] * self.axes[index][k]);
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(s, t)| {
            [0, 1, 2].map(|k| {
                center[k] + s * self.half[a] * self.axes[a][k] + t * self.half[b] * self.axes[b][k]
            })
        })
    }
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
        .abs()
        * 0.5
}

/// Intersection of B's face (a convex quad) with A's face rectangle, in A's face frame.
fn shared_face_area(a: &WorldHull, a_index: usize, b_face: [[f64; 3]; 4]) -> f64 {
    let (u, v) = ((a_index + 1) % 3, (a_index + 2) % 3);
    let (hu, hv) = (a.half[u], a.half[v]);
    let mut polygon = b_face
        .map(|p| {
            let d = [0, 1, 2].map(|k| p[k] - a.center[k]);
            [dot(d, a.axes[u]), dot(d, a.axes[v])]
        })
        .to_vec();
    // Sutherland–Hodgman against the four edges of A's rectangle.
    for (axis, limit, keep_below) in [
        (0, hu, true),
        (0, -hu, false),
        (1, hv, true),
        (1, -hv, false),
    ] {
        let inside = |p: [f64; 2]| {
            if keep_below {
                p[axis] <= limit
            } else {
                p[axis] >= limit
            }
        };
        let mut clipped = Vec::with_capacity(polygon.len() + 2);
        for i in 0..polygon.len() {
            let (p, q) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            if inside(p) {
                clipped.push(p);
            }
            if inside(p) != inside(q) {
                let t = (limit - p[axis]) / (q[axis] - p[axis]);
                clipped.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        polygon = clipped;
        if polygon.len() < 3 {
            return 0.0;
        }
    }
    polygon_area(&polygon)
}

/// Separating-axis classification of two boxes (15 axes: 3 + 3 face normals, 9 edge crosses).
pub(super) fn relate(a: &WorldHull, b: &WorldHull, tolerance_mm: f64) -> HullRelation {
    let offset = [0, 1, 2].map(|k| b.center[k] - a.center[k]);
    let mut axes = Vec::with_capacity(15);
    axes.extend(a.axes);
    axes.extend(b.axes);
    for i in 0..3 {
        for j in 0..3 {
            let c = cross(a.axes[i], b.axes[j]);
            let norm = dot(c, c).sqrt();
            if norm > 1.0e-9 {
                axes.push(c.map(|x| x / norm));
            }
        }
    }
    let mut min_overlap = f64::INFINITY;
    for axis in &axes {
        let overlap = a.radius(*axis) + b.radius(*axis) - dot(offset, *axis).abs();
        if overlap < -tolerance_mm {
            return HullRelation::Separated;
        }
        min_overlap = min_overlap.min(overlap);
    }
    if min_overlap > COINCIDENT_SLAB_MM {
        return HullRelation::Overlapping;
    }
    // Positive area needs a face of A coplanar with an opposite face of B.
    let mut area_mm2: f64 = 0.0;
    for i in 0..3 {
        let normal = a.axes[i];
        let overlap = a.radius(normal) + b.radius(normal) - dot(offset, normal).abs();
        if overlap.abs() > COINCIDENT_SLAB_MM {
            continue;
        }
        let Some(j) = (0..3).find(|&j| dot(normal, b.axes[j]).abs() >= PARALLEL) else {
            continue;
        };
        let b_sign = if dot(offset, b.axes[j]) >= 0.0 {
            -1.0
        } else {
            1.0
        };
        area_mm2 = area_mm2.max(shared_face_area(a, i, b.face(j, b_sign)));
    }
    HullRelation::Touching { area_mm2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(min: [f64; 3], max: [f64; 3], matrix: [f64; 16]) -> WorldHull {
        LocalHull { min, max }.world(matrix).unwrap()
    }

    const IDENTITY: [f64; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    fn translated(x: f64, y: f64, z: f64) -> [f64; 16] {
        let mut m = IDENTITY;
        (m[3], m[7], m[11]) = (x, y, z);
        m
    }

    fn rotated_z(degrees: f64, x: f64, y: f64, z: f64) -> [f64; 16] {
        let (s, c) = degrees.to_radians().sin_cos();
        [
            c, -s, 0.0, x, s, c, 0.0, y, 0.0, 0.0, 1.0, z, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    #[test]
    fn shelf_resting_on_bottom_touches_with_its_full_footprint() {
        let bottom = hull([0.0; 3], [500.0, 350.0, 18.0], IDENTITY);
        let shelf = hull([0.0; 3], [464.0, 330.0, 18.0], translated(18.0, 0.0, 18.0));
        let HullRelation::Touching { area_mm2 } = relate(&bottom, &shelf, 1e-7) else {
            panic!()
        };
        assert!((area_mm2 - 464.0 * 330.0).abs() < 1e-6, "{area_mm2}");
        assert_eq!(
            relate(&shelf, &bottom, 1e-7),
            HullRelation::Touching { area_mm2 }
        );
    }

    #[test]
    fn gap_edge_contact_and_penetration_are_distinguished() {
        let a = hull([0.0; 3], [100.0, 100.0, 18.0], IDENTITY);
        let gap = hull([0.0; 3], [100.0, 100.0, 18.0], translated(0.0, 0.0, 18.001));
        assert_eq!(relate(&a, &gap, 1e-7), HullRelation::Separated);
        let into = hull([0.0; 3], [100.0, 100.0, 18.0], translated(0.0, 0.0, 17.9));
        assert_eq!(relate(&a, &into, 1e-7), HullRelation::Overlapping);
        let edge = hull([0.0; 3], [10.0, 10.0, 10.0], translated(100.0, 100.0, 0.0));
        assert_eq!(
            relate(&a, &edge, 1e-7),
            HullRelation::Touching { area_mm2: 0.0 }
        );
    }

    #[test]
    fn rotated_panels_use_oriented_axes_not_world_bounds() {
        // A 45° board near a box corner: world bounds overlap, the solids do not.
        let a = hull([0.0; 3], [100.0, 100.0, 18.0], IDENTITY);
        let b = hull(
            [-10.0, -2.0, 0.0],
            [10.0, 2.0, 18.0],
            rotated_z(-45.0, 104.0, 104.0, 0.0),
        );
        assert_eq!(relate(&a, &b, 1e-7), HullRelation::Separated);
        // Rotated 90° side standing on a bottom panel shares its 18 x 350 footprint.
        let side = hull(
            [0.0; 3],
            [432.0, 18.0, 350.0],
            [
                0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 450.0, 0.0, 0.0, 0.0, 1.0,
            ],
        );
        let bottom = hull([0.0; 3], [500.0, 350.0, 18.0], IDENTITY);
        let HullRelation::Touching { area_mm2 } = relate(&bottom, &side, 1e-7) else {
            panic!("{:?}", relate(&bottom, &side, 1e-7))
        };
        assert!((area_mm2 - 350.0 * 18.0).abs() < 1e-6, "{area_mm2}");
        // Arbitrary rotation about Z: square on square rotated 30° overlaps in an octagon-like region.
        let top = hull(
            [-50.0, -50.0, 0.0],
            [50.0, 50.0, 10.0],
            rotated_z(30.0, 0.0, 0.0, 18.0),
        );
        let base = hull([-50.0, -50.0, 0.0], [50.0, 50.0, 18.0], IDENTITY);
        let HullRelation::Touching { area_mm2 } = relate(&base, &top, 1e-7) else {
            panic!()
        };
        assert!(area_mm2 > 8000.0 && area_mm2 < 10000.0, "{area_mm2}");
    }

    #[test]
    fn non_rigid_placements_are_refused() {
        let mut scaled = IDENTITY;
        scaled[0] = 2.0;
        assert!(
            LocalHull {
                min: [0.0; 3],
                max: [1.0; 3]
            }
            .world(scaled)
            .is_none()
        );
    }
}
