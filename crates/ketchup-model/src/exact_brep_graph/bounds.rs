use super::*;
use ketchup_geometry::linalg::{CubicBezier, Vec3, dot, sub};
use ketchup_tolerance::MAX_COORDINATE_MM;
use ketchup_tolerance::limits;

pub(super) fn projected_bounds(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
    tolerance_mm: f64,
) -> Result<[f64; 2], ExactBRepGraphError> {
    if !valid_bounds(target_bounds) {
        return Err(ExactBRepGraphError::UnresolvedExtent);
    }
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for x in [target_bounds[0][0], target_bounds[1][0]] {
        for y in [target_bounds[0][1], target_bounds[1][1]] {
            for z in [target_bounds[0][2], target_bounds[1][2]] {
                let projection = dot(sub([x, y, z], origin_mm), direction);
                minimum = minimum.min(projection);
                maximum = maximum.max(projection);
            }
        }
    }
    if !minimum.is_finite()
        || !maximum.is_finite()
        || maximum <= tolerance_mm
        || minimum - 1.0 < -MAX_COORDINATE_MM
        || maximum + 1.0 > MAX_COORDINATE_MM
    {
        return Err(ExactBRepGraphError::UnresolvedExtent);
    }
    Ok([minimum, maximum])
}

pub(super) fn operation_bounds(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
    node_bounds: &[Option<[[f64; 3]; 2]>],
    tolerance_mm: f64,
) -> Result<Option<[[f64; 3]; 2]>, ExactBRepGraphError> {
    match operation {
        ExactBRepOperation::Extrude {
            profile, interval, ..
        } => swept_profile_bounds(&profiles[profile.0 as usize], *interval).map(Some),
        ExactBRepOperation::ProfileCut { target, .. }
        | ExactBRepOperation::Shell { target, .. }
        | ExactBRepOperation::EdgeFinish { target, .. } => Ok(node_bounds[target.0 as usize]),
        ExactBRepOperation::FaceOffset {
            target,
            distance_bits,
            ..
        } => {
            let Some(target_bounds) = node_bounds[target.0 as usize] else {
                return Ok(None);
            };
            let distance = f64::from_bits(*distance_bits);
            if distance < 0.0 {
                return Ok(Some(target_bounds));
            }
            let expanded = [
                [0, 1, 2].map(|axis| target_bounds[0][axis] - distance),
                [0, 1, 2].map(|axis| target_bounds[1][axis] + distance),
            ];
            valid_bounds(expanded)
                .then_some(Some(expanded))
                .ok_or(ExactBRepGraphError::ResourceLimit)
        }
        ExactBRepOperation::RigidTransform {
            target,
            matrix_bits,
        } => node_bounds[target.0 as usize]
            .map(|bounds| transform_bounds(bounds, matrix_bits.map(f64::from_bits)))
            .transpose(),
        ExactBRepOperation::WeldmentJoint { first, second, .. } => Ok(node_bounds
            [first.0 as usize]
            .zip(node_bounds[second.0 as usize])
            .map(|(first, second)| bounds_union(first, second))),
        ExactBRepOperation::SurfaceTrim { target, cutter } => Ok(node_bounds[target.0 as usize]
            .zip(node_bounds[cutter.0 as usize])
            .and_then(|(target, cutter)| bounds_intersection(target, cutter))),
        ExactBRepOperation::SurfaceExtend {
            target,
            distance_bits,
        } => {
            let Some(target_bounds) = node_bounds[target.0 as usize] else {
                return Ok(None);
            };
            let distance = f64::from_bits(*distance_bits);
            let expanded = [
                [0, 1, 2].map(|axis| target_bounds[0][axis] - distance),
                [0, 1, 2].map(|axis| target_bounds[1][axis] + distance),
            ];
            valid_bounds(expanded)
                .then_some(Some(expanded))
                .ok_or(ExactBRepGraphError::ResourceLimit)
        }
        ExactBRepOperation::SurfaceKnit { surfaces, .. } => Ok(surfaces
            .iter()
            .filter_map(|surface| node_bounds[surface.0 as usize])
            .reduce(bounds_union)),
        ExactBRepOperation::SurfaceThicken {
            target,
            thickness_bits,
            ..
        } => {
            let Some(target_bounds) = node_bounds[target.0 as usize] else {
                return Ok(None);
            };
            let thickness = f64::from_bits(*thickness_bits);
            let expanded = [
                [0, 1, 2].map(|axis| target_bounds[0][axis] - thickness),
                [0, 1, 2].map(|axis| target_bounds[1][axis] + thickness),
            ];
            valid_bounds(expanded)
                .then_some(Some(expanded))
                .ok_or(ExactBRepGraphError::ResourceLimit)
        }
        ExactBRepOperation::Boolean {
            operation,
            target,
            tool,
            ..
        } => {
            let target = node_bounds[target.0 as usize];
            let tool = node_bounds[tool.0 as usize];
            Ok(match operation {
                ExactBRepBooleanOperation::Cut => target,
                ExactBRepBooleanOperation::Union => target
                    .zip(tool)
                    .map(|(target, tool)| bounds_union(target, tool)),
                ExactBRepBooleanOperation::Intersect => target
                    .zip(tool)
                    .and_then(|(target, tool)| bounds_intersection(target, tool)),
                ExactBRepBooleanOperation::Split => target
                    .zip(tool)
                    .and_then(|(target, tool)| bounds_intersection(target, tool).map(|_| target)),
            })
        }
        ExactBRepOperation::PlanarOffset {
            profile,
            distance_bits,
        } => planar_offset_profile_bounds(
            &profiles[profile.0 as usize],
            f64::from_bits(*distance_bits),
        )
        .map(Some),
        ExactBRepOperation::PlanarSurface { profile } => {
            planar_surface_bounds(&profiles[profile.0 as usize]).map(Some)
        }
        ExactBRepOperation::Sweep { profile, path } => sweep_profile_bounds(
            &profiles[profile.0 as usize],
            &profiles[path.0 as usize],
            tolerance_mm,
        )
        .map(Some),
        ExactBRepOperation::SpatialSweep { profile, path, .. } => {
            spatial_sweep_bounds(&profiles[profile.0 as usize], path, tolerance_mm).map(Some)
        }
        ExactBRepOperation::Revolve {
            profile,
            axis_start_bits,
            axis_end_bits,
            ..
        } => revolve_profile_bounds(
            &profiles[profile.0 as usize],
            axis_start_bits.map(f64::from_bits),
            axis_end_bits.map(f64::from_bits),
            tolerance_mm,
        )
        .map(Some),
        ExactBRepOperation::Loft { sections, .. }
        | ExactBRepOperation::LoftSurface { sections, .. } => {
            loft_bounds(sections, profiles).map(Some)
        }
        ExactBRepOperation::SheetMetal {
            base_mm_bits,
            thickness_bits,
            k_factor_bits,
            bends,
        } => exact_sheet_metal_shape(base_mm_bits, *thickness_bits, *k_factor_bits, bends)
            .folded_bounds_mm()
            .ok()
            .filter(|bounds| valid_bounds(*bounds))
            .map(Some)
            .ok_or(ExactBRepGraphError::ResourceLimit),
        ExactBRepOperation::ImportedExact { .. } => Ok(None),
    }
}

