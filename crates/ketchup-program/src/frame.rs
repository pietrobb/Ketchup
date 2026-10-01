//! Rigid part frames and oriented bounding boxes.
//!
//! A part's local frame maps a local point `p` to `at + R·p`. `R` is a proper
//! rotation stored row-major; its columns are the part's local x, y and z axes
//! expressed in world coordinates.

use ketchup_geometry::linalg::{self, cross, dot, length};
use ketchup_model::tolerance::ROUNDING;

pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

const ORIENTATION_TOLERANCE: f64 = ROUNDING;

#[must_use]
pub fn is_identity(rotation: &Mat3) -> bool {
    same_orientation(rotation, &IDENTITY)
}

#[must_use]
pub fn same_orientation(a: &Mat3, b: &Mat3) -> bool {
    (0..3).all(|row| {
        (0..3).all(|column| (a[row][column] - b[row][column]).abs() <= ORIENTATION_TOLERANCE)
    })
}

#[must_use]
pub fn apply(rotation: &Mat3, vector: [f64; 3]) -> [f64; 3] {
    linalg::Mat3::from_rows(*rotation)
        .mul_vec(vector.into())
        .to_array()
}

#[must_use]
pub fn apply_transposed(rotation: &Mat3, vector: [f64; 3]) -> [f64; 3] {
    apply(&transposed(rotation), vector)
}

#[must_use]
pub fn multiply(left: &Mat3, right: &Mat3) -> Mat3 {
    (linalg::Mat3::from_rows(*left) * linalg::Mat3::from_rows(*right)).rows
}

/// Inverse of a rotation.
#[must_use]
pub fn transposed(rotation: &Mat3) -> Mat3 {
    linalg::Mat3::from_rows(*rotation).transpose().rows
}

/// Column `index` of `rotation`: the local axis in world coordinates.
#[must_use]
pub fn axis(rotation: &Mat3, index: usize) -> [f64; 3] {
    std::array::from_fn(|row| rotation[row][index])
}

/// Unit vector, or `None` for a zero or non-finite vector.
#[must_use]
pub fn normalized(vector: [f64; 3]) -> Option<[f64; 3]> {
    linalg::normalize_within(vector, ORIENTATION_TOLERANCE)
}

/// Right-handed rotation by `angle_degrees` about `axis` (Rodrigues).
#[must_use]
pub fn axis_angle(axis: [f64; 3], angle_degrees: f64) -> Option<Mat3> {
    let [x, y, z] = normalized(axis)?;
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    let c = 1.0 - cos;
    Some([
        [cos + x * x * c, x * y * c - z * sin, x * z * c + y * sin],
        [y * x * c + z * sin, cos + y * y * c, y * z * c - x * sin],
        [z * x * c - y * sin, z * y * c + x * sin, cos + z * z * c],
    ])
}

/// Frame whose local z points along `z` and local x along the part of `x`
/// perpendicular to `z`. `None` when either is zero or they are parallel.
#[must_use]
pub fn from_axes(x: [f64; 3], z: [f64; 3]) -> Option<Mat3> {
    let z = normalized(z)?;
    let along = dot(x, z);
    let x = normalized(std::array::from_fn(|i| x[i] - along * z[i]))?;
    let y = cross(z, x);
    Some(std::array::from_fn(|row| [x[row], y[row], z[row]]))
}

/// A box in a rigid frame: world centre, world unit axes and half extents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub centre: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub half: [f64; 3],
}

impl Obb {
    #[must_use]
    pub fn new(at: [f64; 3], rotation: &Mat3, local_min: [f64; 3], local_max: [f64; 3]) -> Self {
        let local_centre = std::array::from_fn(|i| (local_min[i] + local_max[i]) * 0.5);
        let offset = apply(rotation, local_centre);
        Self {
            centre: std::array::from_fn(|i| at[i] + offset[i]),
            axes: std::array::from_fn(|i| axis(rotation, i)),
            half: std::array::from_fn(|i| (local_max[i] - local_min[i]) * 0.5),
        }
    }

    fn radius_along(&self, direction: [f64; 3]) -> f64 {
        (0..3)
            .map(|i| self.half[i] * dot(self.axes[i], direction).abs())
            .sum()
    }

