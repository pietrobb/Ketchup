use super::*;
use ketchup_geometry::linalg::{cross, dot};
use ketchup_tolerance::limits;

pub(super) fn planar_segment_endpoints(segment: &PlanarProfileSegment) -> ([f64; 2], [f64; 2]) {
    match segment {
        PlanarProfileSegment::Line { start_mm, end_mm }
        | PlanarProfileSegment::CircularArc {
            start_mm, end_mm, ..
        }
        | PlanarProfileSegment::CubicBezier {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    }
}

pub(super) fn sweep_path_segment_bounds(segment: &PlanarProfileSegment) -> [[f64; 2]; 2] {
    let points = match segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            ..
        } => {
            let start_radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
            let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
            let radius = start_radius.max(end_radius);
            return [
                [center_mm[0] - radius, center_mm[1] - radius],
                [center_mm[0] + radius, center_mm[1] + radius],
            ];
        }
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => vec![*start_mm, *control_1_mm, *control_2_mm, *end_mm],
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

pub(super) fn sweep_path_arc_angle(segment: &PlanarProfileSegment) -> Option<f64> {
    let PlanarProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        clockwise,
    } = segment
    else {
        return None;
    };
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    Some(if *clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    })
}

pub(super) fn sweep_path_join_is_separated(
    left: &PlanarProfileSegment,
    right: &PlanarProfileSegment,
    tangent: [f64; 2],
) -> bool {
    let join = planar_segment_endpoints(left).1;
    let projection =
        |point: [f64; 2]| (point[0] - join[0]) * tangent[0] + (point[1] - join[1]) * tangent[1];
    let left_is_behind = match left {
        PlanarProfileSegment::Line { start_mm, .. } => projection(*start_mm) < -ROUNDING,
        PlanarProfileSegment::CircularArc { .. } => {
            sweep_path_arc_angle(left).is_some_and(|angle| angle < std::f64::consts::PI - ROUNDING)
        }
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            ..
        } => [*start_mm, *control_1_mm, *control_2_mm]
            .into_iter()
            .all(|point| projection(point) < -ROUNDING),
    };
    let right_is_ahead = match right {
        PlanarProfileSegment::Line { end_mm, .. } => projection(*end_mm) > ROUNDING,
        PlanarProfileSegment::CircularArc { .. } => {
            sweep_path_arc_angle(right).is_some_and(|angle| angle < std::f64::consts::PI - ROUNDING)
        }
        PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => [*control_1_mm, *control_2_mm, *end_mm]
            .into_iter()
            .all(|point| projection(point) > ROUNDING),
    };
    left_is_behind && right_is_ahead
}

pub(super) fn sweep_path_self_intersects(
    segments: &[PlanarProfileSegment],
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
                bounds[left][0][axis] <= bounds[right][1][axis] + ROUNDING
                    && bounds[right][0][axis] <= bounds[left][1][axis] + ROUNDING
            }) {
                return true;
            }
        }
    }
    false
}

// Request-digest encoding of segments: stored exact provenance records the digest
// of these values, so they stay frozen and never cross the native boundary.
pub(super) fn planar_segment_digest_values(segment: &PlanarProfileSegment) -> [f64; 10] {
    match segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => [
            0.0,
            start_mm[0],
            start_mm[1],
            end_mm[0],
            end_mm[1],
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => [
            1.0,
            start_mm[0],
            start_mm[1],
            end_mm[0],
            end_mm[1],
            center_mm[0],
            center_mm[1],
            0.0,
            0.0,
            f64::from(*clockwise),
        ],
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => [
            2.0,
            start_mm[0],
            start_mm[1],
            end_mm[0],
            end_mm[1],
            control_1_mm[0],
            control_1_mm[1],
            control_2_mm[0],
            control_2_mm[1],
            0.0,
        ],
    }
}

pub(super) fn circle_digest_values(
    center_mm: [f64; 2],
    radius_mm: f64,
    reversed: bool,
) -> [f64; 10] {
    [
        3.0,
        center_mm[0],
        center_mm[1],
        radius_mm,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        f64::from(reversed),
    ]
}

