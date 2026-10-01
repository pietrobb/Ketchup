use super::*;
use ketchup_geometry::linalg::dot;

#[must_use]
pub fn has_complete_manifold_adjacency(topology: &TopologyEvidence) -> bool {
    if topology.faces.len() != topology.face_count as usize
        || topology.edges.len() != topology.edge_count as usize
        || topology
            .faces
            .iter()
            .enumerate()
            .any(|(ordinal, face)| face.ordinal != ordinal as u32)
        || topology
            .edges
            .iter()
            .enumerate()
            .any(|(ordinal, edge)| edge.ordinal != ordinal as u32)
    {
        return false;
    }

    topology.faces.iter().all(|face| {
        face.edge_count as usize == face.edge_ordinals.len()
            && !face.edge_ordinals.is_empty()
            && face.edge_ordinals.windows(2).all(|pair| pair[0] < pair[1])
            && face.edge_ordinals.iter().all(|edge_ordinal| {
                topology.edges.iter().any(|edge| {
                    edge.ordinal == *edge_ordinal
                        && edge.adjacent_face_ordinals.contains(&face.ordinal)
                })
            })
    }) && topology.edges.iter().all(|edge| {
        edge.adjacent_face_ordinals.len() == 2
            && edge.adjacent_face_ordinals[0] < edge.adjacent_face_ordinals[1]
            && edge.adjacent_face_ordinals.iter().all(|face_ordinal| {
                topology.faces.iter().any(|face| {
                    face.ordinal == *face_ordinal && face.edge_ordinals.contains(&edge.ordinal)
                })
            })
    })
}

pub fn validate_closed_planar_profile(points: &[Point3]) -> Result<(), GeometryError> {
    let input = format!("profile:{points:?}");
    if points.len() != 4
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "A0 supports exactly four finite planar profile vertices".to_owned(),
        ));
    }
    let z = points[0].z;
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left.x * right.y - right.x * left.y)
        .sum::<f64>();
    if points.iter().any(|point| (point.z - z).abs() > ROUNDING)
        || twice_area.abs() <= NEGLIGIBLE
        || segments_intersect(points[0], points[1], points[2], points[3])
        || segments_intersect(points[1], points[2], points[3], points[0])
    {
        return Err(parameter_error(
            GeometryErrorCode::InvalidProfile,
            "validate_profile",
            &input,
            "Profile is non-planar, degenerate, or self-intersecting".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn segments_intersect(a: Point3, b: Point3, c: Point3, d: Point3) -> bool {
    fn orientation(a: Point3, b: Point3, c: Point3) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    let first = orientation(a, b, c);
    let second = orientation(a, b, d);
    let third = orientation(c, d, a);
    let fourth = orientation(c, d, b);
    first * second < 0.0 && third * fourth < 0.0
}

pub(super) fn native_status(status: u8) -> GeometryErrorCode {
    match status {
        1 => GeometryErrorCode::InvalidParameter,
        2 => GeometryErrorCode::NonFiniteParameter,
        3 => GeometryErrorCode::NoGeometricChange,
        4 => GeometryErrorCode::DegenerateOperation,
        5 => GeometryErrorCode::InvalidShape,
        6 => GeometryErrorCode::BackendException,
        _ => GeometryErrorCode::NullResult,
    }
}

pub(super) fn validate_box(
    spec: BoxSpec,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    for (value, name) in [
        (spec.origin_mm.x, "origin_x"),
        (spec.origin_mm.y, "origin_y"),
        (spec.origin_mm.z, "origin_z"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    for (value, name) in [
        (spec.size_mm.x, "size_x"),
        (spec.size_mm.y, "size_y"),
        (spec.size_mm.z, "size_z"),
    ] {
        validate_length(value, name, operation, input)?;
    }
    for (origin, size, name) in [
        (spec.origin_mm.x, spec.size_mm.x, "max_x"),
        (spec.origin_mm.y, spec.size_mm.y, "max_y"),
        (spec.origin_mm.z, spec.size_mm.z, "max_z"),
    ] {
        validate_coordinate(origin + size, name, operation, input)?;
    }
    Ok(())
}

pub(super) fn validate_general_revolve_axis_angle(
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    for (value, name) in [
        (axis_start_mm[0], "axis_start_x"),
        (axis_start_mm[1], "axis_start_y"),
        (axis_end_mm[0], "axis_end_x"),
        (axis_end_mm[1], "axis_end_y"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    if axis_start_mm == axis_end_mm {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            "Revolve axis must have non-zero length".to_owned(),
        ));
    }
    if !angle_degrees.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            "Revolve angle must be finite".to_owned(),
        ));
    }
    if !(0.0 < angle_degrees && angle_degrees <= 360.0) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            "Revolve angle must be within (0, 360] degrees".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_general_revolve_profile(
    segments: &[PlanarProfileSegment],
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
    input: &str,
) -> Result<(), GeometryError> {
    let operation = "revolve_general_profile";
    let invalid = |diagnostic: String| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic,
        )
    };
    if !(2..=MAX_PLANAR_LOOP_SEGMENTS).contains(&segments.len()) {
        return Err(invalid(
            "Revolve profile requires 2..=64 segments".to_owned(),
        ));
    }
    validate_general_revolve_axis_angle(
        axis_start_mm,
        axis_end_mm,
        angle_degrees,
        operation,
        input,
    )?;
    let endpoints = planar_segment_endpoints;
    for (index, segment) in segments.iter().enumerate() {
        let (start, end) = endpoints(segment);
        for (coordinate, name) in [
            (start[0], "start_x"),
            (start[1], "start_y"),
            (end[0], "end_x"),
            (end[1], "end_y"),
        ] {
            validate_coordinate(coordinate, name, operation, input)?;
        }
        if start == end {
            return Err(invalid(format!("Profile segment {index} is degenerate")));
        }
        if let PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } = segment
        {
            for (coordinate, name) in [
                (control_1_mm[0], "control_1_x"),
                (control_1_mm[1], "control_1_y"),
                (control_2_mm[0], "control_2_x"),
                (control_2_mm[1], "control_2_y"),
            ] {
                validate_coordinate(coordinate, name, operation, input)?;
            }
        }
        if let PlanarProfileSegment::CircularArc { center_mm, .. } = segment {
            validate_coordinate(center_mm[0], "center_x", operation, input)?;
            validate_coordinate(center_mm[1], "center_y", operation, input)?;
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            if start_radius < MIN_LENGTH_MM
                || (start_radius - end_radius).abs()
                    > ROUNDING * start_radius.max(end_radius).max(1.0)
            {
                return Err(invalid(format!(
                    "Profile arc {index} has inconsistent radius"
                )));
            }
        }
        let (next_start, _) = endpoints(&segments[(index + 1) % segments.len()]);
        if end != next_start {
            return Err(invalid(format!("Profile is open after segment {index}")));
        }
    }
    Ok(())
}

