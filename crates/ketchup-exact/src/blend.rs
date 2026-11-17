use super::*;

/// How two single-face surfaces meet along the edges they share, measured at
/// interior points of every shared edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeamContinuity {
    /// Total length of the shared edges.
    pub length_mm: f64,
    /// Largest distance from the first surface's seam edge to the second.
    pub gap_mm: f64,
    /// Largest angle between the two surfaces' normals.
    pub normal_angle_rad: f64,
    /// Largest difference between the two normal curvatures across the seam.
    pub curvature_difference_per_mm: f64,
    /// The second surface's largest curvature across the seam, signed by the
    /// first surface's normal.
    pub second_curvature_per_mm: f64,
}

fn continuity_order(continuity: LoftSurfaceContinuity) -> u8 {
    match continuity {
        LoftSurfaceContinuity::Position => 0,
        LoftSurfaceContinuity::Tangent => 1,
        LoftSurfaceContinuity::Curvature => 2,
    }
}

fn single_surface(body: &ExactBody) -> bool {
    body.topology.solid_count == 0 && body.topology.face_count == 1
}

impl ExactBackend {
    /// A surface across the gap between two single-face surfaces, from the
    /// edge of each that faces the other. Each side meets its surface in
    /// position, tangent plane or curvature, and every seam is measured: a
    /// blend that misses an edge by more than the network approximation, or
    /// a requested tangent or curvature by more than the blend tolerances, is
    /// refused. The blend's other two sides are straight lines between the
    /// facing edges' near ends.
    pub fn blend_surfaces(
        &self,
        first: &ExactBody,
        first_continuity: LoftSurfaceContinuity,
        second: &ExactBody,
        second_continuity: LoftSurfaceContinuity,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "blend_surfaces";
        let (first_order, second_order) = (
            continuity_order(first_continuity),
            continuity_order(second_continuity),
        );
        let input = format!(
            "{operation}:{}:{first_order}:{}:{second_order}",
            first.result_fingerprint, second.result_fingerprint
        );
        if !single_surface(first)
            || !single_surface(second)
            || first.result_fingerprint == second.result_fingerprint
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "Surface blend joins two different single-face surfaces".to_owned(),
            ));
        }
        let native_first = first.owned_native("Surface blend first", operation, &input)?;
        let native_second = second.owned_native("Surface blend second", operation, &input)?;
        collect_output(
            ffi::blend_surfaces_native(native_first, first_order, native_second, second_order),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    /// How `first` meets `second` along the edges of `first` that lie on
    /// `second`. Both must be single-face surfaces that share an edge.
    pub fn measure_seam(
        &self,
        first: &ExactBody,
        second: &ExactBody,
    ) -> Result<SeamContinuity, GeometryError> {
        let operation = "measure_seam";
        let input = format!(
            "{operation}:{}:{}",
            first.result_fingerprint, second.result_fingerprint
        );
        let native_first = first.owned_native("Seam first surface", operation, &input)?;
        let native_second = second.owned_native("Seam second surface", operation, &input)?;
        let seam = ffi::measure_seam_native(native_first, native_second);
        if seam.status != 0 {
            return Err(parameter_error(
                match seam.status {
                    1 => GeometryErrorCode::InvalidParameter,
                    6 => GeometryErrorCode::BackendException,
                    _ => GeometryErrorCode::InvalidShape,
                },
                operation,
                &input,
                seam.diagnostic,
            ));
        }
        let measured = [
            seam.length_mm,
            seam.gap_mm,
            seam.normal_angle_rad,
            seam.curvature_difference_per_mm,
            seam.second_curvature_per_mm,
        ];
        if measured.iter().any(|value| !value.is_finite()) || seam.length_mm <= 0.0 {
            return Err(parameter_error(
                GeometryErrorCode::InvalidShape,
                operation,
                &input,
                "Seam measurement is not finite".to_owned(),
            ));
        }
        Ok(SeamContinuity {
            length_mm: seam.length_mm,
            gap_mm: seam.gap_mm,
            normal_angle_rad: seam.normal_angle_rad,
            curvature_difference_per_mm: seam.curvature_difference_per_mm,
            second_curvature_per_mm: seam.second_curvature_per_mm,
        })
    }
}