pub(super) fn planar_segments_digest_values(segments: &[PlanarProfileSegment]) -> Vec<f64> {
    segments
        .iter()
        .flat_map(planar_segment_digest_values)
        .collect()
}

pub(super) fn planar_loop_digest_values(profile: &PlanarProfileLoop) -> Vec<f64> {
    match profile {
        PlanarProfileLoop::Segments(segments) => planar_segments_digest_values(segments),
        PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } => circle_digest_values(*center_mm, *radius_mm, false).to_vec(),
    }
}

pub(super) fn digest_bits(values: &[f64]) -> Vec<u64> {
    values.iter().map(|value| value.to_bits()).collect()
}

fn native_point(point: [f64; 2]) -> ffi::NativePoint {
    ffi::NativePoint {
        x: point[0],
        y: point[1],
        z: 0.0,
    }
}

pub(super) fn native_spatial_point(point: [f64; 3]) -> ffi::NativePoint {
    ffi::NativePoint {
        x: point[0],
        y: point[1],
        z: point[2],
    }
}

fn native_segment(kind: ffi::NativeSegmentKind) -> ffi::NativeSegment {
    let zero = native_spatial_point([0.0; 3]);
    ffi::NativeSegment {
        kind,
        start: zero,
        end: zero,
        center: zero,
        normal: zero,
        control_1: zero,
        control_2: zero,
        radius: 0.0,
        clockwise: false,
    }
}

pub(super) fn native_planar_segment(segment: &PlanarProfileSegment) -> ffi::NativeSegment {
    match *segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => ffi::NativeSegment {
            start: native_point(start_mm),
            end: native_point(end_mm),
            ..native_segment(ffi::NativeSegmentKind::Line)
        },
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => ffi::NativeSegment {
            start: native_point(start_mm),
            end: native_point(end_mm),
            center: native_point(center_mm),
            normal: native_spatial_point([0.0, 0.0, 1.0]),
            clockwise,
            ..native_segment(ffi::NativeSegmentKind::CircularArc)
        },
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ffi::NativeSegment {
            start: native_point(start_mm),
            end: native_point(end_mm),
            control_1: native_point(control_1_mm),
            control_2: native_point(control_2_mm),
            ..native_segment(ffi::NativeSegmentKind::CubicBezier)
        },
    }
}

pub(super) fn native_circle(
    center_mm: [f64; 2],
    radius_mm: f64,
    clockwise: bool,
) -> ffi::NativeSegment {
    ffi::NativeSegment {
        center: native_point(center_mm),
        normal: native_spatial_point([0.0, 0.0, 1.0]),
        radius: radius_mm,
        clockwise,
        ..native_segment(ffi::NativeSegmentKind::Circle)
    }
}

pub(super) fn native_planar_segments(segments: &[PlanarProfileSegment]) -> Vec<ffi::NativeSegment> {
    segments.iter().map(native_planar_segment).collect()
}

pub(super) fn native_planar_loop(profile: &PlanarProfileLoop) -> Vec<ffi::NativeSegment> {
    match profile {
        PlanarProfileLoop::Segments(segments) => native_planar_segments(segments),
        PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } => vec![native_circle(*center_mm, *radius_mm, false)],
    }
}

pub(super) fn native_spatial_segment(segment: &SpatialProfileSegment) -> ffi::NativeSegment {
    match *segment {
        SpatialProfileSegment::Line { start_mm, end_mm } => ffi::NativeSegment {
            start: native_spatial_point(start_mm),
            end: native_spatial_point(end_mm),
            ..native_segment(ffi::NativeSegmentKind::Line)
        },
        SpatialProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            normal,
            clockwise,
        } => ffi::NativeSegment {
            start: native_spatial_point(start_mm),
            end: native_spatial_point(end_mm),
            center: native_spatial_point(center_mm),
            normal: native_spatial_point(normal),
            clockwise,
            ..native_segment(ffi::NativeSegmentKind::CircularArc)
        },
        SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ffi::NativeSegment {
            start: native_spatial_point(start_mm),
            end: native_spatial_point(end_mm),
            control_1: native_spatial_point(control_1_mm),
            control_2: native_spatial_point(control_2_mm),
            ..native_segment(ffi::NativeSegmentKind::CubicBezier)
        },
    }
}