    /// World axis-aligned bounds.
    #[must_use]
    pub fn world_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let extent: [f64; 3] = std::array::from_fn(|world| {
            (0..3)
                .map(|i| self.half[i] * self.axes[i][world].abs())
                .sum()
        });
        (
            std::array::from_fn(|i| self.centre[i] - extent[i]),
            std::array::from_fn(|i| self.centre[i] + extent[i]),
        )
    }

    /// Largest separation along any separating axis candidate (face normals of
    /// both boxes and their edge cross products). Positive: the boxes are at
    /// least that far apart. Near zero: they touch. Negative: they overlap by at
    /// least that much along every candidate axis.
    #[must_use]
    pub fn separation(&self, other: &Self) -> f64 {
        self.separating_axis(other).1
    }

    /// The candidate axis behind [`Self::separation`], pointing from this box
    /// towards `other`, and that separation. When the boxes overlap, moving
    /// `other` by `-separation` along the axis is the shortest way apart.
    #[must_use]
    pub fn separating_axis(&self, other: &Self) -> ([f64; 3], f64) {
        let delta: [f64; 3] = std::array::from_fn(|i| other.centre[i] - self.centre[i]);
        let mut candidates = Vec::with_capacity(15);
        candidates.extend(self.axes);
        candidates.extend(other.axes);
        for a in self.axes {
            for b in other.axes {
                if let Some(axis) = normalized(cross(a, b)) {
                    candidates.push(axis);
                }
            }
        }
        candidates
            .into_iter()
            .map(|axis| {
                let along = dot(delta, axis);
                let oriented = if along < 0.0 {
                    axis.map(|value| -value)
                } else {
                    axis
                };
                let gap = along.abs() - self.radius_along(axis) - other.radius_along(axis);
                (oriented, gap)
            })
            .fold(([0.0, 0.0, 1.0], f64::NEG_INFINITY), |best, candidate| {
                if candidate.1 > best.1 {
                    candidate
                } else {
                    best
                }
            })
    }

    /// Shortest distance between the two boxes; 0 when they touch or overlap.
    #[must_use]
    pub fn distance(&self, other: &Self) -> f64 {
        if self.separation(other) <= 0.0 {
            return 0.0;
        }
        // Two disjoint convex boxes are closest vertex-to-box or edge-to-edge.
        let vertex_box = self
            .vertices()
            .into_iter()
            .map(|point| other.point_distance(point))
            .chain(
                other
                    .vertices()
                    .into_iter()
                    .map(|point| self.point_distance(point)),
            );
        let edge_edge = self.edges().into_iter().flat_map(|(p0, p1)| {
            other
                .edges()
                .into_iter()
                .map(move |(q0, q1)| segment_distance(p0, p1, q0, q1))
        });
        vertex_box.chain(edge_edge).fold(f64::INFINITY, f64::min)
    }

    fn point_distance(&self, point: [f64; 3]) -> f64 {
        let delta: [f64; 3] = std::array::from_fn(|i| point[i] - self.centre[i]);
        (0..3)
            .map(|i| {
                (dot(delta, self.axes[i]).abs() - self.half[i])
                    .max(0.0)
                    .powi(2)
            })
            .sum::<f64>()
            .sqrt()
    }

    fn vertex(&self, signs: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|n| {
            self.centre[n]
                + (0..3)
                    .map(|i| self.axes[i][n] * self.half[i] * signs[i])
                    .sum::<f64>()
        })
    }

    fn vertices(&self) -> [[f64; 3]; 8] {
        std::array::from_fn(|bits| {
            self.vertex(std::array::from_fn(|i| {
                if bits >> i & 1 == 1 { 1.0 } else { -1.0 }
            }))
        })
    }

    fn edges(&self) -> [([f64; 3], [f64; 3]); 12] {
        std::array::from_fn(|index| {
            let (axis, bits) = (index / 4, index % 4);
            let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut signs = [0.0; 3];
            signs[p] = if bits & 1 == 1 { 1.0 } else { -1.0 };
            signs[q] = if bits & 2 == 2 { 1.0 } else { -1.0 };
            signs[axis] = -1.0;
            let start = self.vertex(signs);
            signs[axis] = 1.0;
            (start, self.vertex(signs))
        })
    }

    /// Half-spaces `normal · x <= offset` whose intersection is the box, in
    /// the order x+, x-, y+, y-, z+, z-.
    #[must_use]
    pub fn planes(&self) -> [([f64; 3], f64); 6] {
        std::array::from_fn(|index| {
            let axis = self.axes[index / 2];
            let normal = if index % 2 == 0 {
                axis
            } else {
                axis.map(|value| -value)
            };
            (normal, dot(normal, self.centre) + self.half[index / 2])
        })
    }

    #[must_use]
    pub fn contains(&self, point: [f64; 3], tolerance: f64) -> bool {
        self.planes()
            .iter()
            .all(|(normal, offset)| dot(*normal, point) <= offset + tolerance)
    }
}

