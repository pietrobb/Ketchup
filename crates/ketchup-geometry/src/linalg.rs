//! The one linear algebra of Kečup: 3-vectors, 3×3 matrices, affine
//! transforms, frames and cubic Bézier curves.
//!
//! Every crate uses these instead of writing its own `dot`, `cross` or
//! determinant. [`Mat3::is_singular`] is the only singularity test: the
//! determinant is not finite or at most [`ketchup_tolerance::NEGLIGIBLE`]
//! times the cube of the largest entry, so a uniformly scaled matrix is
//! singular exactly when the unscaled one is.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use ketchup_tolerance::NEGLIGIBLE;

/// A matrix for which [`Mat3::is_singular`] holds has no inverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Singular;

impl std::fmt::Display for Singular {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the matrix is singular")
    }
}

impl std::error::Error for Singular {}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    #[must_use]
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    #[must_use]
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (self - other).length()
    }

    /// The unit vector in the same direction, or `None` when the length is
    /// not finite or at most [`NEGLIGIBLE`].
    #[must_use]
    pub fn normalized(self) -> Option<Self> {
        normalize(self)
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    #[must_use]
    pub fn lerp(self, other: Self, t: f64) -> Self {
        self + (other - self) * t
    }

    #[must_use]
    pub fn component(self, axis: usize) -> f64 {
        self.to_array()[axis]
    }
}

impl From<[f64; 3]> for Vec3 {
    fn from([x, y, z]: [f64; 3]) -> Self {
        Self::new(x, y, z)
    }
}