pub(super) fn native_spatial_segments(
    segments: &[SpatialProfileSegment],
) -> Vec<ffi::NativeSegment> {
    segments.iter().map(native_spatial_segment).collect()
}

pub(super) fn spatial_segment_endpoints(segment: &SpatialProfileSegment) -> ([f64; 3], [f64; 3]) {
    match segment {
        SpatialProfileSegment::Line { start_mm, end_mm }
        | SpatialProfileSegment::CircularArc {
            start_mm, end_mm, ..
        }
        | SpatialProfileSegment::CubicBezier {
            start_mm, end_mm, ..
        } => (*start_mm, *end_mm),
    }
}

pub(super) fn spatial_sub(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

pub(super) fn spatial_length(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}

pub(super) fn spatial_unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = spatial_length(vector);
    (length.is_finite() && length > MIN_SWEEP_PATH_SEGMENT_LENGTH_MM)
        .then(|| vector.map(|coordinate| coordinate / length))
}

pub(super) fn spatial_sweep_path_arc_angle(segment: &SpatialProfileSegment) -> Option<f64> {
    let SpatialProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        normal,
        clockwise,
    } = segment
    else {
        return None;
    };
    let normal = spatial_unit(*normal)?;
    let start_radius = spatial_sub(*start_mm, *center_mm);
    let end_radius = spatial_sub(*end_mm, *center_mm);
    let signed = dot(normal, cross(start_radius, end_radius)).atan2(dot(start_radius, end_radius));
    Some(if *clockwise {
        (-signed).rem_euclid(std::f64::consts::TAU)
    } else {
        signed.rem_euclid(std::f64::consts::TAU)
    })
}

pub(super) fn spatial_sweep_path_join_is_separated(
    left: &SpatialProfileSegment,
    right: &SpatialProfileSegment,
    tangent: [f64; 3],
) -> bool {
    let join = spatial_segment_endpoints(left).1;
    let projection = |point: [f64; 3]| dot(spatial_sub(point, join), tangent);
    let left_is_behind = match left {
        SpatialProfileSegment::Line { start_mm, .. } => projection(*start_mm) < -ROUNDING,
        SpatialProfileSegment::CircularArc { .. } => spatial_sweep_path_arc_angle(left)
            .is_some_and(|angle| angle < std::f64::consts::PI - ROUNDING),
        SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            ..
        } => [*start_mm, *control_1_mm, *control_2_mm]
            .into_iter()
            .all(|point| projection(point) < -ROUNDING),
    };
    let right_is_ahead = match right {
        SpatialProfileSegment::Line { end_mm, .. } => projection(*end_mm) > ROUNDING,
        SpatialProfileSegment::CircularArc { .. } => spatial_sweep_path_arc_angle(right)
            .is_some_and(|angle| angle < std::f64::consts::PI - ROUNDING),
        SpatialProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } => [*control_1_mm, *control_2_mm, *end_mm]
            .into_iter()
            .all(|point| projection(point) > ROUNDING),
    };
    left_is_behind && right_is_ahead
}

pub(super) fn spatial_segment_digest_values(segment: &SpatialProfileSegment) -> [f64; 14] {
    match segment {
        SpatialProfileSegment::Line { start_mm, end_mm } => [
            10.0,
            start_mm[0],
            start_mm[1],
            start_mm[2],
            end_mm[0],
            end_mm[1],
            end_mm[2],
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ],
        SpatialProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            normal,
            clockwise,
        } => [
            11.0,
            start_mm[0],
            start_mm[1],
            start_mm[2],
            end_mm[0],
            end_mm[1],
            end_mm[2],
            center_mm[0],
            center_mm[1],
            center_mm[2],
            normal[0],
            normal[1],
            normal[2],
            f64::from(*clockwise),
        ],
        SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => [
            12.0,
            start_mm[0],
            start_mm[1],
            start_mm[2],
            end_mm[0],
            end_mm[1],
            end_mm[2],
            control_1_mm[0],
            control_1_mm[1],
            control_1_mm[2],
            control_2_mm[0],
            control_2_mm[1],
            control_2_mm[2],
            0.0,
        ],
    }
}

