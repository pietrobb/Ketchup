use super::*;

impl ExactBackend {
    pub fn transform_body(
        &self,
        body: &ExactBody,
        matrix: &[f64; 16],
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("transform_body:{}:{matrix:?}", body.result_fingerprint);
        if !matrix.iter().all(|value| value.is_finite())
            || matrix[12] != 0.0
            || matrix[13] != 0.0
            || matrix[14] != 0.0
            || matrix[15] != 1.0
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "transform_body",
                &input,
                "body transform must be a finite affine 4x4 matrix".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "transform_body",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::transform_body_native(native, matrix),
            "transform_body",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn combine_bodies(
        &self,
        base: &ExactBody,
        added: &ExactBody,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "combine_bodies:{}:{}",
            base.result_fingerprint, added.result_fingerprint
        );
        let native_base = base.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Base exact body lost its owned native shape".to_owned(),
            operation: "combine_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_added = added.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Added exact body lost its owned native shape".to_owned(),
            operation: "combine_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::combine_bodies_native(native_base, native_added),
            "combine_bodies",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn trim_body_by_plane(
        &self,
        body: &ExactBody,
        origin_mm: [f64; 3],
        normal: [f64; 3],
        keep_point_mm: [f64; 3],
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "trim_body_by_plane:{}:{origin_mm:?}:{normal:?}:{keep_point_mm:?}",
            body.result_fingerprint
        );
        if origin_mm
            .into_iter()
            .chain(normal)
            .chain(keep_point_mm)
            .any(|value| !value.is_finite())
            || normal.iter().map(|value| value * value).sum::<f64>() <= ROUNDING * ROUNDING
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "trim_body_by_plane",
                &input,
                "Plane trim requires finite points and a non-degenerate normal".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Plane trim body lost its owned native shape".to_owned(),
            operation: "trim_body_by_plane",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        collect_output(
            ffi::trim_body_by_plane_native(
                native,
                origin_mm[0],
                origin_mm[1],
                origin_mm[2],
                normal[0],
                normal[1],
                normal[2],
                keep_point_mm[0],
                keep_point_mm[1],
                keep_point_mm[2],
            ),
            "trim_body_by_plane",
            &input,
            HistoryConfidence::Partial,
        )
    }

    /// Read-only narrow phase. Empty common results are valid; backend failures
    /// are always errors. Contact tolerance is in mm and never suppresses positive volume.
    pub fn query_body_pair(
        &self,
        left: &ExactBody,
        right: &ExactBody,
        contact_tolerance_mm: f64,
    ) -> Result<ExactPairQueryResult, GeometryError> {
        let input = format!(
            "pair:{}:{}:{contact_tolerance_mm:?}",
            left.result_fingerprint, right.result_fingerprint
        );
        let error = |code, diagnostic| parameter_error(code, "query_body_pair", &input, diagnostic);
        if !contact_tolerance_mm.is_finite() || contact_tolerance_mm < 0.0 {
            return Err(error(
                GeometryErrorCode::InvalidParameter,
                "Contact tolerance must be finite and nonnegative".to_owned(),
            ));
        }
        let left = left.native.as_ref().ok_or_else(|| {
            error(
                GeometryErrorCode::NullResult,
                "Left native body unavailable".to_owned(),
            )
        })?;
        let right = right.native.as_ref().ok_or_else(|| {
            error(
                GeometryErrorCode::NullResult,
                "Right native body unavailable".to_owned(),
            )
        })?;
        let result = ffi::query_body_pair_native(left, right);
        if result.status != 0 {
            return Err(error(
                if result.status == 6 {
                    GeometryErrorCode::BackendException
                } else {
                    GeometryErrorCode::InvalidShape
                },
                result.diagnostic,
            ));
        }
        if !result.common_volume_mm3.is_finite()
            || result.common_volume_mm3 < 0.0
            || !result.common_contact_area_mm2.is_finite()
            || result.common_contact_area_mm2 < 0.0
            || !result.distance_mm.is_finite()
            || result.distance_mm < 0.0
            || (result.common_volume_mm3 > 0.0
                && (result.common_contact_area_mm2 > 0.0 || result.distance_mm != 0.0))
            || (result.common_contact_area_mm2 > 0.0 && result.distance_mm != 0.0)
        {
            return Err(error(
                GeometryErrorCode::InvalidShape,
                "Invalid native pair measurements".to_owned(),
            ));
        }
        Ok(ExactPairQueryResult {
            relation: if result.common_volume_mm3 > 0.0 {
                ExactPairRelation::Penetrating
            } else if result.distance_mm <= contact_tolerance_mm {
                ExactPairRelation::Touching
            } else {
                ExactPairRelation::Separated
            },
            common_volume_mm3: result.common_volume_mm3,
            common_contact_area_mm2: result.common_contact_area_mm2,
            distance_mm: result.distance_mm,
        })
    }

    pub fn boolean_bodies(
        &self,
        target: &ExactBody,
        tool: &ExactBody,
        operation: ExactBodyBooleanOperation,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!(
            "boolean_bodies:{operation:?}:{}:{}",
            target.result_fingerprint, tool.result_fingerprint
        );
        let native_target = target.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Target exact body lost its owned native shape".to_owned(),
            operation: "boolean_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let native_tool = tool.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Tool exact body lost its owned native shape".to_owned(),
            operation: "boolean_bodies",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let operation_code = match operation {
            ExactBodyBooleanOperation::Cut => 0,
            ExactBodyBooleanOperation::Union => 1,
            ExactBodyBooleanOperation::Intersect => 2,
            ExactBodyBooleanOperation::Split => 3,
        };
        collect_output(
            ffi::boolean_bodies_native(native_target, native_tool, operation_code),
            "boolean_bodies",
            &input,
            HistoryConfidence::Partial,
        )
    }
}