pub(super) fn validate_mixed_profile(
    segments: &[PlanarProfileSegment],
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    let invalid = |diagnostic: String| {
        parameter_error(
            GeometryErrorCode::InvalidProfile,
            operation,
            input,
            diagnostic,
        )
    };
    let line_only = segments
        .iter()
        .all(|segment| matches!(segment, PlanarProfileSegment::Line { .. }));
    if !(2..=MAX_PLANAR_LOOP_SEGMENTS).contains(&segments.len())
        || (line_only && segments.len() < 3)
    {
        return Err(invalid(
            "Segmented profile requires 2..=64 segments; line-only polygons require at least three lines".to_owned(),
        ));
    }
    let endpoints = planar_segment_endpoints;
    for (index, segment) in segments.iter().enumerate() {
        let (start, end) = endpoints(segment);
        for (coordinate, name) in [
            (start[0], "start_x"),
            (start[1], "start_y"),
            (end[0], "end_x"),
            (end[1], "end_y"),
        ] {
            validate_coordinate(coordinate, name, operation, input)?;
        }
        if start == end {
            return Err(invalid(format!("Profile segment {index} is degenerate")));
        }
        if let PlanarProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } = segment
        {
            for (coordinate, name) in [
                (control_1_mm[0], "control_1_x"),
                (control_1_mm[1], "control_1_y"),
                (control_2_mm[0], "control_2_x"),
                (control_2_mm[1], "control_2_y"),
            ] {
                validate_coordinate(coordinate, name, operation, input)?;
            }
        }
        if let PlanarProfileSegment::CircularArc { center_mm, .. } = segment {
            validate_coordinate(center_mm[0], "center_x", operation, input)?;
            validate_coordinate(center_mm[1], "center_y", operation, input)?;
            let start_radius = (start[0] - center_mm[0]).hypot(start[1] - center_mm[1]);
            let end_radius = (end[0] - center_mm[0]).hypot(end[1] - center_mm[1]);
            if start_radius < MIN_LENGTH_MM
                || (start_radius - end_radius).abs()
                    > ROUNDING * start_radius.max(end_radius).max(1.0)
            {
                return Err(invalid(format!(
                    "Profile arc {index} has inconsistent radius"
                )));
            }
        }
        let (next_start, _) = endpoints(&segments[(index + 1) % segments.len()]);
        if end != next_start {
            return Err(invalid(format!("Profile is open after segment {index}")));
        }
    }
    if line_only && !is_simple_linear_planar_profile(segments) {
        return Err(invalid(
            "Line-only profile must be a simple non-degenerate polygon".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn is_simple_linear_planar_profile(segments: &[PlanarProfileSegment]) -> bool {
    let points = segments
        .iter()
        .filter_map(|segment| match segment {
            PlanarProfileSegment::Line { start_mm, .. } => Some(*start_mm),
            PlanarProfileSegment::CircularArc { .. } | PlanarProfileSegment::CubicBezier { .. } => {
                None
            }
        })
        .collect::<Vec<_>>();
    if points.len() != segments.len()
        || points.iter().enumerate().any(|(index, point)| {
            points[index + 1..].iter().any(|candidate| {
                (point[0] - candidate[0]).abs() <= ROUNDING
                    && (point[1] - candidate[1]).abs() <= ROUNDING
            })
        })
    {
        return false;
    }
    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>();
    if !twice_area.is_finite() || twice_area.abs() <= ROUNDING {
        return false;
    }
    for left in 0..points.len() {
        let left_next = (left + 1) % points.len();
        for right in (left + 1)..points.len() {
            let right_next = (right + 1) % points.len();
            if left == right_next || left_next == right {
                continue;
            }
            if planar_segments_intersect(
                points[left],
                points[left_next],
                points[right],
                points[right_next],
            ) {
                return false;
            }
        }
    }
    true
}

pub(super) fn planar_segments_intersect(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> bool {
    let cross = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0])
    };
    let on_segment = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        point[0] >= start[0].min(end[0]) - ROUNDING
            && point[0] <= start[0].max(end[0]) + ROUNDING
            && point[1] >= start[1].min(end[1]) - ROUNDING
            && point[1] <= start[1].max(end[1]) + ROUNDING
    };
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    if ((ab_c > ROUNDING && ab_d < -ROUNDING) || (ab_c < -ROUNDING && ab_d > ROUNDING))
        && ((cd_a > ROUNDING && cd_b < -ROUNDING) || (cd_a < -ROUNDING && cd_b > ROUNDING))
    {
        return true;
    }
    (ab_c.abs() <= ROUNDING && on_segment(a, b, c))
        || (ab_d.abs() <= ROUNDING && on_segment(a, b, d))
        || (cd_a.abs() <= ROUNDING && on_segment(c, d, a))
        || (cd_b.abs() <= ROUNDING && on_segment(c, d, b))
}

