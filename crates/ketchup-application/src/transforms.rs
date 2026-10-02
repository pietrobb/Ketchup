use ketchup_geometry::linalg::length;
use ketchup_interaction::Vec3;
use ketchup_model::document::{
    CanonicalCommand, CanonicalError, GroupId, OccurrenceId, Snapshot, Transform,
};

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

pub fn rotation_in_parent_space(
    world_rotation: Transform,
    parent_world_transform: Transform,
    local_transform: Transform,
) -> Option<Transform> {
    let parent_inverse = parent_world_transform.inverse()?;
    let transformed = parent_inverse
        .compose(world_rotation)
        .compose(parent_world_transform)
        .compose(local_transform);
    Transform::from_matrix(*transformed.matrix()).ok()
}

pub fn world_edit_in_parent_space(
    snapshot: &Snapshot,
    parent: Option<GroupId>,
    local: Transform,
    world_edit: Transform,
) -> Option<Transform> {
    let parent_world = parent.map_or(Some(Transform::identity()), |id| {
        snapshot.world_transform_for_group(id)
    })?;
    rotation_in_parent_space(world_edit, parent_world, local)
}

pub fn world_plane_mirror_transform(
    origin_mm: Vec3,
    normal: Vec3,
) -> Result<Transform, CanonicalError> {
    let normal_length = length(normal);
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

/// Why a mirrored copy of an occurrence cannot be added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MirrorCopyError {
    /// The occurrence is not in the snapshot.
    MissingOccurrence,
    /// The world placement of the occurrence's parent group is unknown.
    ParentTransform,
    /// The mirrored placement is not a valid transform in the parent.
    Transform,
}

/// Commands that add occurrence `id`: a copy of `source` mirrored by
/// `world_mirror` in the same parent, with its name, tag, visibility and colour.
pub fn mirrored_copy_commands(
    snapshot: &Snapshot,
    source: OccurrenceId,
    world_mirror: Transform,
    id: OccurrenceId,
) -> Result<Vec<CanonicalCommand>, MirrorCopyError> {
    let source = snapshot
        .occurrence(source)
        .ok_or(MirrorCopyError::MissingOccurrence)?;
    let parent_transform = source
        .parent()
        .map_or(Some(Transform::identity()), |parent| {
            snapshot.world_transform_for_group(parent)
        })
        .ok_or(MirrorCopyError::ParentTransform)?;
    let transform = rotation_in_parent_space(world_mirror, parent_transform, source.transform())
        .ok_or(MirrorCopyError::Transform)?;
    let mut commands = vec![CanonicalCommand::CreateOccurrence {
        id,
        definition_id: source.definition_id(),
        name: source.name().to_owned(),
        transform,
        parent: source.parent(),
        tag: source.tag(),
        visible: source.visible(),
    }];
    if source.color().is_some() {
        commands.push(CanonicalCommand::SetOccurrenceColor {
            id,
            color: source.color(),
        });
    }
    Ok(commands)
}

pub fn world_axis_rotation_transform(
    centre_mm: Vec3,
    axis: Vec3,
    angle_degrees: f64,
) -> Result<Transform, CanonicalError> {
    let axis_length = length(axis);
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