pub(super) fn spatial_segments_digest_values(segments: &[SpatialProfileSegment]) -> Vec<f64> {
    segments
        .iter()
        .flat_map(spatial_segment_digest_values)
        .collect()
}

pub(super) type SpatialSweepPathMetric = (f64, [f64; 3], [f64; 3]);

pub(super) fn spatial_sweep_path_metrics(
    segment: &SpatialProfileSegment,
    operation: &'static str,
    input: &str,
) -> Result<SpatialSweepPathMetric, GeometryError> {
    let invalid = || {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Spatial Sweep path must contain only bounded non-degenerate lines, circular arcs, and cubic Bezier segments"
                .to_owned(),
        )
    };
    let (start, end) = spatial_segment_endpoints(segment);
    for (point_name, point) in [("start", start), ("end", end)] {
        for (axis, coordinate) in point.into_iter().enumerate() {
            validate_coordinate(
                coordinate,
                &format!("path_{point_name}_axis_{axis}"),
                operation,
                input,
            )?;
        }
    }
    match segment {
        SpatialProfileSegment::Line { .. } => {
            let direction = spatial_sub(end, start);
            let tangent = spatial_unit(direction).ok_or_else(invalid)?;
            Ok((spatial_length(direction), tangent, tangent))
        }
        SpatialProfileSegment::CircularArc {
            center_mm,
            normal,
            clockwise,
            ..
        } => {
            for (point_name, point) in [("center", *center_mm), ("normal", *normal)] {
                for (axis, coordinate) in point.into_iter().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("path_{point_name}_axis_{axis}"),
                        operation,
                        input,
                    )?;
                }
            }
            let normal_length = spatial_length(*normal);
            if (normal_length - 1.0).abs() > ROUNDING {
                return Err(invalid());
            }
            let normal = spatial_unit(*normal).ok_or_else(invalid)?;
            let start_radius = spatial_sub(start, *center_mm);
            let end_radius = spatial_sub(end, *center_mm);
            let radius = spatial_length(start_radius);
            let end_radius_length = spatial_length(end_radius);
            if radius <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                || (radius - end_radius_length).abs() > ROUNDING
                || dot(start_radius, normal).abs() > ROUNDING
                || dot(end_radius, normal).abs() > ROUNDING
                || start == end
            {
                return Err(invalid());
            }
            let signed_angle =
                dot(normal, cross(start_radius, end_radius)).atan2(dot(start_radius, end_radius));
            let angle = if *clockwise {
                (-signed_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                signed_angle.rem_euclid(std::f64::consts::TAU)
            };
            let length = radius * angle;
            if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                return Err(invalid());
            }
            let sign = if *clockwise { -1.0 } else { 1.0 };
            let start_tangent =
                spatial_unit(cross(normal, start_radius).map(|v| sign * v)).ok_or_else(invalid)?;
            let end_tangent =
                spatial_unit(cross(normal, end_radius).map(|v| sign * v)).ok_or_else(invalid)?;
            Ok((length, start_tangent, end_tangent))
        }
        SpatialProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } => {
            for (point_name, point) in [("control_1", *control_1_mm), ("control_2", *control_2_mm)]
            {
                for (axis, coordinate) in point.into_iter().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("path_{point_name}_axis_{axis}"),
                        operation,
                        input,
                    )?;
                }
            }
            let chord = spatial_sub(end, start);
            let first = spatial_sub(*control_1_mm, start);
            let middle = spatial_sub(*control_2_mm, *control_1_mm);
            let last = spatial_sub(end, *control_2_mm);
            let chord_squared = dot(chord, chord);
            let projection_1 = dot(first, chord);
            let projection_2 = dot(spatial_sub(*control_2_mm, start), chord);
            if start == end
                || projection_1 <= 0.0
                || projection_2 < projection_1
                || projection_2 >= chord_squared
            {
                return Err(invalid());
            }
            let start_tangent = spatial_unit(first).ok_or_else(invalid)?;
            let end_tangent = spatial_unit(last).ok_or_else(invalid)?;
            Ok((
                spatial_length(first) + spatial_length(middle) + spatial_length(last),
                start_tangent,
                end_tangent,
            ))
        }
    }
}