pub(super) fn revolve_profile_bounds(
    profile: &ExactBRepProfile,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    tolerance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let profile_bounds = planar_geometry_bounds(&profile.geometry)?;
    let axis_delta = [
        axis_end_mm[0] - axis_start_mm[0],
        axis_end_mm[1] - axis_start_mm[1],
    ];
    let axis_length = axis_delta[0].hypot(axis_delta[1]);
    if !axis_length.is_finite() || axis_length <= tolerance_mm {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let local_axis = [axis_delta[0] / axis_length, axis_delta[1] / axis_length];
    let mut minimum_projection = f64::INFINITY;
    let mut maximum_projection = f64::NEG_INFINITY;
    let mut radius = 0.0_f64;
    for x in [profile_bounds[0][0], profile_bounds[1][0]] {
        for y in [profile_bounds[0][1], profile_bounds[1][1]] {
            let relative = [x - axis_start_mm[0], y - axis_start_mm[1]];
            let projection = ketchup_geometry::linalg::dot2(relative, local_axis);
            minimum_projection = minimum_projection.min(projection);
            maximum_projection = maximum_projection.max(projection);
            radius = radius.max((relative[0] * local_axis[1] - relative[1] * local_axis[0]).abs());
        }
    }

    let frame = profile.frame();
    let axis_origin = frame
        .point(Vec3::new(axis_start_mm[0], axis_start_mm[1], 0.0))
        .to_array();
    let world_axis = frame
        .vector(Vec3::new(local_axis[0], local_axis[1], 0.0))
        .to_array();
    let start = [0, 1, 2].map(|axis| axis_origin[axis] + world_axis[axis] * minimum_projection);
    let end = [0, 1, 2].map(|axis| axis_origin[axis] + world_axis[axis] * maximum_projection);
    let bounds = [
        [0, 1, 2].map(|axis| {
            start[axis].min(end[axis])
                - radius * (1.0 - world_axis[axis] * world_axis[axis]).max(0.0).sqrt()
        }),
        [0, 1, 2].map(|axis| {
            start[axis].max(end[axis])
                + radius * (1.0 - world_axis[axis] * world_axis[axis]).max(0.0).sqrt()
        }),
    ];
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn planar_surface_bounds(
    profile: &ExactBRepProfile,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let planar = planar_geometry_bounds(&profile.geometry)?;
    let frame = profile.frame();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [planar[0][0], planar[1][0]] {
        for y in [planar[0][1], planar[1][1]] {
            let point = frame.point(Vec3::new(x, y, 0.0)).to_array();
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(point[axis]);
                bounds[1][axis] = bounds[1][axis].max(point[axis]);
            }
        }
    }
    valid_surface_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn loft_bounds(
    sections: &[ExactBRepLoftSection],
    profiles: &[ExactBRepProfile],
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for section in sections {
        let profile = profiles
            .get(section.profile.0 as usize)
            .ok_or(ExactBRepGraphError::InvalidGraph)?;
        let planar = planar_geometry_bounds(&profile.geometry)?;
        let frame = profile.frame();
        let elevation = f64::from_bits(section.elevation_bits);
        for x in [planar[0][0], planar[1][0]] {
            for y in [planar[0][1], planar[1][1]] {
                let point = frame.point(Vec3::new(x, y, elevation)).to_array();
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
    }
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn transform_bounds(
    bounds: [[f64; 3]; 2],
    matrix: [f64; 16],
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let mut transformed = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [bounds[0][0], bounds[1][0]] {
        for y in [bounds[0][1], bounds[1][1]] {
            for z in [bounds[0][2], bounds[1][2]] {
                let point = [
                    matrix[0] * x + matrix[1] * y + matrix[2] * z + matrix[3],
                    matrix[4] * x + matrix[5] * y + matrix[6] * z + matrix[7],
                    matrix[8] * x + matrix[9] * y + matrix[10] * z + matrix[11],
                ];
                for axis in 0..3 {
                    transformed[0][axis] = transformed[0][axis].min(point[axis]);
                    transformed[1][axis] = transformed[1][axis].max(point[axis]);
                }
            }
        }
    }
    if valid_bounds(transformed) {
        Ok(transformed)
    } else {
        Err(ExactBRepGraphError::ResourceLimit)
    }
}

pub(super) fn swept_profile_bounds(
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let [[min_x, min_y], [max_x, max_y]] = planar_geometry_bounds(&profile.geometry)?;
    let [origin, x_axis, y_axis, _] = profile.frame().to_vectors();
    let direction = interval.direction();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [min_x, max_x] {
        for y in [min_y, max_y] {
            for distance in [interval.start_mm(), interval.end_mm()] {
                let point = [0, 1, 2].map(|axis| {
                    origin[axis] + x_axis[axis] * x + y_axis[axis] * y + direction[axis] * distance
                });
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
    }
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub fn exact_brep_planar_rectangle_bounds(profile: &ExactBRepProfile) -> Option<[f64; 4]> {
    if profile.frame_bits != identity_frame() {
        return None;
    }
    let ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    } = &profile.geometry
    else {
        return None;
    };
    if segments.len() != 4 {
        return None;
    }
    let mut points = Vec::with_capacity(4);
    for segment in segments {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        let start = start_bits.map(f64::from_bits);
        let end = end_bits.map(f64::from_bits);
        let delta = [end[0] - start[0], end[1] - start[1]];
        if start.into_iter().chain(end).any(|value| !value.is_finite())
            || (delta[0] != 0.0 && delta[1] != 0.0)
        {
            return None;
        }
        points.push(start);
    }
    let min_x = points.iter().map(|point| point[0]).reduce(f64::min)?;
    let min_y = points.iter().map(|point| point[1]).reduce(f64::min)?;
    let max_x = points.iter().map(|point| point[0]).reduce(f64::max)?;
    let max_y = points.iter().map(|point| point[1]).reduce(f64::max)?;
    let corners = [
        [min_x, min_y],
        [min_x, max_y],
        [max_x, min_y],
        [max_x, max_y],
    ];
    (min_x < max_x
        && min_y < max_y
        && corners
            .iter()
            .all(|corner| points.iter().any(|point| point == corner)))
    .then_some([min_x, min_y, max_x, max_y])
}

pub(super) fn planar_offset_profile_bounds(
    profile: &ExactBRepProfile,
    distance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let mut local_profile = profile.clone();
    local_profile.frame_bits = identity_frame();
    let local = local_planar_offset_profile_bounds(&local_profile, distance_mm)?;
    let frame = profile.frame();
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for x in [local[0][0], local[1][0]] {
        for y in [local[0][1], local[1][1]] {
            let point = frame.point(Vec3::new(x, y, 0.0)).to_array();
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(point[axis]);
                bounds[1][axis] = bounds[1][axis].max(point[axis]);
            }
        }
    }
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn local_planar_offset_profile_bounds(
    profile: &ExactBRepProfile,
    distance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    if profile.frame_bits != identity_frame()
        || !distance_mm.is_finite()
        || !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&distance_mm.abs())
    {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let bounds = match &profile.geometry {
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let output_radius = radius + distance_mm;
            if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&radius)
                || !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&output_radius)
                || [
                    center[0] - radius,
                    center[1] - radius,
                    center[0] + radius,
                    center[1] + radius,
                ]
                .into_iter()
                .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
            {
                return Err(ExactBRepGraphError::InvalidParameter);
            }
            [
                [center[0] - output_radius, center[1] - output_radius, 0.0],
                [center[0] + output_radius, center[1] + output_radius, 0.0],
            ]
        }
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => {
            if let Some([min_x, min_y, max_x, max_y]) = exact_brep_planar_rectangle_bounds(profile)
            {
                let bounds = [
                    [min_x - distance_mm, min_y - distance_mm, 0.0],
                    [max_x + distance_mm, max_y + distance_mm, 0.0],
                ];
                if bounds[1][0] - bounds[0][0] < limits::MIN_LENGTH_MM
                    || bounds[1][1] - bounds[0][1] < limits::MIN_LENGTH_MM
                {
                    return Err(ExactBRepGraphError::InvalidParameter);
                }
                bounds
            } else {
                let exact = exact_planar_offset_profile_from_segments(segments.clone())
                    .ok_or(ExactBRepGraphError::InvalidParameter)?;
                let [min_x, min_y, max_x, max_y] = exact.bounds_bits.map(f64::from_bits);
                if distance_mm < 0.0
                    && (max_x - min_x <= 2.0 * distance_mm.abs()
                        || max_y - min_y <= 2.0 * distance_mm.abs())
                {
                    return Err(ExactBRepGraphError::InvalidParameter);
                }
                let margin = exact
                    .max_planar_offset_displacement_mm(distance_mm)
                    .ok_or(ExactBRepGraphError::InvalidParameter)?;
                if distance_mm > 0.0 {
                    [
                        [min_x - margin, min_y - margin, 0.0],
                        [max_x + margin, max_y + margin, 0.0],
                    ]
                } else {
                    [[min_x, min_y, 0.0], [max_x, max_y, 0.0]]
                }
            }
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            if !(ExactPlanarOffsetRegion {
                outer: outer.clone(),
                holes: holes.clone(),
            })
            .has_valid_encoding(distance_mm)
            {
                return Err(ExactBRepGraphError::InvalidParameter);
            }
            let mut loop_profile = profile.clone();
            loop_profile.geometry = loop_geometry(outer);
            let bounds = local_planar_offset_profile_bounds(&loop_profile, distance_mm)?;
            for hole in holes {
                loop_profile.geometry = loop_geometry(hole);
                local_planar_offset_profile_bounds(&loop_profile, -distance_mm)?;
            }
            bounds
        }
        _ => return Err(ExactBRepGraphError::InvalidParameter),
    };
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn sweep_path_segment_metrics(
    segment: &ExactBRepPlanarSegment,
    tolerance_mm: f64,
) -> Option<(f64, [f64; 2], [f64; 2])> {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let direction = [end[0] - start[0], end[1] - start[1]];
            let length = direction[0].hypot(direction[1]);
            if !length.is_finite() || length <= tolerance_mm {
                return None;
            }
            let tangent = [direction[0] / length, direction[1] / length];
            Some((length, tangent, tangent))
        }
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let center = center_bits.map(f64::from_bits);
            let start_radius = [start[0] - center[0], start[1] - center[1]];
            let end_radius = [end[0] - center[0], end[1] - center[1]];
            let radius = start_radius[0].hypot(start_radius[1]);
            let end_radius_length = end_radius[0].hypot(end_radius[1]);
            let start_angle = start_radius[1].atan2(start_radius[0]);
            let end_angle = end_radius[1].atan2(end_radius[0]);
            let sweep_angle = if *clockwise {
                (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
            };
            if !radius.is_finite()
                || radius <= tolerance_mm
                || (radius - end_radius_length).abs() > SWEEP_PATH_INTERSECTION_EPSILON_MM
                || start == end
                || !sweep_angle.is_finite()
                || radius * sweep_angle <= tolerance_mm
            {
                return None;
            }
            let tangent = |radial: [f64; 2], radial_length: f64| {
                if *clockwise {
                    [radial[1] / radial_length, -radial[0] / radial_length]
                } else {
                    [-radial[1] / radial_length, radial[0] / radial_length]
                }
            };
            Some((
                radius * sweep_angle,
                tangent(start_radius, radius),
                tangent(end_radius, end_radius_length),
            ))
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let start = start_bits.map(f64::from_bits);
            let control_1 = control_1_bits.map(f64::from_bits);
            let control_2 = control_2_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let forward =
                ketchup_geometry::linalg::CubicBezier::new([start, control_1, control_2, end])
                    .forward(tolerance_mm)?;
            Some((
                forward.control_length,
                forward.start_tangent,
                forward.end_tangent,
            ))
        }
    }
}

pub(super) fn sweep_path_planar_bounds(
    segments: &[ExactBRepPlanarSegment],
) -> Option<[[f64; 2]; 2]> {
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let mut include = |point: [f64; 2]| {
        for axis in 0..2 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    };
    for segment in segments {
        match segment {
            ExactBRepPlanarSegment::Line {
                start_bits,
                end_bits,
            } => {
                include(start_bits.map(f64::from_bits));
                include(end_bits.map(f64::from_bits));
            }
            ExactBRepPlanarSegment::CircularArc {
                center_bits,
                start_bits,
                end_bits,
                ..
            } => {
                let center = center_bits.map(f64::from_bits);
                let start = start_bits.map(f64::from_bits);
                let end = end_bits.map(f64::from_bits);
                let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
                let radius = start_radius.max(end_radius);
                include([center[0] - radius, center[1] - radius]);
                include([center[0] + radius, center[1] + radius]);
            }
            ExactBRepPlanarSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => {
                include(start_bits.map(f64::from_bits));
                include(control_1_bits.map(f64::from_bits));
                include(control_2_bits.map(f64::from_bits));
                include(end_bits.map(f64::from_bits));
            }
        }
    }
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        .then_some(bounds)
}

pub(super) const SWEEP_PATH_INTERSECTION_EPSILON_MM: f64 = ROUNDING;

pub(super) fn sweep_path_segment_endpoints(
    segment: &ExactBRepPlanarSegment,
) -> ([f64; 2], [f64; 2]) {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        }
        | ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            ..
        }
        | ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            end_bits,
            ..
        } => (start_bits.map(f64::from_bits), end_bits.map(f64::from_bits)),
    }
}

