use super::*;

impl ExactBackend {
    pub fn sweep_planar_profile(
        &self,
        profile: &[PlanarProfileSegment],
        path: &[PlanarProfileSegment],
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_planar_profile";
        let input = format!(
            "{operation}:{:?}:{:?}",
            digest_bits(&planar_segments_digest_values(profile)),
            digest_bits(&planar_segments_digest_values(path))
        );
        validate_mixed_profile(profile, operation, &input)?;
        if !(2..=MAX_SWEEP_PATH_SEGMENTS).contains(&path.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Curved Sweep requires between two and 64 path segments".to_owned(),
            ));
        }
        let path_metrics = |segment: &PlanarProfileSegment| {
            let invalid = || {
                parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path must contain only bounded non-degenerate lines, circular arcs, and cubic Bezier segments"
                        .to_owned(),
                )
            };
            let (start, end) = planar_segment_endpoints(segment);
            for (coordinate, name) in [
                (start[0], "path_start_x"),
                (start[1], "path_start_y"),
                (end[0], "path_end_x"),
                (end[1], "path_end_y"),
            ] {
                validate_coordinate(coordinate, name, operation, &input)?;
            }
            match segment {
                PlanarProfileSegment::Line { .. } => {
                    let direction = [end[0] - start[0], end[1] - start[1]];
                    let length = direction[0].hypot(direction[1]);
                    if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                        return Err(invalid());
                    }
                    let tangent = [direction[0] / length, direction[1] / length];
                    Ok((length, tangent, tangent))
                }
                PlanarProfileSegment::CircularArc {
                    center_mm,
                    clockwise,
                    ..
                } => {
                    validate_coordinate(center_mm[0], "path_center_x", operation, &input)?;
                    validate_coordinate(center_mm[1], "path_center_y", operation, &input)?;
                    let start_radius = [start[0] - center_mm[0], start[1] - center_mm[1]];
                    let end_radius = [end[0] - center_mm[0], end[1] - center_mm[1]];
                    let radius = start_radius[0].hypot(start_radius[1]);
                    let end_radius_length = end_radius[0].hypot(end_radius[1]);
                    if radius <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || (radius - end_radius_length).abs() > ROUNDING
                        || start == end
                    {
                        return Err(invalid());
                    }
                    let start_angle = start_radius[1].atan2(start_radius[0]);
                    let end_angle = end_radius[1].atan2(end_radius[0]);
                    let mut sweep = end_angle - start_angle;
                    if *clockwise {
                        if sweep >= 0.0 {
                            sweep -= std::f64::consts::TAU;
                        }
                    } else if sweep <= 0.0 {
                        sweep += std::f64::consts::TAU;
                    }
                    let length = radius * sweep.abs();
                    if length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM {
                        return Err(invalid());
                    }
                    let tangent = |radial: [f64; 2], radial_length: f64| {
                        let sign = if *clockwise { -1.0 } else { 1.0 };
                        [
                            sign * -radial[1] / radial_length,
                            sign * radial[0] / radial_length,
                        ]
                    };
                    Ok((
                        length,
                        tangent(start_radius, radius),
                        tangent(end_radius, end_radius_length),
                    ))
                }
                PlanarProfileSegment::CubicBezier {
                    control_1_mm,
                    control_2_mm,
                    ..
                } => {
                    for (coordinate, name) in [
                        (control_1_mm[0], "path_control_1_x"),
                        (control_1_mm[1], "path_control_1_y"),
                        (control_2_mm[0], "path_control_2_x"),
                        (control_2_mm[1], "path_control_2_y"),
                    ] {
                        validate_coordinate(coordinate, name, operation, &input)?;
                    }
                    let chord = [end[0] - start[0], end[1] - start[1]];
                    let start_handle = [control_1_mm[0] - start[0], control_1_mm[1] - start[1]];
                    let end_handle = [end[0] - control_2_mm[0], end[1] - control_2_mm[1]];
                    let middle = [
                        control_2_mm[0] - control_1_mm[0],
                        control_2_mm[1] - control_1_mm[1],
                    ];
                    let chord_squared = chord[0] * chord[0] + chord[1] * chord[1];
                    let start_length = start_handle[0].hypot(start_handle[1]);
                    let end_length = end_handle[0].hypot(end_handle[1]);
                    let length = start_length + middle[0].hypot(middle[1]) + end_length;
                    let projection_1 = start_handle[0] * chord[0] + start_handle[1] * chord[1];
                    let control_2_from_start =
                        [control_2_mm[0] - start[0], control_2_mm[1] - start[1]];
                    let projection_2 =
                        control_2_from_start[0] * chord[0] + control_2_from_start[1] * chord[1];
                    if start_length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || end_length <= MIN_SWEEP_PATH_SEGMENT_LENGTH_MM
                        || projection_1 <= 0.0
                        || projection_2 < projection_1
                        || projection_2 >= chord_squared
                    {
                        return Err(invalid());
                    }
                    Ok((
                        length,
                        [
                            start_handle[0] / start_length,
                            start_handle[1] / start_length,
                        ],
                        [end_handle[0] / end_length, end_handle[1] / end_length],
                    ))
                }
            }
        };
        let metrics = path
            .iter()
            .map(path_metrics)
            .collect::<Result<Vec<_>, _>>()?;
        for (segments, metrics) in path.windows(2).zip(metrics.windows(2)) {
            if planar_segment_endpoints(&segments[0]).1 != planar_segment_endpoints(&segments[1]).0
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path segments are disconnected".to_owned(),
                ));
            }
            let dot = metrics[0].2[0] * metrics[1].1[0] + metrics[0].2[1] * metrics[1].1[1];
            let cross = metrics[0].2[0] * metrics[1].1[1] - metrics[0].2[1] * metrics[1].1[0];
            if dot < 1.0 - ROUNDING || cross.abs() > ROUNDING {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &input,
                    "Sweep path segments must be C1 tangent-continuous".to_owned(),
                ));
            }
        }
        if sweep_path_self_intersects(path, &metrics) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Sweep path must not self-intersect".to_owned(),
            ));
        }
        validate_length(
            metrics.iter().map(|metrics| metrics.0).sum(),
            "path_length",
            operation,
            &input,
        )?;
        let output = collect_output(
            ffi::sweep_planar_profile_native(
                &native_planar_segments(profile),
                &native_planar_segments(path),
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn sweep_spatial_profile(
        &self,
        profile: &[PlanarProfileSegment],
        path: &[SpatialProfileSegment],
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_spatial_profile";
        let input = format!(
            "{operation}:{:?}:{:?}",
            digest_bits(&planar_segments_digest_values(profile)),
            digest_bits(&spatial_segments_digest_values(path))
        );
        validate_mixed_profile(profile, operation, &input)?;
        validate_spatial_sweep_path(path, operation, &input)?;
        let output = collect_output(
            ffi::sweep_spatial_profile_native(
                &native_planar_segments(profile),
                &native_spatial_segments(path),
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn loft_framed_profiles(
        &self,
        spec: &FramedLoftSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_controls(spec, None, LoftSurfaceContinuity::Position)
    }

    pub fn loft_framed_profiles_with_controls(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_body_kind(spec, guide, continuity, true)
    }

    pub fn loft_framed_surface(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.loft_framed_profiles_with_body_kind(spec, guide, continuity, false)
    }

    pub(super) fn loft_framed_profiles_with_body_kind(
        &self,
        spec: &FramedLoftSpec,
        guide: Option<&[SpatialProfileSegment]>,
        continuity: LoftSurfaceContinuity,
        make_solid: bool,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = if make_solid {
            "loft_framed_profiles"
        } else {
            "loft_framed_surface"
        };
        if guide.is_some() && continuity == LoftSurfaceContinuity::Curvature {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                operation,
                "Guided Loft does not support curvature continuity".to_owned(),
            ));
        }
        if let Some(guide) = guide {
            validate_spatial_sweep_path(guide, operation, operation)?;
        }
        let guide = guide.unwrap_or_default();
        let mut values = vec![spec.sections.len() as f64];
        let mut sections = Vec::with_capacity(spec.sections.len());
        let mut segments = Vec::new();
        let mut spline_points = Vec::new();
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                operation,
                "Framed Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                operation,
                operation,
            )?;
            let x_axis = [section.frame[3], section.frame[4], section.frame[5]];
            let y_axis = [section.frame[6], section.frame[7], section.frame[8]];
            let normal = [section.frame[9], section.frame[10], section.frame[11]];
            let norm = |axis: [f64; 3]| {
                axis.into_iter()
                    .map(|value| value * value)
                    .sum::<f64>()
                    .sqrt()
            };
            let dot = |left: [f64; 3], right: [f64; 3]| {
                left.into_iter()
                    .zip(right)
                    .map(|(left, right)| left * right)
                    .sum::<f64>()
            };
            let cross = [
                x_axis[1] * y_axis[2] - x_axis[2] * y_axis[1],
                x_axis[2] * y_axis[0] - x_axis[0] * y_axis[2],
                x_axis[0] * y_axis[1] - x_axis[1] * y_axis[0],
            ];
            if section.elevation_mm <= previous_elevation
                || section
                    .frame
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
                || (norm(x_axis) - 1.0).abs() > ACCUMULATED_ROUNDING
                || (norm(y_axis) - 1.0).abs() > ACCUMULATED_ROUNDING
                || (norm(normal) - 1.0).abs() > ACCUMULATED_ROUNDING
                || dot(x_axis, y_axis).abs() > ACCUMULATED_ROUNDING
                || dot(x_axis, normal).abs() > ACCUMULATED_ROUNDING
                || dot(y_axis, normal).abs() > ACCUMULATED_ROUNDING
                || dot(cross, normal) < 1.0 - ACCUMULATED_ROUNDING
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    operation,
                    operation,
                    "Framed Loft section frame or ordering is invalid".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            let point = |index: usize| {
                native_spatial_point([
                    section.frame[index],
                    section.frame[index + 1],
                    section.frame[index + 2],
                ])
            };
            let mut native_section = ffi::NativeLoftSection {
                origin: point(0),
                x_axis: point(3),
                y_axis: point(6),
                normal: point(9),
                elevation: section.elevation_mm,
                segment_count: 0,
                spline_point_count: 0,
            };
            // Digest kinds of the frozen request encoding: boundary, circle, spline.
            let (kind, count, payload) = match &section.profile {
                FramedLoftProfile::Planar(PlanarProfileLoop::Segments(profile)) => {
                    validate_mixed_profile(profile, operation, operation)?;
                    native_section.segment_count = profile.len() as u32;
                    segments.extend(native_planar_segments(profile));
                    (0.0, profile.len(), planar_segments_digest_values(profile))
                }
                FramedLoftProfile::Planar(PlanarProfileLoop::Circle {
                    center_mm,
                    radius_mm,
                }) => {
                    validate_circle(*center_mm, *radius_mm, operation, operation)?;
                    native_section.segment_count = 1;
                    segments.push(native_circle(*center_mm, *radius_mm, false));
                    (1.0, 1, vec![center_mm[0], center_mm[1], *radius_mm])
                }
                FramedLoftProfile::Spline { control_points_mm } => {
                    if !(4..=64).contains(&control_points_mm.len()) {
                        return Err(parameter_error(
                            GeometryErrorCode::InvalidParameter,
                            operation,
                            operation,
                            "Framed Loft spline section is outside the bounded envelope".to_owned(),
                        ));
                    }
                    let mut payload = Vec::with_capacity(control_points_mm.len() * 2);
                    for (point_index, point) in control_points_mm.iter().enumerate() {
                        for (axis, coordinate) in point.iter().copied().enumerate() {
                            validate_coordinate(
                                coordinate,
                                &format!("section_{section_index}_point_{point_index}_axis_{axis}"),
                                operation,
                                operation,
                            )?;
                            payload.push(coordinate);
                        }
                        spline_points.push(native_spatial_point([point[0], point[1], 0.0]));
                    }
                    native_section.spline_point_count = control_points_mm.len() as u32;
                    (2.0, control_points_mm.len(), payload)
                }
            };
            values.extend([kind, section.elevation_mm, count as f64]);
            values.extend(section.frame);
            values.extend(payload);
            sections.push(native_section);
        }
        let continuity_code = match continuity {
            LoftSurfaceContinuity::Position => 0,
            LoftSurfaceContinuity::Tangent => 1,
            LoftSurfaceContinuity::Curvature => 2,
        };
        let input = format!(
            "{operation}:{:?}:{:?}:{continuity_code}:{make_solid}",
            digest_bits(&values),
            digest_bits(&spatial_segments_digest_values(guide))
        );
        collect_output(
            ffi::loft_framed_profiles_native(
                &sections,
                &segments,
                &spline_points,
                &native_spatial_segments(guide),
                continuity_code,
                make_solid,
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn loft_planar_profiles(
        &self,
        spec: &PlanarLoftSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "loft_planar_profiles";
        let mut input = format!("{operation}:{}", spec.sections.len());
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Planar Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut segments = Vec::new();
        let mut section_segment_counts = Vec::with_capacity(spec.sections.len());
        let mut elevations = Vec::with_capacity(spec.sections.len());
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                operation,
                &input,
            )?;
            if section.elevation_mm <= previous_elevation {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    operation,
                    &input,
                    "Planar Loft section elevations must be strictly increasing".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            match &section.profile {
                PlanarProfileLoop::Segments(profile_segments) => {
                    validate_mixed_profile(profile_segments, operation, &input)?;
                    section_segment_counts.push(profile_segments.len() as u32);
                }
                PlanarProfileLoop::Circle {
                    center_mm,
                    radius_mm,
                } => {
                    validate_circle(*center_mm, *radius_mm, operation, &input)?;
                    section_segment_counts.push(1);
                }
            }
            input.push_str(&format!(
                ":{}:{:016x}:{:?}",
                section_segment_counts.last().unwrap(),
                section.elevation_mm.to_bits(),
                digest_bits(&planar_loop_digest_values(&section.profile))
            ));
            segments.extend(native_planar_loop(&section.profile));
            elevations.push(section.elevation_mm);
        }
        let output = collect_output(
            ffi::loft_planar_profiles_native(&segments, &section_segment_counts, &elevations),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        let bounds = output.body.topology.bounds_mm;
        for (name, coordinate) in [
            ("output_min_x", bounds.min.x),
            ("output_min_y", bounds.min.y),
            ("output_min_z", bounds.min.z),
            ("output_max_x", bounds.max.x),
            ("output_max_y", bounds.max.y),
            ("output_max_z", bounds.max.z),
        ] {
            validate_coordinate(coordinate, name, operation, &input)?;
        }
        Ok(output)
    }

    pub fn loft_spline(&self, spec: &SplineLoftSpec) -> Result<ExactOpOutput, GeometryError> {
        let mut input = format!("loft_spline:{}", spec.sections.len());
        let mut values = vec![spec.sections.len() as f64];
        if !(2..=16).contains(&spec.sections.len()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "loft_spline",
                &input,
                "Spline Loft requires 2 to 16 sections".to_owned(),
            ));
        }
        let mut previous_elevation = f64::NEG_INFINITY;
        for (section_index, section) in spec.sections.iter().enumerate() {
            validate_coordinate(
                section.elevation_mm,
                &format!("section_{section_index}_elevation"),
                "loft_spline",
                &input,
            )?;
            if !(4..=64).contains(&section.control_points_mm.len())
                || section.elevation_mm <= previous_elevation
            {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidParameter,
                    "loft_spline",
                    &input,
                    "Spline Loft sections are outside the bounded envelope".to_owned(),
                ));
            }
            previous_elevation = section.elevation_mm;
            input.push_str(&format!(
                ":{}:{:016x}",
                section.control_points_mm.len(),
                section.elevation_mm.to_bits()
            ));
            values.push(section.control_points_mm.len() as f64);
            values.push(section.elevation_mm);
            for (point_index, point) in section.control_points_mm.iter().enumerate() {
                for (axis, coordinate) in point.iter().copied().enumerate() {
                    validate_coordinate(
                        coordinate,
                        &format!("section_{section_index}_point_{point_index}_axis_{axis}"),
                        "loft_spline",
                        &input,
                    )?;
                    input.push_str(&format!(":{:016x}", coordinate.to_bits()));
                    values.push(coordinate);
                }
            }
        }
        let native = ffi::loft_spline_native(&values);
        collect_output(native, "loft_spline", &input, HistoryConfidence::Complete)
    }

    pub fn extrude_circle(&self, spec: CircleExtrudeSpec) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_circle:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.center_mm[0].to_bits(),
            spec.center_mm[1].to_bits(),
            spec.radius_mm.to_bits(),
            spec.height_mm.to_bits()
        );
        validate_circle(spec.center_mm, spec.radius_mm, "extrude_circle", &input)?;
        validate_length(spec.height_mm, "height_mm", "extrude_circle", &input)?;
        let native = ffi::extrude_circle_native(
            spec.center_mm[0],
            spec.center_mm[1],
            spec.radius_mm,
            spec.height_mm,
        );
        collect_output(
            native,
            "extrude_circle",
            &input,
            HistoryConfidence::Complete,
        )
    }
}