pub(super) fn spatial_sweep_path_self_intersects(
    segments: &[SpatialProfileSegment],
    metrics: &[(f64, [f64; 3], [f64; 3])],
) -> bool {
    if segments
        .windows(2)
        .zip(metrics.windows(2))
        .any(|(segments, metrics)| {
            !spatial_sweep_path_join_is_separated(&segments[0], &segments[1], metrics[0].2)
        })
    {
        return true;
    }
    let closed = spatial_segment_endpoints(segments.first().unwrap()).0
        == spatial_segment_endpoints(segments.last().unwrap()).1;
    if closed
        && !spatial_sweep_path_join_is_separated(
            segments.last().unwrap(),
            segments.first().unwrap(),
            metrics.last().unwrap().2,
        )
    {
        return true;
    }
    if segments.windows(2).zip(metrics.windows(2)).any(
        |(segments, metrics)| {
            matches!(
                (&segments[0], &segments[1]),
                (
                    SpatialProfileSegment::CircularArc {
                        start_mm,
                        center_mm: left_center,
                        ..
                    },
                    SpatialProfileSegment::CircularArc {
                        center_mm: right_center,
                        ..
                    }
                ) if left_center == right_center
                    && metrics[0].0 + metrics[1].0
                        >= std::f64::consts::TAU * spatial_length(spatial_sub(*start_mm, *left_center))
                            - ROUNDING
            )
        },
    ) {
        return true;
    }
    // Axis-aligned bounds are only a broad phase for spatial curves. The native
    // OCCT operation performs the authoritative edge-distance intersection test.
    false
}

pub(super) fn validate_spatial_sweep_path(
    segments: &[SpatialProfileSegment],
    operation: &'static str,
    input: &str,
) -> Result<Vec<SpatialSweepPathMetric>, GeometryError> {
    let invalid = |diagnostic: &str| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic.to_owned(),
        )
    };
    if !(1..=limits::PATH_SEGMENTS).contains(&segments.len()) {
        return Err(invalid(
            "Spatial Sweep requires between one and 64 path segments",
        ));
    }
    let metrics = segments
        .iter()
        .map(|segment| spatial_sweep_path_metrics(segment, operation, input))
        .collect::<Result<Vec<_>, _>>()?;
    let closed = segments.first().map(spatial_segment_endpoints).unwrap().0
        == segments.last().map(spatial_segment_endpoints).unwrap().1;
    for (segments, metrics) in segments.windows(2).zip(metrics.windows(2)) {
        if spatial_segment_endpoints(&segments[0]).1 != spatial_segment_endpoints(&segments[1]).0 {
            return Err(invalid("Spatial Sweep path segments are disconnected"));
        }
        if dot(metrics[0].2, metrics[1].1) < 1.0 - ROUNDING
            || spatial_length(cross(metrics[0].2, metrics[1].1)) > ROUNDING
        {
            return Err(invalid(
                "Spatial Sweep path segments must be C1 tangent-continuous",
            ));
        }
    }
    if closed {
        let outgoing = metrics.last().unwrap().2;
        let incoming = metrics.first().unwrap().1;
        if dot(outgoing, incoming) < 1.0 - ROUNDING
            || spatial_length(cross(outgoing, incoming)) > ROUNDING
        {
            return Err(invalid(
                "Closed Spatial Sweep path seam must be C1 tangent-continuous",
            ));
        }
    }
    if spatial_sweep_path_self_intersects(segments, &metrics) {
        return Err(invalid("Spatial Sweep path must not self-intersect"));
    }
    validate_length(
        metrics.iter().map(|metric| metric.0).sum(),
        "path_length",
        operation,
        input,
    )?;
    Ok(metrics)
}

