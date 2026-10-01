use super::*;
use ketchup_tolerance::MAX_COORDINATE_MM;
use ketchup_tolerance::limits;

impl ExactBackend {
    pub fn make_box(&self, spec: BoxSpec) -> Result<ExactOpOutput, GeometryError> {
        let input = box_input("box", spec);
        validate_box(spec, "box", &input)?;
        let native = ffi::make_box_native(
            spec.origin_mm.x,
            spec.origin_mm.y,
            spec.origin_mm.z,
            spec.size_mm.x,
            spec.size_mm.y,
            spec.size_mm.z,
        );
        collect_output(native, "box", &input, HistoryConfidence::Complete)
    }

    pub fn offset_rectangle(
        &self,
        spec: RectangleOffsetSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_rectangle:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}",
            spec.min_mm[0].to_bits(),
            spec.min_mm[1].to_bits(),
            spec.max_mm[0].to_bits(),
            spec.max_mm[1].to_bits(),
            spec.distance_mm.to_bits()
        );
        for (name, coordinate) in [
            ("min_x", spec.min_mm[0]),
            ("min_y", spec.min_mm[1]),
            ("max_x", spec.max_mm[0]),
            ("max_y", spec.max_mm[1]),
        ] {
            validate_coordinate(coordinate, name, "offset_rectangle", &input)?;
        }
        let output_min = [
            spec.min_mm[0] - spec.distance_mm,
            spec.min_mm[1] - spec.distance_mm,
        ];
        let output_max = [
            spec.max_mm[0] + spec.distance_mm,
            spec.max_mm[1] + spec.distance_mm,
        ];
        if !spec.distance_mm.is_finite()
            || spec.distance_mm.abs() < limits::MIN_LENGTH_MM
            || spec.max_mm[0] <= spec.min_mm[0]
            || spec.max_mm[1] <= spec.min_mm[1]
            || output_max[0] - output_min[0] < limits::MIN_LENGTH_MM
            || output_max[1] - output_min[1] < limits::MIN_LENGTH_MM
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_rectangle",
                &input,
                "Rectangle offset is outside the bounded planar envelope".to_owned(),
            ));
        }
        for (name, coordinate) in [
            ("output_min_x", output_min[0]),
            ("output_min_y", output_min[1]),
            ("output_max_x", output_max[0]),
            ("output_max_y", output_max[1]),
        ] {
            validate_coordinate(coordinate, name, "offset_rectangle", &input)?;
        }
        let native = ffi::offset_rectangle_native(
            spec.min_mm[0],
            spec.min_mm[1],
            spec.max_mm[0],
            spec.max_mm[1],
            spec.distance_mm,
        );
        collect_output(
            native,
            "offset_rectangle",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn planar_surface_profile(
        &self,
        profile: &PlanarProfileLoop,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "planar_surface_profile";
        let input = format!("{operation}:{profile:?}");
        let PlanarProfileLoop::Segments(segments) = profile else {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &input,
                "Planar surface currently requires a bounded segmented loop".to_owned(),
            ));
        };
        validate_mixed_profile(segments, operation, &input)?;
        collect_output(
            ffi::planar_surface_profile_native(&native_planar_segments(segments)),
            operation,
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn trim_surface(
        &self,
        target: &ExactBody,
        cutter: &ExactBody,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "trim_surface";
        let input = format!(
            "{operation}:{}:{}",
            target.result_fingerprint, cutter.result_fingerprint
        );
        if target.topology.solid_count != 0
            || cutter.topology.solid_count != 0
            || target.topology.face_count == 0
            || cutter.topology.face_count == 0
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface trim requires non-solid target and cutter surfaces".to_owned(),
            ));
        }
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface trim target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_cutter = cutter.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface trim cutter lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::trim_surface_native(native_target, native_cutter),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn extend_planar_surface(
        &self,
        target: &ExactBody,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "extend_planar_surface";
        let input = format!(
            "{operation}:{}:{:016x}",
            target.result_fingerprint,
            distance_mm.to_bits()
        );
        validate_length(distance_mm, "distance_mm", operation, &input)?;
        if target.topology.solid_count != 0
            || target.topology.face_count != 1
            || target.topology.wire_count != 1
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface extend requires one simply bounded non-solid face".to_owned(),
            ));
        }
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface extend target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::extend_planar_surface_native(native_target, distance_mm),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn knit_surfaces(
        &self,
        surfaces: &[&ExactBody],
        tolerance_mm: f64,
        make_solid: bool,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "knit_surfaces";
        let input = format!(
            "{operation}:{make_solid}:{:016x}:{}",
            tolerance_mm.to_bits(),
            surfaces
                .iter()
                .map(|surface| surface.result_fingerprint.as_str())
                .collect::<Vec<_>>()
                .join(":")
        );
        if !(2..=256).contains(&surfaces.len())
            || !tolerance_mm.is_finite()
            || !(DEFAULT_LINEAR_TOLERANCE_MM..=10.0).contains(&tolerance_mm)
            || surfaces.iter().any(|surface| {
                surface.topology.solid_count != 0 || surface.topology.face_count == 0
            })
            || surfaces.iter().enumerate().any(|(index, surface)| {
                surfaces[..index]
                    .iter()
                    .any(|other| other.result_fingerprint == surface.result_fingerprint)
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                format!(
                    "Surface knit requires 2..256 distinct non-solid surfaces and a tolerance from {DEFAULT_LINEAR_TOLERANCE_MM} to 10 mm"
                ),
            ));
        }
        let missing_native = || GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface knit input lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        };
        let first = surfaces[0].native.as_ref().ok_or_else(missing_native)?;
        let second = surfaces[1].native.as_ref().ok_or_else(missing_native)?;
        let mut compound = collect_output(
            ffi::combine_surfaces_native(first, second),
            operation,
            &input,
            HistoryConfidence::None,
        )?;
        for surface in &surfaces[2..] {
            let added = surface.native.as_ref().ok_or_else(missing_native)?;
            let next = {
                let base = compound.body.native.as_ref().ok_or_else(missing_native)?;
                collect_output(
                    ffi::combine_surfaces_native(base, added),
                    operation,
                    &input,
                    HistoryConfidence::None,
                )?
            };
            compound = next;
        }
        let native_compound = compound.body.native.as_ref().ok_or_else(missing_native)?;
        collect_output(
            ffi::knit_surface_compound_native(native_compound, tolerance_mm, make_solid),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn thicken_surface(
        &self,
        surface: &ExactBody,
        thickness_mm: f64,
        direction: ShellDirection,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "thicken_surface";
        let input = format!(
            "{operation}:{}:{:016x}:{direction:?}",
            surface.result_fingerprint,
            thickness_mm.to_bits()
        );
        validate_length(thickness_mm, "thickness_mm", operation, &input)?;
        if surface.topology.solid_count != 0
            || surface.topology.face_count == 0
            || surface.topology.face_count > 256
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface thicken requires a valid non-solid face-bearing body".to_owned(),
            ));
        }
        let native_surface = surface.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Surface thicken target lost its owned native shape".to_owned(),
            operation,
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::thicken_surface_native(
                native_surface,
                thickness_mm,
                match direction {
                    ShellDirection::Inward => 0,
                    ShellDirection::Outward => 1,
                    ShellDirection::Symmetric => 2,
                },
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn offset_planar_profile(
        &self,
        profile: &PlanarProfileLoop,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_planar_profile:{profile:?}:{:016x}",
            distance_mm.to_bits()
        );
        if !distance_mm.is_finite() {
            return Err(parameter_error(
                GeometryErrorCode::NonFiniteParameter,
                "offset_planar_profile",
                &input,
                "Planar offset distance must be finite".to_owned(),
            ));
        }
        if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&distance_mm.abs()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_planar_profile",
                &input,
                "Planar offset distance is outside the bounded envelope".to_owned(),
            ));
        }
        if let PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } = profile
        {
            validate_circle(*center_mm, *radius_mm, "offset_planar_profile", &input)?;
            let output_radius_mm = *radius_mm + distance_mm;
            validate_circle(
                *center_mm,
                output_radius_mm,
                "offset_planar_profile",
                &input,
            )?;
            let output = collect_output(
                ffi::offset_planar_circle_native(
                    center_mm[0],
                    center_mm[1],
                    *radius_mm,
                    distance_mm,
                ),
                "offset_planar_profile",
                &input,
                HistoryConfidence::Complete,
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
                validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
            }
            return Ok(output);
        }
        let PlanarProfileLoop::Segments(segments) = profile else {
            unreachable!("all planar profile loop variants were handled")
        };
        validate_mixed_profile(segments, "offset_planar_profile", &input)?;
        for segment in segments {
            match segment {
                PlanarProfileSegment::Line { start_mm, end_mm } => validate_length(
                    (end_mm[0] - start_mm[0]).hypot(end_mm[1] - start_mm[1]),
                    "line_length",
                    "offset_planar_profile",
                    &input,
                )?,
                PlanarProfileSegment::CircularArc {
                    start_mm,
                    center_mm,
                    ..
                } => {
                    let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                    validate_length(radius, "arc_radius", "offset_planar_profile", &input)?;
                    for (name, coordinate) in [
                        ("arc_min_x", center_mm[0] - radius),
                        ("arc_min_y", center_mm[1] - radius),
                        ("arc_max_x", center_mm[0] + radius),
                        ("arc_max_y", center_mm[1] + radius),
                    ] {
                        validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
                    }
                }
                PlanarProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => validate_length(
                    (control_1_mm[0] - start_mm[0]).hypot(control_1_mm[1] - start_mm[1])
                        + (control_2_mm[0] - control_1_mm[0])
                            .hypot(control_2_mm[1] - control_1_mm[1])
                        + (end_mm[0] - control_2_mm[0]).hypot(end_mm[1] - control_2_mm[1]),
                    "cubic_control_polygon_length",
                    "offset_planar_profile",
                    &input,
                )?,
            }
        }
        let origin = planar_segment_endpoints(&segments[0]).0;
        let mut signed_area = 0.0;
        let mut compensation = 0.0;
        for segment in segments {
            let adjusted = planar_segment_signed_area(segment, origin) - compensation;
            let next = signed_area + adjusted;
            compensation = (next - signed_area) - adjusted;
            signed_area = next;
        }
        if !signed_area.is_finite()
            || signed_area.abs() <= limits::MIN_LENGTH_MM * limits::MIN_LENGTH_MM
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                "offset_planar_profile",
                &input,
                "Planar offset loop has zero signed area".to_owned(),
            ));
        }
        let mut segments = if signed_area < 0.0 {
            reverse_planar_segments(segments)
        } else {
            segments.to_vec()
        };
        let encoded = segments
            .iter()
            .map(planar_segment_digest_values)
            .collect::<Vec<_>>();
        let canonical_start = (0..segments.len())
            .min_by(|left, right| {
                (0..segments.len())
                    .flat_map(|offset| {
                        let left = &encoded[(left + offset) % segments.len()];
                        let right = &encoded[(right + offset) % segments.len()];
                        left.iter()
                            .zip(right)
                            .map(|(left, right)| left.total_cmp(right))
                    })
                    .find(|ordering| !ordering.is_eq())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("validated planar offset profile is non-empty");
        segments.rotate_left(canonical_start);
        let input = format!(
            "offset_planar_profile:{segments:?}:{:016x}",
            distance_mm.to_bits()
        );
        let output = collect_output(
            ffi::offset_planar_profile_native(&native_planar_segments(&segments), distance_mm),
            "offset_planar_profile",
            &input,
            HistoryConfidence::Complete,
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
            validate_coordinate(coordinate, name, "offset_planar_profile", &input)?;
        }
        Ok(output)
    }

    pub fn offset_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "offset_planar_region";
        let bounded_input = format!(
            "{operation}:loop_count={}:distance={:016x}",
            holes.len().saturating_add(1),
            distance_mm.to_bits()
        );
        if !distance_mm.is_finite() {
            return Err(parameter_error(
                GeometryErrorCode::NonFiniteParameter,
                operation,
                &bounded_input,
                "Planar region offset distance must be finite".to_owned(),
            ));
        }
        if !(limits::MIN_LENGTH_MM..=MAX_COORDINATE_MM).contains(&distance_mm.abs()) {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &bounded_input,
                "Planar region offset distance is outside the bounded envelope".to_owned(),
            ));
        }
        if holes.is_empty() || holes.len() > limits::REGION_HOLES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidProfile,
                operation,
                &bounded_input,
                "Planar region requires 1..=64 holes".to_owned(),
            ));
        }
        let mut segment_count = 0_usize;
        for planar_loop in std::iter::once(outer).chain(holes) {
            let loop_segment_count = match planar_loop {
                PlanarProfileLoop::Segments(segments) => segments.len(),
                PlanarProfileLoop::Circle { .. } => 1,
            };
            if loop_segment_count == 0 || loop_segment_count > limits::PATH_SEGMENTS {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &bounded_input,
                    "Planar region loop exceeds the segment limit".to_owned(),
                ));
            }
            segment_count = segment_count.saturating_add(loop_segment_count);
            if segment_count > limits::REGION_SEGMENTS {
                return Err(parameter_error(
                    GeometryErrorCode::InvalidProfile,
                    operation,
                    &bounded_input,
                    "Planar region exceeds the total segment limit".to_owned(),
                ));
            }
        }
        let region = flatten_planar_region(outer, holes, operation, &bounded_input)?.canonicalize();
        let loop_segment_counts = region.loop_segment_counts();
        let encoded_bits = digest_bits(&region.digest_values());
        let input = format!(
            "{operation}:{loop_segment_counts:?}:{encoded_bits:?}:{:016x}",
            distance_mm.to_bits()
        );
        let output = collect_output(
            ffi::offset_planar_region_native(
                &region.native_segments(),
                &loop_segment_counts,
                distance_mm,
            ),
            operation,
            &input,
            HistoryConfidence::Complete,
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
}
