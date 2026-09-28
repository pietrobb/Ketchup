//! Rigid part frames and oriented bounding boxes.
//!
//! A part's local frame maps a local point `p` to `at + R·p`. `R` is a proper
//! rotation stored row-major; its columns are the part's local x, y and z axes
//! expressed in world coordinates.

pub type Mat3 = [[f64; 3]; 3];

pub const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

const ORIENTATION_TOLERANCE: f64 = 1.0e-9;

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
    std::array::from_fn(|row| dot(rotation[row], vector))
}

#[must_use]
pub fn apply_transposed(rotation: &Mat3, vector: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|column| (0..3).map(|row| rotation[row][column] * vector[row]).sum())
}

#[must_use]
pub fn multiply(left: &Mat3, right: &Mat3) -> Mat3 {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| left[row][k] * right[k][column]).sum())
    })
}

/// Inverse of a rotation.
#[must_use]
pub fn transposed(rotation: &Mat3) -> Mat3 {
    std::array::from_fn(|row| std::array::from_fn(|column| rotation[column][row]))
}

/// Column `index` of `rotation`: the local axis in world coordinates.
#[must_use]
pub fn axis(rotation: &Mat3, index: usize) -> [f64; 3] {
    std::array::from_fn(|row| rotation[row][index])
}

#[must_use]
pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[must_use]
pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[must_use]
pub fn length(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}

/// Unit vector, or `None` for a zero or non-finite vector.
#[must_use]
pub fn normalized(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = length(vector);
    (length.is_finite() && length > ORIENTATION_TOLERANCE).then(|| vector.map(|v| v / length))
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
            .map(|axis| dot(delta, axis).abs() - self.radius_along(axis) - other.radius_along(axis))
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// Half-spaces `normal · x <= offset` whose intersection is the box.
    fn planes(&self) -> [([f64; 3], f64); 6] {
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

    /// Where a face of this box lies flat against a face of `other` (opposite
    /// normals, same plane within `tolerance`) over a patch whose average
    /// width exceeds `tolerance`. Faces are searched x-, x+ ... in this box's
    /// order `(axis, positive)`: x+ before x-, then y, then z.
    #[must_use]
    pub fn face_contact(&self, other: &Self, tolerance: f64) -> Option<FaceContact> {
        for axis in 0..3 {
            for positive in [true, false] {
                let sign = if positive { 1.0 } else { -1.0 };
                let normal = self.axes[axis].map(|value| value * sign);
                let face_centre: [f64; 3] =
                    std::array::from_fn(|i| self.centre[i] + normal[i] * self.half[axis]);
                let (u_axis, v_axis) = in_plane_axes(axis);
                let (u, v) = (self.axes[u_axis], self.axes[v_axis]);
                for other_axis in 0..3 {
                    let alignment = dot(normal, other.axes[other_axis]);
                    if alignment.abs() < 1.0 - ORIENTATION_TOLERANCE {
                        continue;
                    }
                    // The touching face of `other` faces back against `normal`.
                    let other_positive = alignment < 0.0;
                    let other_sign = if other_positive { 1.0 } else { -1.0 };
                    let other_centre: [f64; 3] = std::array::from_fn(|i| {
                        other.centre[i]
                            + other.axes[other_axis][i] * other_sign * other.half[other_axis]
                    });
                    let offset: [f64; 3] =
                        std::array::from_fn(|i| other_centre[i] - face_centre[i]);
                    if dot(offset, normal).abs() > tolerance {
                        continue;
                    }
                    let (p_axis, q_axis) = in_plane_axes(other_axis);
                    let corners =
                        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(p, q)| {
                            let corner: [f64; 3] = std::array::from_fn(|i| {
                                offset[i]
                                    + other.axes[p_axis][i] * p * other.half[p_axis]
                                    + other.axes[q_axis][i] * q * other.half[q_axis]
                            });
                            [dot(corner, u), dot(corner, v)]
                        });
                    let patch =
                        clip_to_rectangle(corners.to_vec(), [self.half[u_axis], self.half[v_axis]]);
                    let (min, max) = bounds_2d(&patch);
                    let longest = (max[0] - min[0]).max(max[1] - min[1]);
                    if longest <= tolerance || polygon_area(&patch) <= tolerance * longest {
                        continue;
                    }
                    let world = |[pu, pv]: [f64; 2]| -> [f64; 3] {
                        std::array::from_fn(|i| face_centre[i] + u[i] * pu + v[i] * pv)
                    };
                    return Some(FaceContact {
                        face: (axis, positive),
                        other_face: (other_axis, other_positive),
                        normal,
                        u,
                        v,
                        origin: world(min),
                        size: [max[0] - min[0], max[1] - min[1]],
                        points: patch.into_iter().map(world).collect(),
                    });
                }
            }
        }
        None
    }
}

/// A flat patch where two boxes touch. Faces are `(local axis, positive)`;
/// `u`/`v` are the first box's in-plane face axes in world, and the patch's
/// bounding rectangle along them starts at world `origin` with `size`.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceContact {
    pub face: (usize, bool),
    pub other_face: (usize, bool),
    pub normal: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub origin: [f64; 3],
    pub size: [f64; 2],
    /// World corners of the convex patch.
    pub points: Vec<[f64; 3]>,
}

/// Face (u, v) axes: x faces use (y, z), y faces (x, z), z faces (x, y).
const fn in_plane_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// Clips a convex polygon to the rectangle `[-half, half]`.
fn clip_to_rectangle(mut polygon: Vec<[f64; 2]>, half: [f64; 2]) -> Vec<[f64; 2]> {
    for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
        let inside = |point: [f64; 2]| sign * point[axis] <= half[axis];
        let mut clipped = Vec::with_capacity(polygon.len() + 1);
        for (index, &current) in polygon.iter().enumerate() {
            let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
            if inside(current) != inside(previous) {
                let t = (sign * half[axis] - previous[axis]) / (current[axis] - previous[axis]);
                clipped.push(std::array::from_fn(|i| {
                    previous[i] + (current[i] - previous[i]) * t
                }));
            }
            if inside(current) {
                clipped.push(current);
            }
        }
        polygon = clipped;
        if polygon.is_empty() {
            break;
        }
    }
    polygon
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

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let twice: f64 = (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum();
    twice.abs() * 0.5
}

/// Vertices of the convex region shared by all `boxes`: every point where
/// three of their face planes meet that lies inside every box (within
/// `tolerance`). Empty when the boxes share no region.
#[must_use]
pub fn intersection_vertices(boxes: &[Obb], tolerance: f64) -> Vec<[f64; 3]> {
    let planes: Vec<_> = boxes.iter().flat_map(Obb::planes).collect();
    let mut vertices = Vec::new();
    for i in 0..planes.len() {
        for j in i + 1..planes.len() {
            for k in j + 1..planes.len() {
                let ((a, da), (b, db), (c, dc)) = (planes[i], planes[j], planes[k]);
                let (bc, ca, ab) = (cross(b, c), cross(c, a), cross(a, b));
                let determinant = dot(a, bc);
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