impl From<Vec3> for [f64; 3] {
    fn from(vector: Vec3) -> Self {
        vector.to_array()
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<f64> for Vec3 {
    type Output = Self;

    fn div(self, rhs: f64) -> Self {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// A 3-vector stored either as [`Vec3`] or as `[f64; 3]`; the free
/// functions below accept and return whichever form the caller uses.
pub trait Vector3: Copy + From<Vec3> + Into<Vec3> {}

impl Vector3 for Vec3 {}

impl Vector3 for [f64; 3] {}

#[must_use]
pub fn dot<V: Vector3>(left: V, right: V) -> f64 {
    left.into().dot(right.into())
}

#[must_use]
pub fn cross<V: Vector3>(left: V, right: V) -> V {
    V::from(left.into().cross(right.into()))
}

#[must_use]
pub fn add<V: Vector3>(left: V, right: V) -> V {
    V::from(left.into() + right.into())
}

#[must_use]
pub fn sub<V: Vector3>(left: V, right: V) -> V {
    V::from(left.into() - right.into())
}

#[must_use]
pub fn scale<V: Vector3>(vector: V, factor: f64) -> V {
    V::from(vector.into() * factor)
}

#[must_use]
pub fn length<V: Vector3>(vector: V) -> f64 {
    vector.into().length()
}

#[must_use]
pub fn length_squared<V: Vector3>(vector: V) -> f64 {
    vector.into().length_squared()
}

#[must_use]
pub fn distance<V: Vector3>(left: V, right: V) -> f64 {
    left.into().distance(right.into())
}

/// The unit vector, or `None` when the length is not finite or at most
/// [`NEGLIGIBLE`].
#[must_use]
pub fn normalize<V: Vector3>(vector: V) -> Option<V> {
    normalize_within(vector, NEGLIGIBLE)
}

/// The unit vector, or `None` when the length is not finite or at most
/// `min_length` (a caller with a domain tolerance passes it here).
#[must_use]
pub fn normalize_within<V: Vector3>(vector: V, min_length: f64) -> Option<V> {
    let vector = vector.into();
    let length = vector.length();
    (length.is_finite() && length > min_length).then(|| V::from(vector / length))
}

/// The centre of the circle through `a`, `b` and `c`, or `None` when the
/// three points are collinear (the sine of the angle at `a` is at most
/// [`NEGLIGIBLE`]) or not finite.
#[must_use]
pub fn circumcenter<V: Vector3>(a: V, b: V, c: V) -> Option<V> {
    let (a, b, c): (Vec3, Vec3, Vec3) = (a.into(), b.into(), c.into());
    let (ab, ac) = (b - a, c - a);
    let normal = ab.cross(ac);
    let denominator = 2.0 * normal.length_squared();
    let floor = NEGLIGIBLE * ab.length_squared() * ac.length_squared();
    if !denominator.is_finite() || denominator <= floor || denominator == 0.0 {
        return None;
    }
    let offset = (normal.cross(ab) * ac.length_squared() + ac.cross(normal) * ab.length_squared())
        / denominator;
    Some(V::from(a + offset))
}

/// The dot product of two plane vectors.
#[must_use]
pub fn dot2(left: [f64; 2], right: [f64; 2]) -> f64 {
    dot([left[0], left[1], 0.0], [right[0], right[1], 0.0])
}

/// The z component of the cross product of two plane vectors (positive when
/// `right` turns counter-clockwise from `left`).
#[must_use]
pub fn cross2(left: [f64; 2], right: [f64; 2]) -> f64 {
    cross([left[0], left[1], 0.0], [right[0], right[1], 0.0])[2]
}

/// The distance in the plane from `point` to the nearest point of the segment
/// from `start` to `end`.
#[must_use]
pub fn point_segment_distance2(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = [end[0] - start[0], end[1] - start[1]];
    let from_start = [point[0] - start[0], point[1] - start[1]];
    let length_squared = dot2(direction, direction);
    let parameter = if length_squared > 0.0 {
        (dot2(from_start, direction) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (from_start[0] - parameter * direction[0]).hypot(from_start[1] - parameter * direction[1])
}

/// The distance from `point` to the closed outline through `outline` (the last
/// point joins the first): negative inside, positive outside, by the even-odd
/// rule. `None` for fewer than two points.
#[must_use]
pub fn signed_outline_distance(outline: &[[f64; 2]], point: [f64; 2]) -> Option<f64> {
    if outline.len() < 2 {
        return None;
    }
    let edges = || outline.iter().zip(outline.iter().cycle().skip(1));
    let distance = edges()
        .map(|(start, end)| point_segment_distance2(point, *start, *end))
        .fold(f64::INFINITY, f64::min);
    let inside = edges()
        .filter(|(start, end)| {
            (start[1] > point[1]) != (end[1] > point[1])
                && point[0]
                    < start[0] + (point[1] - start[1]) * (end[0] - start[0]) / (end[1] - start[1])
        })
        .count()
        % 2
        == 1;
    Some(if inside { -distance } else { distance })
}

/// A 3×3 matrix stored by rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub rows: [[f64; 3]; 3],
}

impl Mat3 {
    pub const IDENTITY: Self = Self::from_rows([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    #[must_use]
    pub const fn from_rows(rows: [[f64; 3]; 3]) -> Self {
        Self { rows }
    }

    #[must_use]
    pub fn from_columns(columns: [[f64; 3]; 3]) -> Self {
        Self::from_rows(columns).transpose()
    }

    #[must_use]
    pub fn transpose(self) -> Self {
        let m = self.rows;
        Self::from_rows([
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ])
    }

    #[must_use]
    pub fn determinant(self) -> f64 {
        let [first, second, third] = self.rows;
        dot(first, cross(second, third))
    }

    /// The largest absolute entry; zero for the zero matrix.
    #[must_use]
    pub fn max_abs_entry(self) -> f64 {
        self.rows
            .iter()
            .flatten()
            .map(|value| value.abs())
            .fold(0.0, f64::max)
    }

    /// Whether the matrix has no usable inverse: an entry or the determinant
    /// is not finite, or the determinant is at most [`NEGLIGIBLE`] times the
    /// cube of the largest entry.
    #[must_use]
    pub fn is_singular(self) -> bool {
        let determinant = self.determinant();
        let scale = self.max_abs_entry();
        !determinant.is_finite()
            || !scale.is_finite()
            || scale == 0.0
            || determinant.abs() <= NEGLIGIBLE * scale.powi(3)
    }

    /// The inverse, or [`Singular`] when [`Self::is_singular`].
    pub fn inverse(self) -> Result<Self, Singular> {
        if self.is_singular() {
            return Err(Singular);
        }
        let determinant = self.determinant();
        let [first, second, third] = self.rows;
        // The columns of the inverse are the cross products of the rows.
        let adjugate_columns = [
            cross(second, third),
            cross(third, first),
            cross(first, second),
        ];
        let inverse = Self::from_columns(adjugate_columns);
        Ok(Self::from_rows(
            inverse.rows.map(|row| scale(row, 1.0 / determinant)),
        ))
    }

    #[must_use]
    pub fn mul_vec(self, vector: Vec3) -> Vec3 {
        let [first, second, third] = self.rows;
        Vec3::new(
            Vec3::from(first).dot(vector),
            Vec3::from(second).dot(vector),
            Vec3::from(third).dot(vector),
        )
    }

    /// Whether the rows are orthonormal within `tolerance`.
    #[must_use]
    pub fn is_orthonormal(self, tolerance: f64) -> bool {
        let product = self * self.transpose();
        (0..3).all(|row| {
            (0..3).all(|column| {
                let expected = if row == column { 1.0 } else { 0.0 };
                (product.rows[row][column] - expected).abs() <= tolerance
            })
        })
    }
}

impl Mul for Mat3 {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        let columns = other.transpose().rows;
        Self::from_rows(self.rows.map(|row| columns.map(|column| dot(row, column))))
    }
}

/// An affine transform `p ↦ linear·p + translation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine3 {
    pub linear: Mat3,
    pub translation: Vec3,
}

impl Affine3 {
    pub const IDENTITY: Self = Self {
        linear: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// From a row-major 4×4 matrix; the bottom row is ignored.
    #[must_use]
    pub fn from_row_major(matrix: [f64; 16]) -> Self {
        Self {
            linear: Mat3::from_rows([
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ]),
            translation: Vec3::new(matrix[3], matrix[7], matrix[11]),
        }
    }

    /// A row-major 4×4 matrix with bottom row `[0, 0, 0, 1]`.
    #[must_use]
    pub fn to_row_major(self) -> [f64; 16] {
        let [first, second, third] = self.linear.rows;
        let t = self.translation;
        [
            first[0], first[1], first[2], t.x, second[0], second[1], second[2], t.y, third[0],
            third[1], third[2], t.z, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    /// From a column-major 4×4 matrix (glTF layout); the bottom row is
    /// ignored.
    #[must_use]
    pub fn from_column_major(matrix: [f64; 16]) -> Self {
        let mut row_major = [0.0; 16];
        for row in 0..4 {
            for column in 0..4 {
                row_major[row * 4 + column] = matrix[column * 4 + row];
            }
        }
        Self::from_row_major(row_major)
    }

    /// A column-major 4×4 matrix (glTF layout).
    #[must_use]
    pub fn to_column_major(self) -> [f64; 16] {
        let row_major = self.to_row_major();
        let mut column_major = [0.0; 16];
        for row in 0..4 {
            for column in 0..4 {
                column_major[column * 4 + row] = row_major[row * 4 + column];
            }
        }
        column_major
    }

    #[must_use]
    pub fn determinant(self) -> f64 {
        self.linear.determinant()
    }

    #[must_use]
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.linear.mul_vec(point) + self.translation
    }

    #[must_use]
    pub fn transform_vector(self, vector: Vec3) -> Vec3 {
        self.linear.mul_vec(vector)
    }

    /// `self ∘ inner`: apply `inner` first.
    #[must_use]
    pub fn compose(self, inner: Self) -> Self {
        Self {
            linear: self.linear.mul(inner.linear),
            translation: self.transform_point(inner.translation),
        }
    }

    pub fn invert(self) -> Result<Self, Singular> {
        let linear = self.linear.inverse()?;
        Ok(Self {
            linear,
            translation: -linear.mul_vec(self.translation),
        })
    }
}

/// An origin with three axes, stored flat as
/// `[origin, x, y, z]` (twelve numbers) in the worker protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Frame {
    pub const WORLD: Self = Self {
        origin: Vec3::ZERO,
        x: Vec3::X,
        y: Vec3::Y,
        z: Vec3::Z,
    };

    #[must_use]
    pub fn axis(self, index: usize) -> Vec3 {
        [self.x, self.y, self.z][index]
    }

    /// The world point at local coordinates `local`.
    #[must_use]
    pub fn point(self, local: Vec3) -> Vec3 {
        self.origin + self.vector(local)
    }

    /// The world direction of the local direction `local`.
    #[must_use]
    pub fn vector(self, local: Vec3) -> Vec3 {
        self.x * local.x + self.y * local.y + self.z * local.z
    }

    /// The local coordinates of world `point` for an orthonormal frame.
    #[must_use]
    pub fn local(self, point: Vec3) -> Vec3 {
        let offset = point - self.origin;
        Vec3::new(offset.dot(self.x), offset.dot(self.y), offset.dot(self.z))
    }

    /// The origin and the x, y and z axes as arrays, in that order.
    #[must_use]
    pub fn to_vectors(self) -> [[f64; 3]; 4] {
        [self.origin, self.x, self.y, self.z].map(Vec3::to_array)
    }

    #[must_use]
    pub fn to_array(self) -> [f64; 12] {
        let [o, x, y, z] = self.to_vectors();
        [
            o[0], o[1], o[2], x[0], x[1], x[2], y[0], y[1], y[2], z[0], z[1], z[2],
        ]
    }

    /// The transform from local to world coordinates: the axes are the
    /// columns of the linear part and the origin is the translation.
    #[must_use]
    pub fn to_affine(self) -> Affine3 {
        Affine3 {
            linear: Mat3::from_columns([self.x, self.y, self.z].map(Vec3::to_array)),
            translation: self.origin,
        }
    }
}

impl From<[f64; 12]> for Frame {
    fn from(flat: [f64; 12]) -> Self {
        let vector = |start: usize| Vec3::new(flat[start], flat[start + 1], flat[start + 2]);
        Self {
            origin: vector(0),
            x: vector(3),
            y: vector(6),
            z: vector(9),
        }
    }
}

impl From<Frame> for [f64; 12] {
    fn from(frame: Frame) -> Self {
        frame.to_array()
    }
}

/// A cubic Bézier curve in `N` dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier<const N: usize> {
    pub points: [[f64; N]; 4],
}

impl<const N: usize> CubicBezier<N> {
    #[must_use]
    pub const fn new(points: [[f64; N]; 4]) -> Self {
        Self { points }
    }

    /// The control points weighted by `weights`.
    fn combine(&self, weights: [f64; 4]) -> [f64; N] {
        std::array::from_fn(|axis| {
            weights
                .iter()
                .zip(&self.points)
                .map(|(weight, point)| weight * point[axis])
                .sum()
        })
    }

    #[must_use]
    pub fn eval(&self, t: f64) -> [f64; N] {
        let s = 1.0 - t;
        self.combine([s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t])
    }

    #[must_use]
    pub fn derivative(&self, t: f64) -> [f64; N] {
        let s = 1.0 - t;
        self.combine([
            -3.0 * s * s,
            3.0 * s * s - 6.0 * s * t,
            6.0 * s * t - 3.0 * t * t,
            3.0 * t * t,
        ])
    }

    /// The power-basis coefficients `[c0, c1, c2, c3]` of `axis`, so that
    /// the coordinate is `c0 + c1·t + c2·t² + c3·t³`.
    #[must_use]
    pub fn power_coefficients(&self, axis: usize) -> [f64; 4] {
        let [p0, p1, p2, p3] = self.points.map(|point| point[axis]);
        [
            p0,
            3.0 * (p1 - p0),
            3.0 * (p0 - 2.0 * p1 + p2),
            p3 - p0 + 3.0 * (p1 - p2),
        ]
    }
}

/// A plane Bézier usable as a sweep path: its control polygon length and the
/// unit tangents at its start and end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForwardBezier {
    pub control_length: f64,
    pub start_tangent: [f64; 2],
    pub end_tangent: [f64; 2],
}

impl CubicBezier<2> {
    /// The curve as a sweep path when it runs forward along its chord: both
    /// handles longer than `min_handle`, the first control point ahead of
    /// the start, the second not behind the first and before the end.
    /// `None` for a curve that turns back or a non-finite control length.
    #[must_use]
    pub fn forward(&self, min_handle: f64) -> Option<ForwardBezier> {
        let [start, control_1, control_2, end] = self.points;
        let delta = |from: [f64; 2], to: [f64; 2]| [to[0] - from[0], to[1] - from[1]];
        let chord = delta(start, end);
        let start_handle = delta(start, control_1);
        let end_handle = delta(control_2, end);
        let middle = delta(control_1, control_2);
        let start_length = start_handle[0].hypot(start_handle[1]);
        let end_length = end_handle[0].hypot(end_handle[1]);
        let control_length = start_length + middle[0].hypot(middle[1]) + end_length;
        let projection_1 = dot2(start_handle, chord);
        let projection_2 = dot2(delta(start, control_2), chord);
        if !control_length.is_finite()
            || start_length <= min_handle
            || end_length <= min_handle
            || projection_1 <= 0.0
            || projection_2 < projection_1
            || projection_2 >= dot2(chord, chord)
        {
            return None;
        }
        Some(ForwardBezier {
            control_length,
            start_tangent: start_handle.map(|value| value / start_length),
            end_tangent: end_handle.map(|value| value / end_length),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() <= 1.0e-9
    }

    fn close_affine(left: Affine3, right: Affine3) -> bool {
        left.to_row_major()
            .iter()
            .zip(right.to_row_major())
            .all(|(a, b)| close(*a, b))
    }

    fn sample() -> Affine3 {
        // Rotation about z by 30°, non-uniform scale and a translation.
        let (sin, cos) = 30.0_f64.to_radians().sin_cos();
        Affine3 {
            linear: Mat3::from_rows([
                [cos * 2.0, -sin, 0.0],
                [sin * 2.0, cos, 0.0],
                [0.0, 0.0, 0.5],
            ]),
            translation: Vec3::new(10.0, -4.0, 7.5),
        }
    }

    #[test]
    fn vector_products_follow_the_right_hand_rule() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(cross([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        assert!(close(dot([1.0, 2.0, 3.0], [4.0, -5.0, 6.0]), 12.0));
        assert!(close(length([3.0, 4.0, 12.0]), 13.0));
        assert_eq!(normalize([0.0, 0.0, 0.0]), None);
        assert_eq!(normalize([0.0, 0.0, 2.0]), Some([0.0, 0.0, 1.0]));
        assert_eq!(normalize_within(Vec3::X * 0.5, 0.5), None);
        assert_eq!(sub(Vec3::X, Vec3::Y), Vec3::new(1.0, -1.0, 0.0));
        assert!(close(cross2([1.0, 0.0], [0.0, 1.0]), 1.0));
        assert!(close(dot2([1.0, 2.0], [3.0, 4.0]), 11.0));
    }

    #[test]
    fn circumcenter_is_equidistant_and_rejects_collinear_points() {
        let [a, b, c] = [[1.0, 0.0, 2.0], [-1.0, 0.0, 2.0], [0.0, 1.0, 2.0]];
        let center = circumcenter(a, b, c).unwrap();
        assert!(close(distance(center, [0.0, 0.0, 2.0]), 0.0));
        let tilted = [
            Vec3::new(3.0, 1.0, 0.0),
            Vec3::new(0.0, 4.0, 2.0),
            Vec3::new(-1.0, 0.0, 5.0),
        ];
        let center = circumcenter(tilted[0], tilted[1], tilted[2]).unwrap();
        let radius = center.distance(tilted[0]);
        assert!(
            tilted
                .iter()
                .all(|point| close(center.distance(*point), radius))
        );
        assert_eq!(
            circumcenter([0.0; 3], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]),
            None
        );
        assert_eq!(circumcenter([0.0; 3], [0.0; 3], [1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn matrix_inverse_round_trips_and_rejects_singular() {
        let matrix = sample().linear;
        let product = matrix.mul(matrix.inverse().unwrap());
        assert!(product.is_orthonormal(1.0e-12));
        let flat = Mat3::from_rows([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]);
        assert_eq!(flat.inverse(), Err(Singular));
        let tiny = Mat3::from_rows([[1.0e-4, 0.0, 0.0], [0.0, 1.0e-4, 0.0], [0.0, 0.0, 1.0e-4]]);
        assert!(tiny.inverse().is_ok(), "a uniform scale is never singular");
        assert_eq!(Mat3::from_rows([[0.0; 3]; 3]).inverse(), Err(Singular));
        let nan = Mat3::from_rows([[f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(nan.inverse(), Err(Singular));
    }

    #[test]
    fn affine_inverse_composes_to_identity_and_maps_points_back() {
        let transform = sample();
        let inverse = transform.invert().unwrap();
        assert!(close_affine(transform.compose(inverse), Affine3::IDENTITY));
        assert!(close_affine(inverse.compose(transform), Affine3::IDENTITY));
        let point = Vec3::new(1.0, 2.0, 3.0);
        let back = inverse.transform_point(transform.transform_point(point));
        assert!(close(back.distance(point), 0.0));
    }

    #[test]
    fn affine_matrix_layouts_round_trip() {
        let transform = sample();
        assert_eq!(Affine3::from_row_major(transform.to_row_major()), transform);
        assert_eq!(
            Affine3::from_column_major(transform.to_column_major()),
            transform
        );
        let column_major = transform.to_column_major();
        assert_eq!(column_major[12..15], transform.translation.to_array());
    }

    #[test]
    fn frame_round_trips_its_flat_layout_and_local_coordinates() {
        let flat = [1.0, 2.0, 3.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let frame = Frame::from(flat);
        assert_eq!(frame.to_array(), flat);
        assert_eq!(frame.axis(1), Vec3::new(-1.0, 0.0, 0.0));
        let local = Vec3::new(4.0, 5.0, 6.0);
        assert!(close(frame.local(frame.point(local)).distance(local), 0.0));
        let affine = frame.to_affine();
        assert!(close(
            affine.transform_point(local).distance(frame.point(local)),
            0.0
        ));
        assert_eq!(frame.to_vectors()[2], [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn forward_bezier_measures_its_control_polygon_and_rejects_turning_back() {
        let curve = CubicBezier::new([[0.0, 0.0], [3.0, 4.0], [7.0, 4.0], [10.0, 0.0]]);
        let forward = curve.forward(0.5).unwrap();
        assert!(close(forward.control_length, 5.0 + 4.0 + 5.0));
        assert_eq!(forward.start_tangent, [0.6, 0.8]);
        assert_eq!(forward.end_tangent, [0.6, -0.8]);
        assert_eq!(curve.forward(5.0), None);
        let back = CubicBezier::new([[0.0, 0.0], [-1.0, 4.0], [7.0, 4.0], [10.0, 0.0]]);
        assert_eq!(back.forward(0.5), None);
        let crossed = CubicBezier::new([[0.0, 0.0], [7.0, 4.0], [3.0, 4.0], [10.0, 0.0]]);
        assert_eq!(crossed.forward(0.5), None);
        let past_end = CubicBezier::new([[0.0, 0.0], [3.0, 4.0], [11.0, 4.0], [10.0, 0.0]]);
        assert_eq!(past_end.forward(0.5), None);
    }

    #[test]
    fn cubic_bezier_hits_its_end_points_and_matches_its_derivative() {
        let curve = CubicBezier::new([[0.0, 0.0], [1.0, 2.0], [3.0, 2.0], [4.0, 0.0]]);
        assert_eq!(curve.eval(0.0), [0.0, 0.0]);
        assert_eq!(curve.eval(1.0), [4.0, 0.0]);
        assert_eq!(curve.derivative(0.0), [3.0, 6.0]);
        let step = 1.0e-6;
        for t in [0.1, 0.5, 0.9] {
            let [ahead, behind] = [curve.eval(t + step), curve.eval(t - step)];
            let derivative = curve.derivative(t);
            for axis in 0..2 {
                let difference = (ahead[axis] - behind[axis]) / (2.0 * step);
                assert!((difference - derivative[axis]).abs() <= 1.0e-6);
                let [c0, c1, c2, c3] = curve.power_coefficients(axis);
                assert!(close(
                    c0 + t * (c1 + t * (c2 + t * c3)),
                    curve.eval(t)[axis]
                ));
            }
        }
    }

    #[test]
    fn signed_outline_distance_is_negative_inside_and_positive_outside_any_polygon() {
        // An L, so a point in its notch is outside although inside its bounds.
        let outline = [
            [0.0, 0.0],
            [40.0, 0.0],
            [40.0, 10.0],
            [10.0, 10.0],
            [10.0, 30.0],
            [0.0, 30.0],
        ];
        assert_eq!(signed_outline_distance(&outline, [5.0, 5.0]), Some(-5.0));
        assert_eq!(signed_outline_distance(&outline, [30.0, 4.0]), Some(-4.0));
        assert_eq!(signed_outline_distance(&outline, [30.0, 20.0]), Some(10.0));
        assert_eq!(signed_outline_distance(&outline, [-3.0, 15.0]), Some(3.0));
        assert_eq!(signed_outline_distance(&outline, [43.0, 14.0]), Some(5.0));
        assert_eq!(signed_outline_distance(&outline, [40.0, 5.0]), Some(0.0));
        assert_eq!(signed_outline_distance(&outline[..1], [0.0, 0.0]), None);
        assert_eq!(
            point_segment_distance2([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]),
            5.0
        );
    }
}
