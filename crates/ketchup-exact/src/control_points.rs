use super::*;
use ketchup_geometry::linalg::distance;
use ketchup_tolerance::{CONTROL_POINT_MATCH_MM, CONTROL_POINT_MATCH_SHARE};

/// One control point of a surface's B-spline form, which lies at `from_mm`,
/// to move by `offset_mm` and, with `weight`, to give a new (positive) weight.
/// Indices count from 0. A point of these indices elsewhere is not moved: the
/// control net changed since the move was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceControlPoint {
    pub u_index: u32,
    pub v_index: u32,
    pub from_mm: [f64; 3],
    pub offset_mm: [f64; 3],
    pub weight: Option<f64>,
}

/// The control net of a surface's B-spline form: `u_count` rows of
/// `v_count` control points.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceControlNet {
    pub u_count: u32,
    pub v_count: u32,
    /// Control point positions, row by row (v fastest).
    pub points_mm: Vec<[f64; 3]>,
    /// The weight of every control point, in the same order.
    pub weights: Vec<f64>,
}

impl SurfaceControlNet {
    /// How far from its point a move's `from_mm` may lie and still name it:
    /// [`CONTROL_POINT_MATCH_MM`], or less on a net whose neighbouring poles lie
    /// nearer than four times that (R1120ac-EXA-900: a match within the linear
    /// tolerance refused a position copied with fewer than all its digits).
    pub fn match_tolerance_mm(&self) -> f64 {
        let columns = usize::try_from(self.v_count).unwrap_or(usize::MAX);
        let point = |index: usize| self.points_mm.get(index).copied();
        let nearest = (0..self.points_mm.len())
            .flat_map(|index| {
                let next_column = (columns > 0 && (index + 1) % columns != 0)
                    .then(|| point(index + 1))
                    .flatten();
                [next_column, index.checked_add(columns).and_then(point)]
                    .into_iter()
                    .flatten()
                    .map(move |other| distance(self.points_mm[index], other))
            })
            .filter(|gap| *gap > DEFAULT_LINEAR_TOLERANCE_MM)
            .fold(f64::INFINITY, f64::min);
        (CONTROL_POINT_MATCH_SHARE * nearest)
            .clamp(DEFAULT_LINEAR_TOLERANCE_MM, CONTROL_POINT_MATCH_MM)
    }
}

/// What breaks `continuity` at a measured seam, if anything; the tolerances
/// are the surface blend's.
fn broken(seam: &SeamContinuity, continuity: LoftSurfaceContinuity) -> Option<String> {
    let allowed_curvature = SURFACE_BLEND_CURVATURE_SHARE
        * seam.second_curvature_per_mm.abs().max(1.0 / seam.length_mm);
    if seam.gap_mm > NETWORK_SURFACE_APPROXIMATION_MM {
        Some(format!(
            "a gap of {} mm, more than {NETWORK_SURFACE_APPROXIMATION_MM}",
            seam.gap_mm
        ))
    } else if continuity != LoftSurfaceContinuity::Position
        && seam.normal_angle_rad > SURFACE_BLEND_ANGLE_RAD
    {
        Some(format!(
            "normals that differ by {} rad, more than {SURFACE_BLEND_ANGLE_RAD}",
            seam.normal_angle_rad
        ))
    } else if continuity == LoftSurfaceContinuity::Curvature
        && seam.curvature_difference_per_mm > allowed_curvature
    {
        Some(format!(
            "curvatures that differ by {} per mm, more than {allowed_curvature}",
            seam.curvature_difference_per_mm
        ))
    } else {
        None
    }
}

fn continuity_name(continuity: LoftSurfaceContinuity) -> &'static str {
    match continuity {
        LoftSurfaceContinuity::Position => "position",
        LoftSurfaceContinuity::Tangent => "tangent",
        LoftSurfaceContinuity::Curvature => "curvature",
    }
}

impl ExactBackend {
    /// The control net of a single-face surface's B-spline form.
    pub fn surface_control_net(
        &self,
        surface: &ExactBody,
    ) -> Result<SurfaceControlNet, GeometryError> {
        let operation = "surface_control_net";
        let input = format!("{operation}:{}", surface.result_fingerprint);
        let native = surface.owned_native("Control net surface", operation, &input)?;
        let net = ffi::surface_control_net_native(native);
        if net.status != 0 {
            return Err(parameter_error(
                native_status(net.status),
                operation,
                &input,
                net.diagnostic,
            ));
        }
        let points = net.values.chunks_exact(4);
        Ok(SurfaceControlNet {
            u_count: net.u_count,
            v_count: net.v_count,
            points_mm: points
                .clone()
                .map(|point| [point[0], point[1], point[2]])
                .collect(),
            weights: points.map(|point| point[3]).collect(),
        })
    }

