use super::*;

impl ExactBackend {
    pub fn sweep_axial_tool(
        &self,
        spec: AxialToolSweepSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "sweep_axial_tool";
        let input = format!("{operation}:{spec:?}");
        validate_length(spec.radius_mm, "radius_mm", operation, &input)?;
        validate_length(spec.axial_length_mm, "axial_length_mm", operation, &input)?;
        let motion = match spec.motion {
            AxialToolMotion::Line { start_mm, end_mm } => {
                for (index, value) in start_mm.into_iter().chain(end_mm).enumerate() {
                    validate_coordinate(value, &format!("point_{index}"), operation, &input)?;
                }
                SpatialProfileSegment::Line { start_mm, end_mm }
            }
            AxialToolMotion::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                for (index, value) in start_mm
                    .into_iter()
                    .chain(end_mm)
                    .chain(center_mm)
                    .enumerate()
                {
                    validate_coordinate(value, &format!("point_{index}"), operation, &input)?;
                }
                let start_radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
                let end_radius = (end_mm[0] - center_mm[0]).hypot(end_mm[1] - center_mm[1]);
                if (start_mm[2] - end_mm[2]).abs() > ROUNDING
                    || (start_mm[2] - center_mm[2]).abs() > ROUNDING
                    || start_radius <= ROUNDING
                    || (start_radius - end_radius).abs() > DEFAULT_LINEAR_TOLERANCE_MM
                    || start_mm == end_mm
                {
                    return Err(parameter_error(
                        GeometryErrorCode::InvalidParameter,
                        operation,
                        &input,
                        "Axial tool arc must be planar, non-closed, concentric, and non-degenerate"
                            .to_owned(),
                    ));
                }
                SpatialProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    normal: [0.0, 0.0, 1.0],
                    clockwise,
                }
            }
        };
        collect_output(
            ffi::sweep_axial_tool_native(
                &native_spatial_segment(&motion),
                spec.radius_mm,
                spec.axial_length_mm,
            ),
            operation,
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn extrude_mixed_profile(
        &self,
        segments: &[PlanarProfileSegment],
        height_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_mixed_profile:{segments:?}:{:016x}",
            height_mm.to_bits()
        );
        validate_mixed_profile(segments, "extrude_mixed_profile", &input)?;
        validate_length(height_mm, "height_mm", "extrude_mixed_profile", &input)?;
        collect_output(
            ffi::extrude_mixed_profile_native(&native_planar_segments(segments), height_mm),
            "extrude_mixed_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn extrude_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        height_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "extrude_planar_region:{outer:?}:{holes:?}:{:016x}",
            height_mm.to_bits()
        );
        validate_length(height_mm, "height_mm", "extrude_planar_region", &input)?;
        let region = flatten_planar_region(outer, holes, "extrude_planar_region", &input)?;
        collect_output(
            ffi::extrude_planar_region_native(
                &region.native_segments(),
                &region.loop_segment_counts(),
                height_mm,
            ),
            "extrude_planar_region",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn revolve_general_profile(
        &self,
        segments: &[PlanarProfileSegment],
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "revolve_general_profile:{segments:?}:{axis_start_mm:?}:{axis_end_mm:?}:{:016x}",
            angle_degrees.to_bits()
        );
        validate_general_revolve_profile(
            segments,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            &input,
        )?;
        collect_output(
            ffi::revolve_general_profile_native(
                &native_planar_segments(segments),
                axis_start_mm[0],
                axis_start_mm[1],
                axis_end_mm[0],
                axis_end_mm[1],
                angle_degrees,
            ),
            "revolve_general_profile",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn revolve_planar_region(
        &self,
        outer: &PlanarProfileLoop,
        holes: &[PlanarProfileLoop],
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "revolve_planar_region:{outer:?}:{holes:?}:{axis_start_mm:?}:{axis_end_mm:?}:{:016x}",
            angle_degrees.to_bits()
        );
        validate_general_revolve_axis_angle(
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            "revolve_planar_region",
            &input,
        )?;
        let region = flatten_planar_region(outer, holes, "revolve_planar_region", &input)?;
        collect_output(
            ffi::revolve_planar_region_native(
                &region.native_segments(),
                &region.loop_segment_counts(),
                axis_start_mm[0],
                axis_start_mm[1],
                axis_end_mm[0],
                axis_end_mm[1],
                angle_degrees,
            ),
            "revolve_planar_region",
            &input,
            HistoryConfidence::Complete,
        )
    }

    pub fn shell_body(
        &self,
        body: &ExactBody,
        face_ordinals: &[u32],
        thickness_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.shell_body_with_direction(body, face_ordinals, thickness_mm, ShellDirection::Inward)
    }

    pub fn shell_body_with_direction(
        &self,
        body: &ExactBody,
        face_ordinals: &[u32],
        thickness_mm: f64,
        direction: ShellDirection,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "shell_body:{}:{face_ordinals:?}:{:016x}:{direction:?}",
            body.result_fingerprint,
            thickness_mm.to_bits()
        );
        validate_length(thickness_mm, "thickness_mm", "shell_body", &input)?;
        if face_ordinals.len() > 64
            || face_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || face_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.face_count)
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "shell_body",
                &input,
                "Shell faces must be a canonical in-range selection".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "shell_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::shell_body_native(
                native,
                face_ordinals,
                thickness_mm,
                match direction {
                    ShellDirection::Inward => 0,
                    ShellDirection::Outward => 1,
                    ShellDirection::Symmetric => 2,
                },
            ),
            "shell_body",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn offset_body_face(
        &self,
        body: &ExactBody,
        face_ordinal: u32,
        distance_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "offset_body_face:{}:{face_ordinal}:{:016x}",
            body.result_fingerprint,
            distance_mm.to_bits()
        );
        validate_length(distance_mm.abs(), "distance_mm", "offset_body_face", &input)?;
        if face_ordinal >= body.topology.face_count {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "offset_body_face",
                &input,
                "Face offset requires one in-range face and a non-zero distance".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "offset_body_face",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::offset_body_face_native(native, face_ordinal, distance_mm),
            "offset_body_face",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn finish_body(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        finish: EdgeFinish,
        amount_mm: f64,
    ) -> Result<ExactOpOutput, GeometryError> {
        self.finish_body_with_radius_stations(body, edge_ordinals, finish, amount_mm, &[])
    }

    pub fn finish_body_with_radius_stations(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        finish: EdgeFinish,
        amount_mm: f64,
        fillet_radius_stations: &[[f64; 2]],
    ) -> Result<ExactOpOutput, GeometryError> {
        let station_bits = fillet_radius_stations
            .iter()
            .map(|station| station.map(f64::to_bits))
            .collect::<Vec<_>>();
        let input = format!(
            "finish_body:{}:{edge_ordinals:?}:{finish:?}:{:016x}:{station_bits:?}",
            body.result_fingerprint,
            amount_mm.to_bits()
        );
        validate_length(amount_mm, "amount_mm", "finish_body", &input)?;
        let mut previous_position = 0.0;
        if fillet_radius_stations.len() > 32
            || (finish != EdgeFinish::Fillet && !fillet_radius_stations.is_empty())
            || (!fillet_radius_stations.is_empty()
                && fillet_radius_stations.last().map(|station| station[0]) != Some(1.0))
            || fillet_radius_stations.iter().any(|station| {
                let valid = station[0].is_finite()
                    && station[0] > previous_position
                    && station[0] <= 1.0
                    && validate_length(station[1], "radius", "finish_body", &input).is_ok();
                previous_position = station[0];
                !valid
            })
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body",
                &input,
                "Variable fillet stations are invalid or non-canonical".to_owned(),
            ));
        }
        if edge_ordinals.is_empty()
            || edge_ordinals.len() > 64
            || edge_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || edge_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.edge_count)
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body",
                &input,
                "Finish edges must be a non-empty canonical in-range selection".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "finish_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let flattened_stations = fillet_radius_stations
            .iter()
            .flat_map(|station| *station)
            .collect::<Vec<_>>();
        collect_output(
            ffi::finish_body_native(
                native,
                edge_ordinals,
                &[],
                amount_mm,
                finish == EdgeFinish::Fillet,
                &flattened_stations,
                0,
                0.0,
            ),
            "finish_body",
            &input,
            HistoryConfidence::Partial,
        )
    }

    pub fn finish_body_advanced_chamfer(
        &self,
        body: &ExactBody,
        edge_ordinals: &[u32],
        face_ordinals: &[u32],
        amount_mm: f64,
        mode: AdvancedChamferMode,
    ) -> Result<ExactOpOutput, GeometryError> {
        let (mode_code, secondary) = match mode {
            AdvancedChamferMode::TwoDistance { second_distance_mm } => (1, second_distance_mm),
            AdvancedChamferMode::DistanceAngle { angle_degrees } => (2, angle_degrees),
        };
        let input = format!(
            "finish_body_advanced_chamfer:{}:{edge_ordinals:?}:{face_ordinals:?}:{:016x}:{mode_code}:{:016x}",
            body.result_fingerprint,
            amount_mm.to_bits(),
            secondary.to_bits()
        );
        validate_length(
            amount_mm,
            "amount_mm",
            "finish_body_advanced_chamfer",
            &input,
        )?;
        let valid_secondary = match mode {
            AdvancedChamferMode::TwoDistance { second_distance_mm } => validate_length(
                second_distance_mm,
                "second_distance_mm",
                "finish_body_advanced_chamfer",
                &input,
            )
            .is_ok(),
            AdvancedChamferMode::DistanceAngle { angle_degrees } => {
                angle_degrees.is_finite() && angle_degrees > 0.1 && angle_degrees < 89.9
            }
        };
        if edge_ordinals.is_empty()
            || edge_ordinals.len() > 64
            || edge_ordinals.len() != face_ordinals.len()
            || edge_ordinals.windows(2).any(|pair| pair[0] >= pair[1])
            || edge_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.edge_count)
            || face_ordinals
                .iter()
                .any(|ordinal| *ordinal >= body.topology.face_count)
            || edge_ordinals
                .iter()
                .zip(face_ordinals)
                .any(|(edge_ordinal, face_ordinal)| {
                    !body.topology.edges.iter().any(|edge| {
                        edge.ordinal == *edge_ordinal
                            && edge.adjacent_face_ordinals.contains(face_ordinal)
                    })
                })
            || !valid_secondary
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "finish_body_advanced_chamfer",
                &input,
                "Advanced chamfer edge/face pairs or parameters are invalid".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "finish_body_advanced_chamfer",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::finish_body_native(
                native,
                edge_ordinals,
                face_ordinals,
                amount_mm,
                false,
                &[],
                mode_code,
                secondary,
            ),
            "finish_body_advanced_chamfer",
            &input,
            HistoryConfidence::Partial,
        )
    }
}
