use ketchup_core::document::{CanonicalError, Transform};
use ketchup_interaction::Vec3;

pub fn translated_transform(
    transform: Transform,
    delta_mm: Vec3,
) -> Result<Transform, CanonicalError> {
    let mut matrix = *transform.matrix();
    matrix[3] += delta_mm.x;
    matrix[7] += delta_mm.y;
    matrix[11] += delta_mm.z;
    Transform::from_matrix(matrix)
}

fn inverse_affine_transform(transform: Transform) -> Option<Transform> {
    let matrix = transform.matrix();
    let [a, b, c, d, e, f, g, h, i] = [
        matrix[0], matrix[1], matrix[2], matrix[4], matrix[5], matrix[6], matrix[8], matrix[9],
        matrix[10],
    ];
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !determinant.is_finite() || determinant.abs() <= 1.0e-12 {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let inverse_basis = [
        (e * i - f * h) * inverse_determinant,
        (c * h - b * i) * inverse_determinant,
        (b * f - c * e) * inverse_determinant,
        (f * g - d * i) * inverse_determinant,
        (a * i - c * g) * inverse_determinant,
        (c * d - a * f) * inverse_determinant,
        (d * h - e * g) * inverse_determinant,
        (b * g - a * h) * inverse_determinant,
        (a * e - b * d) * inverse_determinant,
    ];
    let translation = [matrix[3], matrix[7], matrix[11]];
    let mut inverse = [0.0; 16];
    for row in 0..3 {
        inverse[row * 4] = inverse_basis[row * 3];
        inverse[row * 4 + 1] = inverse_basis[row * 3 + 1];
        inverse[row * 4 + 2] = inverse_basis[row * 3 + 2];
        inverse[row * 4 + 3] = -(0..3)
            .map(|column| inverse_basis[row * 3 + column] * translation[column])
            .sum::<f64>();
    }
    inverse[15] = 1.0;
    Transform::from_matrix(inverse).ok()
}

pub fn rotation_in_parent_space(
    world_rotation: Transform,
    parent_world_transform: Transform,
    local_transform: Transform,
) -> Option<Transform> {
    let parent_inverse = inverse_affine_transform(parent_world_transform)?;
    let transformed = parent_inverse
        .compose(world_rotation)
        .compose(parent_world_transform)
        .compose(local_transform);
    Transform::from_matrix(*transformed.matrix()).ok()
}

pub fn world_plane_mirror_transform(
    origin_mm: Vec3,
    normal: Vec3,
) -> Result<Transform, CanonicalError> {
    let normal_length = vector_length(normal);
    if !normal_length.is_finite() || normal_length <= f64::EPSILON {
        return Err(CanonicalError::InvalidTransform);
    }
    let unit = normal * (1.0 / normal_length);
    let components = [unit.x, unit.y, unit.z];
    let origin = [origin_mm.x, origin_mm.y, origin_mm.z];
    if origin.iter().any(|value| !value.is_finite()) {
        return Err(CanonicalError::InvalidTransform);
    }
    let offset = 2.0
        * components
            .iter()
            .zip(origin)
            .map(|(normal, coordinate)| normal * coordinate)
            .sum::<f64>();
    let mut matrix = [0.0; 16];
    for row in 0..3 {
        for column in 0..3 {
            matrix[row * 4 + column] = if row == column { 1.0 } else { 0.0 };
            matrix[row * 4 + column] -= 2.0 * components[row] * components[column];
        }
        matrix[row * 4 + 3] = components[row] * offset;
    }
    matrix[15] = 1.0;
    Transform::from_matrix(matrix)
}

pub fn world_axis_rotation_transform(
    centre_mm: Vec3,
    axis: Vec3,
    angle_degrees: f64,
) -> Result<Transform, CanonicalError> {
    let axis_length = vector_length(axis);
    if !angle_degrees.is_finite() || !axis_length.is_finite() || axis_length <= f64::EPSILON {
        return Err(CanonicalError::InvalidTransform);
    }
    let (sin, cos) = angle_degrees.to_radians().sin_cos();
    let unit = axis * (1.0 / axis_length);
    let one_minus_cos = 1.0 - cos;
    let basis = [
        [
            cos + unit.x * unit.x * one_minus_cos,
            unit.x * unit.y * one_minus_cos - unit.z * sin,
            unit.x * unit.z * one_minus_cos + unit.y * sin,
        ],
        [
            unit.y * unit.x * one_minus_cos + unit.z * sin,
            cos + unit.y * unit.y * one_minus_cos,
            unit.y * unit.z * one_minus_cos - unit.x * sin,
        ],
        [
            unit.z * unit.x * one_minus_cos - unit.y * sin,
            unit.z * unit.y * one_minus_cos + unit.x * sin,
            cos + unit.z * unit.z * one_minus_cos,
        ],
    ];
    let centre = [centre_mm.x, centre_mm.y, centre_mm.z];
    let mut matrix = [0.0; 16];
    for row in 0..3 {
        let rotated = (0..3).map(|column| basis[row][column] * centre[column]);
        matrix[row * 4] = basis[row][0];
        matrix[row * 4 + 1] = basis[row][1];
        matrix[row * 4 + 2] = basis[row][2];
        matrix[row * 4 + 3] = centre[row] - rotated.sum::<f64>();
    }
    matrix[15] = 1.0;
    Transform::from_matrix(matrix)
}

pub fn vector_length(vector: Vec3) -> f64 {
    (vector.x * vector.x + vector.y * vector.y + vector.z * vector.z).sqrt()
}