    /// The surface with one control point of its B-spline form moved (and
    /// optionally reweighted); the face keeps its trimming curves in the
    /// surface's parameters, so its edges follow the new shape. Every seam in
    /// `keep` is measured before and after: the surfaces must meet with that
    /// continuity before the move, and a move that breaks it is refused with
    /// the measured break.
    pub fn move_surface_control_point(
        &self,
        surface: &ExactBody,
        point: SurfaceControlPoint,
        keep: &[(&ExactBody, LoftSurfaceContinuity)],
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "move_surface_control_point";
        let SurfaceControlPoint {
            u_index,
            v_index,
            from_mm,
            offset_mm,
            weight,
        } = point;
        let mut input = format!(
            "{operation}:{}:{u_index}:{v_index}:{:?}:{:?}:{:?}",
            surface.result_fingerprint, from_mm, offset_mm, weight
        );
        for (neighbour, continuity) in keep {
            input.push_str(&format!(
                ":{}:{}",
                neighbour.result_fingerprint,
                continuity_name(*continuity)
            ));
        }
        let refuse = |diagnostic: String| {
            parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                diagnostic,
            )
        };
        if offset_mm
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        {
            return Err(refuse(
                "A control point moves by a finite offset within the model".to_owned(),
            ));
        }
        if weight.is_some_and(|weight| !(weight.is_finite() && weight > 0.0)) {
            return Err(refuse(
                "A control point's weight is finite and positive".to_owned(),
            ));
        }
        if offset_mm == [0.0; 3] && weight.is_none() {
            return Err(refuse(
                "Moving a control point needs an offset or a weight".to_owned(),
            ));
        }
        if from_mm
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        {
            return Err(refuse(
                "A control point lies at a finite position within the model".to_owned(),
            ));
        }
        // An earlier edit that changes the net, such as an approximation with more
        // points, would otherwise move another point unsaid (review 2026-11-19 og
        // EXA-800).
        let net = self.surface_control_net(surface)?;
        let at = (u_index < net.u_count && v_index < net.v_count)
            .then(|| u64::from(u_index) * u64::from(net.v_count) + u64::from(v_index))
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| net.points_mm.get(index).copied());
        let tolerance_mm = net.match_tolerance_mm();
        let shown = |point: [f64; 3]| format!("({}, {}, {}) mm", point[0], point[1], point[2]);
        match at {
            None => {
                return Err(refuse(format!(
                    "Control point ({u_index}, {v_index}), expected at {}, is outside the \
                     surface's {} x {} control net. If an earlier edit changed the net, move \
                     the point of the net again",
                    shown(from_mm),
                    net.u_count,
                    net.v_count
                )));
            }
            Some(at) if distance(at, from_mm) > tolerance_mm => {
                return Err(refuse(format!(
                    "Control point ({u_index}, {v_index}) lies at {}, not within {} mm of {} \
                     where the move expects it, in the surface's {} x {} control net. Read the \
                     net with inspect surface_control_net; if an earlier edit changed the net, \
                     move the point of the net again",
                    shown(at),
                    tolerance_mm,
                    shown(from_mm),
                    net.u_count,
                    net.v_count
                )));
            }
            Some(_) => {}
        }
        let native = surface.owned_native("Control point surface", operation, &input)?;
        for (index, (neighbour, continuity)) in keep.iter().enumerate() {
            let seam = self.measure_seam(surface, neighbour).map_err(|error| {
                refuse(format!(
                    "Kept surface {} does not meet the surface before the move: {}",
                    index + 1,
                    error.diagnostic
                ))
            })?;
            if let Some(failure) = broken(&seam, *continuity) {
                return Err(refuse(format!(
                    "Kept surface {} does not meet the surface with {} continuity before the \
                     move: it has {failure}",
                    index + 1,
                    continuity_name(*continuity)
                )));
            }
        }
        let output = collect_output(
            ffi::move_surface_control_point_native(
                native,
                u_index,
                v_index,
                ffi::NativePoint {
                    x: offset_mm[0],
                    y: offset_mm[1],
                    z: offset_mm[2],
                },
                weight.unwrap_or(0.0),
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )?;
        for (index, (neighbour, continuity)) in keep.iter().enumerate() {
            let failure = match self.measure_seam(&output.body, neighbour) {
                Ok(seam) => broken(&seam, *continuity),
                Err(error) => Some(format!("no shared edge left ({})", error.diagnostic)),
            };
            if let Some(failure) = failure {
                return Err(refuse(format!(
                    "Moving control point ({u_index}, {v_index}) breaks the {} continuity with \
                     kept surface {}: the seam would have {failure}. Control points next to a \
                     kept edge hold its position (first row), tangent plane (first two rows) \
                     and curvature (first three rows)",
                    continuity_name(*continuity),
                    index + 1
                )));
            }
        }
        Ok(output)
    }
}