/// Shortest distance between segments `p0-p1` and `q0-q1`.
fn segment_distance(p0: [f64; 3], p1: [f64; 3], q0: [f64; 3], q1: [f64; 3]) -> f64 {
    let sub = |a: [f64; 3], b: [f64; 3]| -> [f64; 3] { std::array::from_fn(|i| a[i] - b[i]) };
    let (d1, d2, r) = (sub(p1, p0), sub(q1, q0), sub(p0, q0));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (c, b) = (dot(d1, r), dot(d1, d2));
    let (s, t) = if a <= f64::EPSILON && e <= f64::EPSILON {
        (0.0, 0.0)
    } else if a <= f64::EPSILON {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else if e <= f64::EPSILON {
        ((-c / a).clamp(0.0, 1.0), 0.0)
    } else {
        let denominator = a * e - b * b;
        let mut s = if denominator > f64::EPSILON {
            ((b * f - c * e) / denominator).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut t = (b * s + f) / e;
        if t < 0.0 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else if t > 1.0 {
            t = 1.0;
            s = ((b - c) / a).clamp(0.0, 1.0);
        }
        (s, t)
    };
    let closest_p: [f64; 3] = std::array::from_fn(|i| p0[i] + d1[i] * s);
    let closest_q: [f64; 3] = std::array::from_fn(|i| q0[i] + d2[i] * t);
    length(sub(closest_p, closest_q))
}

/// Vertices of the convex region shared by all `boxes`: every point where
/// three of their face planes meet that lies inside every box (within
/// `tolerance`). Empty when the boxes share no region.
#[must_use]
pub fn intersection_vertices(boxes: &[Obb], tolerance: f64) -> Vec<[f64; 3]> {
    let planes: Vec<_> = boxes.iter().flat_map(Obb::planes).collect();
    polytope_vertices(&planes, tolerance)
}

/// Vertices of the convex region where every half-space `normal · x <= offset`
/// holds (within `tolerance`). Empty when the region is empty.
#[must_use]
pub fn polytope_vertices(planes: &[([f64; 3], f64)], tolerance: f64) -> Vec<[f64; 3]> {
    let mut vertices = Vec::new();
    for i in 0..planes.len() {
        for j in i + 1..planes.len() {
            for k in j + 1..planes.len() {
                let ((a, da), (b, db), (c, dc)) = (planes[i], planes[j], planes[k]);
                let (bc, ca, ab) = (cross(b, c), cross(c, a), cross(a, b));
                let determinant = linalg::Mat3::from_rows([a, b, c]).determinant();
                if determinant.abs() < ORIENTATION_TOLERANCE {
                    continue;
                }
                let point: [f64; 3] =
                    std::array::from_fn(|n| (da * bc[n] + db * ca[n] + dc * ab[n]) / determinant);
                if planes
                    .iter()
                    .all(|(normal, offset)| dot(*normal, point) <= offset + tolerance)
                {
                    vertices.push(point);
                }
            }
        }
    }
    vertices
}

/// World axis-aligned bounds of the region shared by all `boxes`, or `None`
/// when it is empty or no thicker than `tolerance` across any box face.
#[must_use]
pub fn common_region(boxes: &[Obb], tolerance: f64) -> Option<([f64; 3], [f64; 3])> {
    let points = intersection_vertices(boxes, tolerance);
    if points.is_empty() {
        return None;
    }
    let thick = boxes.iter().flat_map(|obb| obb.axes).all(|direction| {
        let (low, high) =
            points
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                    let along = dot(*point, direction);
                    (low.min(along), high.max(along))
                });
        high - low > tolerance
    });
    if !thick {
        return None;
    }
    let (min, max) = points.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(min, max), point| {
            (
                std::array::from_fn(|i| f64::min(min[i], point[i])),
                std::array::from_fn(|i| f64::max(max[i], point[i])),
            )
        },
    );
    Some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1.0e-9)
    }

    #[test]
    fn axis_angle_turns_x_into_y_about_z() {
        let rotation = axis_angle([0.0, 0.0, 1.0], 90.0).unwrap();
        assert!(close(apply(&rotation, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]));
        assert!(close(
            apply_transposed(&rotation, [0.0, 1.0, 0.0]),
            [1.0, 0.0, 0.0]
        ));
    }

    #[test]
    fn from_axes_orthogonalises_x_against_z() {
        let rotation = from_axes([1.0, 0.0, 1.0], [0.0, 0.0, 2.0]).unwrap();
        assert!(close(axis(&rotation, 0), [1.0, 0.0, 0.0]));
        assert!(close(axis(&rotation, 1), [0.0, 1.0, 0.0]));
        assert!(from_axes([0.0, 0.0, 1.0], [0.0, 0.0, 5.0]).is_none());
    }

    #[test]
    fn tilted_boxes_touch_overlap_and_separate() {
        let tilted = axis_angle([0.0, 1.0, 0.0], 45.0).unwrap();
        let floor = Obb::new(
            [0.0; 3],
            &IDENTITY,
            [-100.0, -100.0, -10.0],
            [100.0, 100.0, 0.0],
        );
        let half_diagonal = 10.0 * std::f64::consts::SQRT_2;
        let cube = |z: f64| Obb::new([0.0, 0.0, z], &tilted, [-10.0; 3], [10.0; 3]);
        assert!(cube(half_diagonal).separation(&floor).abs() < 1.0e-9);
        assert!(cube(half_diagonal - 1.0).separation(&floor) < -0.5);
        assert!((cube(half_diagonal + 3.0).separation(&floor) - 3.0).abs() < 1.0e-9);
        let (min, max) = cube(0.0).world_bounds();
        assert!(close(min, [-half_diagonal, -10.0, -half_diagonal]));
        assert!(close(max, [half_diagonal, 10.0, half_diagonal]));
    }
}