pub(super) fn sweep_path_segment_bounds(segment: &ExactBRepPlanarSegment) -> [[f64; 2]; 2] {
    let points = match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => vec![start_bits.map(f64::from_bits), end_bits.map(f64::from_bits)],
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            ..
        } => {
            let start = start_bits.map(f64::from_bits);
            let end = end_bits.map(f64::from_bits);
            let center = center_bits.map(f64::from_bits);
            let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
            let radius = start_radius.max(end_radius);
            return [
                [center[0] - radius, center[1] - radius],
                [center[0] + radius, center[1] + radius],
            ];
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => vec![
            start_bits.map(f64::from_bits),
            control_1_bits.map(f64::from_bits),
            control_2_bits.map(f64::from_bits),
            end_bits.map(f64::from_bits),
        ],
    };
    [0, 1].map(|bound| {
        [0, 1].map(|axis| {
            points.iter().fold(
                if bound == 0 {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
                |value, point| {
                    if bound == 0 {
                        value.min(point[axis])
                    } else {
                        value.max(point[axis])
                    }
                },
            )
        })
    })
}

pub(super) fn sweep_path_arc_angle(segment: &ExactBRepPlanarSegment) -> Option<f64> {
    let ExactBRepPlanarSegment::CircularArc {
        start_bits,
        end_bits,
        center_bits,
        clockwise,
    } = segment
    else {
        return None;
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let center = center_bits.map(f64::from_bits);
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    Some(if *clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    })
}