pub(super) fn planar_segment_signed_area(segment: &PlanarProfileSegment, origin: [f64; 2]) -> f64 {
    let rebase = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
    match segment {
        PlanarProfileSegment::Line { start_mm, end_mm } => {
            let start = rebase(*start_mm);
            let end = rebase(*end_mm);
            0.5 * (start[0] * end[1] - end[0] * start[1])
        }
        PlanarProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let start = rebase(*start_mm);
            let end = rebase(*end_mm);
            let center = rebase(*center_mm);
            let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
            let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
            let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
            let mut sweep = end_angle - start_angle;
            if *clockwise {
                if sweep >= 0.0 {
                    sweep -= std::f64::consts::TAU;
                }
            } else if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            0.5 * (radius * center[0] * (end_angle.sin() - start_angle.sin())
                - radius * center[1] * (end_angle.cos() - start_angle.cos())
                + radius * radius * sweep)
        }
        PlanarProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => {
            let start = rebase(*start_mm);
            let control_1 = rebase(*control_1_mm);
            let control_2 = rebase(*control_2_mm);
            let end = rebase(*end_mm);
            let coefficients = |axis: usize| {
                [
                    start[axis],
                    3.0 * (control_1[axis] - start[axis]),
                    3.0 * (start[axis] - 2.0 * control_1[axis] + control_2[axis]),
                    -start[axis] + 3.0 * control_1[axis] - 3.0 * control_2[axis] + end[axis],
                ]
            };
            let x = coefficients(0);
            let y = coefficients(1);
            let mut integral = 0.0;
            for left_degree in 0..=3 {
                for right_degree in 1..=3 {
                    let denominator = (left_degree + right_degree) as f64;
                    integral += (x[left_degree] * y[right_degree] * right_degree as f64
                        - y[left_degree] * x[right_degree] * right_degree as f64)
                        / denominator;
                }
            }
            0.5 * integral
        }
    }
}

pub(super) fn reverse_planar_segments(
    segments: &[PlanarProfileSegment],
) -> Vec<PlanarProfileSegment> {
    segments
        .iter()
        .rev()
        .map(|segment| match *segment {
            PlanarProfileSegment::Line { start_mm, end_mm } => PlanarProfileSegment::Line {
                start_mm: end_mm,
                end_mm: start_mm,
            },
            PlanarProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm: end_mm,
                end_mm: start_mm,
                center_mm,
                clockwise: !clockwise,
            },
            PlanarProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => PlanarProfileSegment::CubicBezier {
                start_mm: end_mm,
                control_1_mm: control_2_mm,
                control_2_mm: control_1_mm,
                end_mm: start_mm,
            },
        })
        .collect()
}

/// One region segment in both encodings, so canonical ordering keeps them aligned.
#[derive(Clone, Copy)]
struct RegionSegment {
    digest: [f64; 10],
    native: ffi::NativeSegment,
}

/// A planar region: the outer loop first, then its holes, outer counterclockwise
/// and holes clockwise.
pub(super) struct PlanarRegionPayload {
    loops: Vec<Vec<RegionSegment>>,
}

impl PlanarRegionPayload {
    pub(super) fn digest_values(&self) -> Vec<f64> {
        self.loops
            .iter()
            .flatten()
            .flat_map(|segment| segment.digest)
            .collect()
    }

    pub(super) fn native_segments(&self) -> Vec<ffi::NativeSegment> {
        self.loops
            .iter()
            .flatten()
            .map(|segment| segment.native)
            .collect()
    }

    pub(super) fn loop_segment_counts(&self) -> Vec<u32> {
        // A region holds at most limits::REGION_SEGMENTS segments.
        self.loops
            .iter()
            .map(|segments| segments.len() as u32)
            .collect()
    }