pub(super) fn validate_circle(
    center_mm: [f64; 2],
    radius_mm: f64,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    validate_coordinate(center_mm[0], "center_x", operation, input)?;
    validate_coordinate(center_mm[1], "center_y", operation, input)?;
    validate_length(radius_mm, "radius_mm", operation, input)?;
    for (value, name) in [
        (center_mm[0] - radius_mm, "min_x"),
        (center_mm[0] + radius_mm, "max_x"),
        (center_mm[1] - radius_mm, "min_y"),
        (center_mm[1] + radius_mm, "max_y"),
    ] {
        validate_coordinate(value, name, operation, input)?;
    }
    Ok(())
}

pub(super) fn validate_length(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if !(MIN_LENGTH_MM..=MAX_LENGTH_MM).contains(&value) {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} must be within {MIN_LENGTH_MM}..={MAX_LENGTH_MM} mm"),
        ));
    }
    Ok(())
}

pub(super) fn validate_coordinate(
    value: f64,
    name: &str,
    operation: &'static str,
    input: &str,
) -> Result<(), GeometryError> {
    if !value.is_finite() {
        return Err(parameter_error(
            GeometryErrorCode::NonFiniteParameter,
            operation,
            input,
            format!("{name} must be finite"),
        ));
    }
    if value.abs() > MAX_COORDINATE_MM {
        return Err(parameter_error(
            GeometryErrorCode::InvalidParameter,
            operation,
            input,
            format!("{name} exceeds the local coordinate envelope"),
        ));
    }
    Ok(())
}

pub(super) fn parameter_error(
    code: GeometryErrorCode,
    operation: &'static str,
    input: &str,
    diagnostic: String,
) -> GeometryError {
    GeometryError {
        code,
        diagnostic,
        operation,
        input_digest: stable_digest(input),
        backend_fingerprint: BACKEND_FINGERPRINT,
    }
}

pub(super) fn box_input(label: &str, spec: BoxSpec) -> String {
    format!(
        "{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
        label,
        spec.origin_mm.x.to_bits(),
        spec.origin_mm.y.to_bits(),
        spec.origin_mm.z.to_bits(),
        spec.size_mm.x.to_bits(),
        spec.size_mm.y.to_bits(),
        spec.size_mm.z.to_bits()
    )
}

pub(super) fn subtract3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

pub(super) fn squared_distance3(left: [f64; 3], right: [f64; 3]) -> f64 {
    let delta = subtract3(left, right);
    dot(delta, delta)
}

pub(super) fn stable_digest(value: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}