pub(super) fn sweep_path_join_is_separated(
    left: &ExactBRepPlanarSegment,
    right: &ExactBRepPlanarSegment,
    tangent: [f64; 2],
) -> bool {
    let join = sweep_path_segment_endpoints(left).1;
    let projection =
        |point: [f64; 2]| (point[0] - join[0]) * tangent[0] + (point[1] - join[1]) * tangent[1];
    let left_is_behind = match left {
        ExactBRepPlanarSegment::Line { start_bits, .. } => {
            projection(start_bits.map(f64::from_bits)) < -SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        ExactBRepPlanarSegment::CircularArc { .. } => sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            ..
        } => [*start_bits, *control_1_bits, *control_2_bits]
            .map(|point| point.map(f64::from_bits))
            .into_iter()
            .all(|point| projection(point) < -SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    let right_is_ahead = match right {
        ExactBRepPlanarSegment::Line { end_bits, .. } => {
            projection(end_bits.map(f64::from_bits)) > SWEEP_PATH_INTERSECTION_EPSILON_MM
        }
        ExactBRepPlanarSegment::CircularArc { .. } => sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - SWEEP_PATH_INTERSECTION_EPSILON_MM),
        ExactBRepPlanarSegment::CubicBezier {
            control_1_bits,
            control_2_bits,
            end_bits,
            ..
        } => [*control_1_bits, *control_2_bits, *end_bits]
            .map(|point| point.map(f64::from_bits))
            .into_iter()
            .all(|point| projection(point) > SWEEP_PATH_INTERSECTION_EPSILON_MM),
    };
    left_is_behind && right_is_ahead
}

pub(super) fn sweep_path_self_intersects(
    segments: &[ExactBRepPlanarSegment],
    metrics: &[(f64, [f64; 2], [f64; 2])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let bounds = segments
        .iter()
        .map(sweep_path_segment_bounds)
        .collect::<Vec<_>>();
    for left in 0..bounds.len() {
        for right in left + 2..bounds.len() {
            if [0, 1].into_iter().all(|axis| {
                bounds[left][0][axis] <= bounds[right][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
                    && bounds[right][0][axis]
                        <= bounds[left][1][axis] + SWEEP_PATH_INTERSECTION_EPSILON_MM
            }) {
                return true;
            }
        }
    }
    false
}

pub(super) fn sweep_path_length(
    segments: &[ExactBRepPlanarSegment],
    tolerance_mm: f64,
) -> Option<f64> {
    if !(1..=limits::PATH_SEGMENTS).contains(&segments.len())
        || segments.len() == 1 && !matches!(segments[0], ExactBRepPlanarSegment::Line { .. })
    {
        return None;
    }
    let metrics = segments
        .iter()
        .map(|segment| sweep_path_segment_metrics(segment, tolerance_mm))
        .collect::<Option<Vec<_>>>()?;
    let total_length = metrics.iter().map(|metrics| metrics.0).sum::<f64>();
    if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&total_length)
        || metrics.windows(2).any(|pair| {
            let outgoing = pair[0].2;
            let incoming = pair[1].1;
            let dot = ketchup_geometry::linalg::dot2(outgoing, incoming);
            let cross = outgoing[0] * incoming[1] - outgoing[1] * incoming[0];
            dot < 1.0 - ROUNDING || cross.abs() > ROUNDING
        })
        || sweep_path_self_intersects(segments, &metrics)
    {
        return None;
    }
    Some(total_length)
}

pub(super) fn sweep_profile_bounds(
    profile: &ExactBRepProfile,
    path: &ExactBRepProfile,
    tolerance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    let [[min_u, min_v], [max_u, max_v]] = planar_geometry_bounds(&profile.geometry)?;
    let ExactBRepPlanarGeometry::Boundary {
        closed: false,
        segments,
    } = &path.geometry
    else {
        return Err(ExactBRepGraphError::InvalidParameter);
    };
    let path_length =
        sweep_path_length(segments, tolerance_mm).ok_or(ExactBRepGraphError::InvalidParameter)?;
    if segments.len() > 1 {
        if profile.frame_bits != identity_frame()
            || path.frame_bits != identity_frame()
            || !matches!(
                profile.geometry,
                ExactBRepPlanarGeometry::Boundary { closed: true, .. }
            )
        {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        let [[min_x, min_y], [max_x, max_y]] =
            sweep_path_planar_bounds(segments).ok_or(ExactBRepGraphError::InvalidParameter)?;
        let radial_extent = min_u.abs().max(max_u.abs());
        let bounds = [
            [min_x - radial_extent, min_y - radial_extent, min_v],
            [max_x + radial_extent, max_y + radial_extent, max_v],
        ];
        return valid_bounds(bounds)
            .then_some(bounds)
            .ok_or(ExactBRepGraphError::InvalidParameter);
    }
    let [
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        },
    ] = segments.as_slice()
    else {
        return Err(ExactBRepGraphError::InvalidParameter);
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let direction = [end[0] - start[0], end[1] - start[1]];
    let tangent = [direction[0] / path_length, direction[1] / path_length];
    let section = [tangent[1], -tangent[0]];
    let frame = profile.frame();
    let mut framed_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for u in [min_u, max_u] {
        for v in [min_v, max_v] {
            for along in [0.0, path_length] {
                let profile_point = frame.point(Vec3::new(u, v, along)).to_array();
                let point = [
                    start[0] + section[0] * profile_point[0] + tangent[0] * profile_point[2],
                    start[1] + section[1] * profile_point[0] + tangent[1] * profile_point[2],
                    profile_point[1],
                ];
                for axis in 0..3 {
                    framed_bounds[0][axis] = framed_bounds[0][axis].min(profile_point[axis]);
                    framed_bounds[1][axis] = framed_bounds[1][axis].max(profile_point[axis]);
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
            }
        }
    }
    (valid_bounds(framed_bounds) && valid_bounds(bounds))
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn spatial_sweep_bounds(
    profile: &ExactBRepProfile,
    path: &ExactBRepSpatialPath,
    tolerance_mm: f64,
) -> Result<[[f64; 3]; 2], ExactBRepGraphError> {
    if profile.frame_bits != identity_frame() || !valid_spatial_path(path, tolerance_mm) {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let [[min_u, min_v], [max_u, max_v]] = planar_geometry_bounds(&profile.geometry)?;
    let section_radius = min_u
        .abs()
        .max(max_u.abs())
        .hypot(min_v.abs().max(max_v.abs()));
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    let mut include = |point: [f64; 3]| {
        for axis in 0..3 {
            bounds[0][axis] = bounds[0][axis].min(point[axis] - section_radius);
            bounds[1][axis] = bounds[1][axis].max(point[axis] + section_radius);
        }
    };
    for segment in &path.segments {
        match segment {
            ExactBRepSpatialPathSegment::Line {
                start_bits,
                end_bits,
            } => {
                include(start_bits.map(f64::from_bits));
                include(end_bits.map(f64::from_bits));
            }
            ExactBRepSpatialPathSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                ..
            } => {
                let start = start_bits.map(f64::from_bits);
                let center = center_bits.map(f64::from_bits);
                let radius = sub(start, center);
                let radius = dot(radius, radius).sqrt();
                include(center.map(|coordinate| coordinate - radius));
                include(center.map(|coordinate| coordinate + radius));
                include(end_bits.map(f64::from_bits));
            }
            ExactBRepSpatialPathSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => {
                for point in [start_bits, control_1_bits, control_2_bits, end_bits] {
                    include(point.map(f64::from_bits));
                }
            }
        }
    }
    valid_bounds(bounds)
        .then_some(bounds)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn sweep_profile_geometry(
    profile: &FeatureKind,
    tolerance_mm: f64,
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    match profile {
        FeatureKind::Profile {
            segments,
            closed: true,
        } => boundary_geometry(segments, true, tolerance_mm),
        _ => Err(ExactBRepGraphError::InvalidParameter),
    }
}

pub(crate) fn sweep_profile_is_valid(profile: &FeatureKind, tolerance_mm: f64) -> bool {
    sweep_profile_geometry(profile, tolerance_mm)
        .and_then(|geometry| planar_geometry_bounds(&geometry))
        .is_ok()
}

pub(crate) fn spatial_sweep_bounds_are_valid(
    profile: &FeatureKind,
    segments: &[SpatialPathSegment],
    tolerance_mm: f64,
) -> bool {
    let (Ok(geometry), Ok(path)) = (
        sweep_profile_geometry(profile, tolerance_mm),
        spatial_path(FeatureId(1), segments, tolerance_mm),
    ) else {
        return false;
    };
    spatial_sweep_bounds(
        &ExactBRepProfile {
            id: ExactBRepProfileId(0),
            source_feature_id: 1,
            region_id: None,
            frame_bits: identity_frame(),
            segment_entity_ids: Vec::new(),
            geometry,
        },
        &path,
        tolerance_mm,
    )
    .is_ok()
}

/// Whether `point` lies inside `geometry` (even-odd rule over its loops).
pub(super) fn planar_geometry_contains(
    geometry: &ExactBRepPlanarGeometry,
    point: [f64; 2],
) -> bool {
    let in_circle = |center_bits: &[u64; 2], radius_bits: &u64| {
        let center = center_bits.map(f64::from_bits);
        (point[0] - center[0]).hypot(point[1] - center[1]) < f64::from_bits(*radius_bits)
    };
    let in_boundary = |segments: &[ExactBRepPlanarSegment]| {
        let outline = segments
            .iter()
            .flat_map(|segment| planar_segment_points(segment, 32))
            .collect::<Vec<_>>();
        let mut inside = false;
        for (index, start) in outline.iter().enumerate() {
            let end = outline[(index + 1) % outline.len()];
            if (start[1] > point[1]) != (end[1] > point[1])
                && point[0]
                    < start[0] + (point[1] - start[1]) * (end[0] - start[0]) / (end[1] - start[1])
            {
                inside = !inside;
            }
        }
        inside
    };
    let in_loop = |planar_loop: &ExactBRepPlanarLoop| match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => in_boundary(segments),
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => in_circle(center_bits, radius_bits),
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => in_boundary(segments),
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => in_circle(center_bits, radius_bits),
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            in_loop(outer) && !holes.iter().any(in_loop)
        }
        ExactBRepPlanarGeometry::Boundary { closed: false, .. }
        | ExactBRepPlanarGeometry::Spline { .. } => false,
    }
}

/// Points along a segment from its start up to (not including) its end.
pub(super) fn planar_segment_points(segment: &ExactBRepPlanarSegment, steps: u32) -> Vec<[f64; 2]> {
    match segment {
        ExactBRepPlanarSegment::Line { start_bits, .. } => vec![start_bits.map(f64::from_bits)],
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let [start, end, center] =
                [start_bits, end_bits, center_bits].map(|bits| bits.map(f64::from_bits));
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let from = (start[1] - center[1]).atan2(start[0] - center[0]);
            let to = (end[1] - center[1]).atan2(end[0] - center[0]);
            let sweep = if *clockwise {
                -(from - to).rem_euclid(std::f64::consts::TAU)
            } else {
                (to - from).rem_euclid(std::f64::consts::TAU)
            };
            (0..steps)
                .map(|step| {
                    let angle = from + sweep * f64::from(step) / f64::from(steps);
                    [
                        center[0] + radius * angle.cos(),
                        center[1] + radius * angle.sin(),
                    ]
                })
                .collect()
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let [p0, p1, p2, p3] = [start_bits, control_1_bits, control_2_bits, end_bits]
                .map(|bits| bits.map(f64::from_bits));
            let curve = CubicBezier::new([p0, p1, p2, p3]);
            (0..steps)
                .map(|step| curve.eval(f64::from(step) / f64::from(steps)))
                .collect()
        }
    }
}

/// Distance in the profile plane from `point` to the boundary of `geometry`.
pub(super) fn planar_geometry_distance(
    geometry: &ExactBRepPlanarGeometry,
    point: [f64; 2],
) -> Option<f64> {
    let circle = |center_bits: &[u64; 2], radius_bits: &u64| {
        let center = center_bits.map(f64::from_bits);
        ((point[0] - center[0]).hypot(point[1] - center[1]) - f64::from_bits(*radius_bits)).abs()
    };
    let segments = |segments: &[ExactBRepPlanarSegment]| {
        segments
            .iter()
            .map(|segment| planar_segment_distance(segment, point))
            .reduce(f64::min)
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary { segments: list, .. } => segments(list),
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => Some(circle(center_bits, radius_bits)),
        ExactBRepPlanarGeometry::Region { outer, holes } => std::iter::once(outer)
            .chain(holes)
            .filter_map(|planar_loop| match planar_loop {
                ExactBRepPlanarLoop::Boundary { segments: list } => segments(list),
                ExactBRepPlanarLoop::Circle {
                    center_bits,
                    radius_bits,
                } => Some(circle(center_bits, radius_bits)),
            })
            .reduce(f64::min),
        ExactBRepPlanarGeometry::Spline { .. } => None,
    }
}

pub(super) fn planar_segment_distance(segment: &ExactBRepPlanarSegment, point: [f64; 2]) -> f64 {
    let to_line = |start: [f64; 2], end: [f64; 2]| {
        let edge = [end[0] - start[0], end[1] - start[1]];
        let length_squared = ketchup_geometry::linalg::dot2(edge, edge);
        let t = if length_squared > 0.0 {
            (((point[0] - start[0]) * edge[0] + (point[1] - start[1]) * edge[1]) / length_squared)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        (point[0] - start[0] - edge[0] * t).hypot(point[1] - start[1] - edge[1] * t)
    };
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => to_line(start_bits.map(f64::from_bits), end_bits.map(f64::from_bits)),
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            let [start, end, center] =
                [start_bits, end_bits, center_bits].map(|bits| bits.map(f64::from_bits));
            let angle = |at: [f64; 2]| (at[1] - center[1]).atan2(at[0] - center[0]);
            let sweep = |from: f64, to: f64| {
                let turn = if *clockwise { from - to } else { to - from };
                turn.rem_euclid(std::f64::consts::TAU)
            };
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let on_arc = sweep(angle(start), angle(point)) <= sweep(angle(start), angle(end));
            let to_circle = ((point[0] - center[0]).hypot(point[1] - center[1]) - radius).abs();
            let to_ends = (point[0] - start[0])
                .hypot(point[1] - start[1])
                .min((point[0] - end[0]).hypot(point[1] - end[1]));
            if on_arc { to_circle } else { to_ends }
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            let [p0, p1, p2, p3] = [start_bits, control_1_bits, control_2_bits, end_bits]
                .map(|bits| bits.map(f64::from_bits));
            let curve = CubicBezier::new([p0, p1, p2, p3]);
            let at = |t: f64| curve.eval(t);
            (0..64)
                .map(|step| to_line(at(f64::from(step) / 64.0), at(f64::from(step + 1) / 64.0)))
                .fold(f64::INFINITY, f64::min)
        }
    }
}

pub(super) fn planar_geometry_bounds(
    geometry: &ExactBRepPlanarGeometry,
) -> Result<[[f64; 2]; 2], ExactBRepGraphError> {
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    let mut include = |point: [f64; 2]| {
        for axis in 0..2 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary { segments, .. } => {
            for segment in segments {
                match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => {
                        include(start_bits.map(f64::from_bits));
                        include(end_bits.map(f64::from_bits));
                    }
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        ..
                    } => {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        let center = center_bits.map(f64::from_bits);
                        let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                        include(start);
                        include(end);
                        include([center[0] - radius, center[1] - radius]);
                        include([center[0] + radius, center[1] + radius]);
                    }
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => {
                        include(start_bits.map(f64::from_bits));
                        include(control_1_bits.map(f64::from_bits));
                        include(control_2_bits.map(f64::from_bits));
                        include(end_bits.map(f64::from_bits));
                    }
                }
            }
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            include([center[0] - radius, center[1] - radius]);
            include([center[0] + radius, center[1] + radius]);
        }
        ExactBRepPlanarGeometry::Spline { control_point_bits } => {
            for point in control_point_bits {
                include(point.map(f64::from_bits));
            }
        }
        ExactBRepPlanarGeometry::Region { outer, .. } => {
            let loop_bounds = planar_geometry_bounds(&loop_geometry(outer))?;
            include(loop_bounds[0]);
            include(loop_bounds[1]);
        }
    }
    if bounds.iter().flatten().all(|value| value.is_finite())
        && (0..2).all(|axis| bounds[0][axis] < bounds[1][axis])
    {
        Ok(bounds)
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

pub(super) fn bounds_union(left: [[f64; 3]; 2], right: [[f64; 3]; 2]) -> [[f64; 3]; 2] {
    [
        [0, 1, 2].map(|axis| left[0][axis].min(right[0][axis])),
        [0, 1, 2].map(|axis| left[1][axis].max(right[1][axis])),
    ]
}

pub(super) fn bounds_intersection(
    left: [[f64; 3]; 2],
    right: [[f64; 3]; 2],
) -> Option<[[f64; 3]; 2]> {
    let bounds = [
        [0, 1, 2].map(|axis| left[0][axis].max(right[0][axis])),
        [0, 1, 2].map(|axis| left[1][axis].min(right[1][axis])),
    ];
    valid_bounds(bounds).then_some(bounds)
}

pub(super) fn valid_bounds(bounds: [[f64; 3]; 2]) -> bool {
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        && (0..3).all(|axis| bounds[0][axis] < bounds[1][axis])
}

pub(super) fn valid_surface_bounds(bounds: [[f64; 3]; 2]) -> bool {
    bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM)
        && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis])
        && (0..3)
            .filter(|axis| bounds[0][*axis] < bounds[1][*axis])
            .count()
            >= 2
}