    /// Order-independent form: each loop starts at its least segment and the holes
    /// are sorted, so equal regions give one request digest and one native input.
    pub(super) fn canonicalize(mut self) -> Self {
        let positive_zero = |value: f64| if value == 0.0 { 0.0 } else { value };
        let point = |point: ffi::NativePoint| ffi::NativePoint {
            x: positive_zero(point.x),
            y: positive_zero(point.y),
            z: positive_zero(point.z),
        };
        for segment in self.loops.iter_mut().flatten() {
            segment.digest = segment.digest.map(positive_zero);
            let native = &mut segment.native;
            *native = ffi::NativeSegment {
                start: point(native.start),
                end: point(native.end),
                center: point(native.center),
                normal: point(native.normal),
                control_1: point(native.control_1),
                control_2: point(native.control_2),
                radius: positive_zero(native.radius),
                ..*native
            };
        }
        let digest_order = |left: &[RegionSegment], right: &[RegionSegment]| {
            compare_planar_encoding(
                &left
                    .iter()
                    .flat_map(|segment| segment.digest)
                    .collect::<Vec<_>>(),
                &right
                    .iter()
                    .flat_map(|segment| segment.digest)
                    .collect::<Vec<_>>(),
            )
        };
        for segments in &mut self.loops {
            let rotated = |start: usize| {
                segments[start..]
                    .iter()
                    .chain(&segments[..start])
                    .copied()
                    .collect::<Vec<_>>()
            };
            let canonical_start = (0..segments.len())
                .min_by(|left, right| digest_order(&rotated(*left), &rotated(*right)))
                .expect("validated planar region loop is non-empty");
            segments.rotate_left(canonical_start);
        }
        self.loops[1..].sort_by(|left, right| digest_order(left, right));
        self
    }
}

pub(super) fn flatten_planar_region(
    outer: &PlanarProfileLoop,
    holes: &[PlanarProfileLoop],
    operation: &'static str,
    input: &str,
) -> Result<PlanarRegionPayload, GeometryError> {
    if holes.is_empty() || holes.len() > limits::REGION_HOLES {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Planar region requires 1..=64 holes".to_owned(),
        ));
    }
    let mut loops = Vec::with_capacity(holes.len() + 1);
    for (loop_index, planar_loop) in std::iter::once(outer).chain(holes).enumerate() {
        match planar_loop {
            PlanarProfileLoop::Segments(segments) => {
                validate_mixed_profile(segments, operation, input)?;
                let origin = planar_segment_endpoints(&segments[0]).0;
                let mut signed_area = 0.0;
                let mut compensation = 0.0;
                for segment in segments {
                    let adjusted = planar_segment_signed_area(segment, origin) - compensation;
                    let next = signed_area + adjusted;
                    compensation = (next - signed_area) - adjusted;
                    signed_area = next;
                }
                if !signed_area.is_finite() || signed_area.abs() <= MIN_LENGTH_MM * MIN_LENGTH_MM {
                    return Err(parameter_error(
                        GeometryErrorCode::InvalidProfile,
                        operation,
                        input,
                        "Planar region loop has zero signed area".to_owned(),
                    ));
                }
                let should_reverse = (loop_index == 0) != (signed_area > 0.0);
                let oriented = if should_reverse {
                    reverse_planar_segments(segments)
                } else {
                    segments.clone()
                };
                loops.push(
                    oriented
                        .iter()
                        .map(|segment| RegionSegment {
                            digest: planar_segment_digest_values(segment),
                            native: native_planar_segment(segment),
                        })
                        .collect(),
                );
            }
            PlanarProfileLoop::Circle {
                center_mm,
                radius_mm,
            } => {
                validate_circle(*center_mm, *radius_mm, operation, input)?;
                let hole = loop_index != 0;
                loops.push(vec![RegionSegment {
                    digest: circle_digest_values(*center_mm, *radius_mm, hole),
                    native: native_circle(*center_mm, *radius_mm, hole),
                }]);
            }
        }
    }
    if loops.iter().map(Vec::len).sum::<usize>() > limits::REGION_SEGMENTS {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            "Planar region exceeds the segment limit".to_owned(),
        ));
    }
    Ok(PlanarRegionPayload { loops })
}

pub(super) fn compare_planar_encoding(left: &[f64], right: &[f64]) -> std::cmp::Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| left.total_cmp(right))
        .find(|ordering| !ordering.is_eq())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}
