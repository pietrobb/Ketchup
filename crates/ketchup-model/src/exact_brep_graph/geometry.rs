use super::*;

pub(super) fn rotate_planar_geometry(
    geometry: &mut ExactBRepPlanarGeometry,
    orientation_degrees: f64,
) -> Result<(), ExactBRepGraphError> {
    if !orientation_degrees.is_finite() || !(-180.0..180.0).contains(&orientation_degrees) {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let (sin, cos) = orientation_degrees.to_radians().sin_cos();
    map_planar_geometry(
        geometry,
        PlanarRigidMap {
            linear: [[cos, -sin], [sin, cos]],
            offset: [0.0, 0.0],
        },
    )
}

/// A rotation or reflection of the plane followed by a translation.
#[derive(Clone, Copy)]
pub(super) struct PlanarRigidMap {
    pub(super) linear: [[f64; 2]; 2],
    pub(super) offset: [f64; 2],
}

impl PlanarRigidMap {
    pub(super) fn mirrors(self) -> bool {
        self.linear[0][0] * self.linear[1][1] - self.linear[0][1] * self.linear[1][0] < 0.0
    }

    pub(super) fn apply(self, point_bits: &mut [u64; 2]) -> Result<(), ExactBRepGraphError> {
        let [x, y] = point_bits.map(f64::from_bits);
        let mapped =
            [0, 1].map(|row| self.linear[row][0] * x + self.linear[row][1] * y + self.offset[row]);
        if mapped
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
        {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        *point_bits = mapped.map(canonical_bits);
        Ok(())
    }
}

pub(super) fn map_planar_geometry(
    geometry: &mut ExactBRepPlanarGeometry,
    map: PlanarRigidMap,
) -> Result<(), ExactBRepGraphError> {
    match geometry {
        ExactBRepPlanarGeometry::Boundary { segments, .. } => {
            for segment in segments {
                map_planar_segment(segment, map)?;
            }
        }
        ExactBRepPlanarGeometry::Circle { center_bits, .. } => map.apply(center_bits)?,
        ExactBRepPlanarGeometry::Spline { control_point_bits } => {
            for point in control_point_bits {
                map.apply(point)?;
            }
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            for planar_loop in std::iter::once(outer).chain(holes) {
                match planar_loop {
                    ExactBRepPlanarLoop::Boundary { segments } => {
                        for segment in segments {
                            map_planar_segment(segment, map)?;
                        }
                    }
                    ExactBRepPlanarLoop::Circle { center_bits, .. } => map.apply(center_bits)?,
                }
            }
        }
    }
    Ok(())
}

pub(super) fn map_planar_segment(
    segment: &mut ExactBRepPlanarSegment,
    map: PlanarRigidMap,
) -> Result<(), ExactBRepGraphError> {
    match segment {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } => {
            map.apply(start_bits)?;
            map.apply(end_bits)?;
        }
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            clockwise,
        } => {
            map.apply(start_bits)?;
            map.apply(end_bits)?;
            map.apply(center_bits)?;
            *clockwise ^= map.mirrors();
        }
        ExactBRepPlanarSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => {
            map.apply(start_bits)?;
            map.apply(control_1_bits)?;
            map.apply(control_2_bits)?;
            map.apply(end_bits)?;
        }
    }
    Ok(())
}

pub(super) fn exact_profile_face_reference(
    reference: &ProfileFaceReference,
) -> ExactBRepProfileFaceReference {
    match reference {
        ProfileFaceReference::Start => ExactBRepProfileFaceReference::Start,
        ProfileFaceReference::End => ExactBRepProfileFaceReference::End,
        ProfileFaceReference::Segment {
            entity_id,
            source_name,
        } => ExactBRepProfileFaceReference::Segment {
            entity_id: *entity_id,
            source_name: source_name.clone(),
        },
        ProfileFaceReference::NamedResult(name) => {
            ExactBRepProfileFaceReference::NamedResult { name: name.clone() }
        }
    }
}

pub(super) fn topology_selectors(
    references: &[TopologicalElementRef],
    expected_kind: TopologicalElementKind,
    target_feature_id: FeatureId,
) -> Result<Vec<ExactBRepTopologySelector>, ExactBRepGraphError> {
    if references.is_empty() || references.len() > MAX_EXACT_BREP_TOPOLOGY_SELECTORS {
        return Err(ExactBRepGraphError::InvalidTopologySelector);
    }
    references
        .iter()
        .map(|reference| {
            if reference.kind != expected_kind
                || reference.producer_feature_id != target_feature_id
                || !reference.has_valid_lineage()
            {
                return Err(ExactBRepGraphError::InvalidTopologySelector);
            }
            Ok(ExactBRepTopologySelector {
                kind: match expected_kind {
                    TopologicalElementKind::Face => ExactBRepTopologyKind::Face,
                    TopologicalElementKind::Edge => ExactBRepTopologyKind::Edge,
                    TopologicalElementKind::Vertex => {
                        return Err(ExactBRepGraphError::InvalidTopologySelector);
                    }
                },
                reference_bytes: reference
                    .to_bytes()
                    .map_err(ExactBRepGraphError::InvalidTopologyReference)?,
            })
        })
        .collect()
}

pub(super) fn solved_geometry(
    region: &SolvedSketchRegion,
    tolerance_mm: f64,
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if region.holes.is_empty() {
        return match solved_loop(&region.outer, tolerance_mm)? {
            ExactBRepPlanarLoop::Boundary { segments } => Ok(ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments,
            }),
            ExactBRepPlanarLoop::Circle {
                center_bits,
                radius_bits,
            } => Ok(ExactBRepPlanarGeometry::Circle {
                center_bits,
                radius_bits,
            }),
        };
    }
    Ok(ExactBRepPlanarGeometry::Region {
        outer: solved_loop(&region.outer, tolerance_mm)?,
        holes: region
            .holes
            .iter()
            .map(|hole| solved_loop(hole, tolerance_mm))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

pub(super) fn solved_loop(
    profile: &SolvedSketchRegionProfile,
    tolerance_mm: f64,
) -> Result<ExactBRepPlanarLoop, ExactBRepGraphError> {
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => {
            let ExactBRepPlanarGeometry::Boundary { segments, .. } = polygon_geometry(points)?
            else {
                unreachable!()
            };
            Ok(ExactBRepPlanarLoop::Boundary { segments })
        }
        SolvedSketchRegionProfile::Boundary(edges) => Ok(ExactBRepPlanarLoop::Boundary {
            segments: edges
                .iter()
                .map(|edge| match edge {
                    SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                        profile_segment(*start_mm, *end_mm, None, false)
                    }
                    SolvedSketchRegionEdge::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => profile_segment(*start_mm, *end_mm, Some(*center_mm), *clockwise),
                    SolvedSketchRegionEdge::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                    } => cubic_bezier_segment(
                        *start_mm,
                        *control_1_mm,
                        *control_2_mm,
                        *end_mm,
                        tolerance_mm,
                    ),
                })
                .collect::<Result<Vec<_>, _>>()?,
        }),
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => Ok(ExactBRepPlanarLoop::Circle {
            center_bits: valid_point(*center_mm)?.map(f64::to_bits),
            radius_bits: positive_distance(*radius_mm, tolerance_mm)?,
        }),
    }
}

pub(super) fn polygon_geometry(
    points: &[[f64; 2]],
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if points.len() < 3 {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    let segments = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| profile_segment(*start, *end, None, false))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactBRepPlanarGeometry::Boundary {
        closed: true,
        segments,
    })
}

pub(super) fn boundary_geometry(
    segments: &[ProfileSegment],
    closed: bool,
    tolerance_mm: f64,
) -> Result<ExactBRepPlanarGeometry, ExactBRepGraphError> {
    if segments.is_empty() {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    // A spline closing on itself is smooth through its closing point: a periodic curve
    // through its distinct points.
    if let [ProfileSegment::Spline { points_mm }] = segments
        && closed
        && let Some((_, points)) = points_mm.split_last()
    {
        return Ok(ExactBRepPlanarGeometry::Spline {
            control_point_bits: point_bits(points)?,
        });
    }
    if let [
        ProfileSegment::CircularArc {
            start_mm: first_start,
            end_mm: first_end,
            center_mm: first_center,
            clockwise: first_clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: second_start,
            end_mm: second_end,
            center_mm: second_center,
            clockwise: second_clockwise,
        },
    ] = segments
        && closed
        && first_start == second_end
        && first_end == second_start
        && first_center == second_center
        && first_clockwise == second_clockwise
    {
        let start_radius = [
            first_start[0] - first_center[0],
            first_start[1] - first_center[1],
        ];
        let end_radius = [
            first_end[0] - first_center[0],
            first_end[1] - first_center[1],
        ];
        let radius = start_radius[0].hypot(start_radius[1]);
        let antipodal_error =
            (start_radius[0] + end_radius[0]).hypot(start_radius[1] + end_radius[1]);
        if antipodal_error <= ROUNDING * radius.max(1.0) {
            return Ok(ExactBRepPlanarGeometry::Circle {
                center_bits: valid_point(*first_center)?.map(f64::to_bits),
                radius_bits: positive_distance(radius, tolerance_mm)?,
            });
        }
    }
    let segments = segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                profile_segment(*start_mm, *end_mm, None, false)
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => profile_segment(*start_mm, *end_mm, Some(*center_mm), *clockwise),
            ProfileSegment::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => cubic_bezier_segment(
                *start_mm,
                *control_1_mm,
                *control_2_mm,
                *end_mm,
                tolerance_mm,
            ),
            ProfileSegment::Spline { .. } => Err(ExactBRepGraphError::InvalidParameter),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExactBRepPlanarGeometry::Boundary { closed, segments })
}

pub(super) fn spatial_path(
    source_feature_id: FeatureId,
    segments: &[SpatialPathSegment],
    tolerance_mm: f64,
) -> Result<ExactBRepSpatialPath, ExactBRepGraphError> {
    let path = ExactBRepSpatialPath {
        source_feature_id: source_feature_id.0,
        segments: segments
            .iter()
            .map(|segment| match segment {
                SpatialPathSegment::Line { start_mm, end_mm } => {
                    ExactBRepSpatialPathSegment::Line {
                        start_bits: start_mm.map(canonical_bits),
                        end_bits: end_mm.map(canonical_bits),
                    }
                }
                SpatialPathSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    normal,
                    clockwise,
                } => ExactBRepSpatialPathSegment::CircularArc {
                    start_bits: start_mm.map(canonical_bits),
                    end_bits: end_mm.map(canonical_bits),
                    center_bits: center_mm.map(canonical_bits),
                    normal_bits: normal.map(canonical_bits),
                    clockwise: *clockwise,
                },
                SpatialPathSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => ExactBRepSpatialPathSegment::CubicBezier {
                    start_bits: start_mm.map(canonical_bits),
                    control_1_bits: control_1_mm.map(canonical_bits),
                    control_2_bits: control_2_mm.map(canonical_bits),
                    end_bits: end_mm.map(canonical_bits),
                },
            })
            .collect(),
    };
    valid_spatial_path(&path, tolerance_mm)
        .then_some(path)
        .ok_or(ExactBRepGraphError::InvalidParameter)
}

pub(super) fn valid_spatial_path(path: &ExactBRepSpatialPath, tolerance_mm: f64) -> bool {
    if path.source_feature_id == 0 {
        return false;
    }
    let segments = path
        .segments
        .iter()
        .map(|segment| match segment {
            ExactBRepSpatialPathSegment::Line {
                start_bits,
                end_bits,
            } => SpatialPathSegment::Line {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
            ExactBRepSpatialPathSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                normal_bits,
                clockwise,
            } => SpatialPathSegment::CircularArc {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
                center_mm: center_bits.map(f64::from_bits),
                normal: normal_bits.map(f64::from_bits),
                clockwise: *clockwise,
            },
            ExactBRepSpatialPathSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => SpatialPathSegment::CubicBezier {
                start_mm: start_bits.map(f64::from_bits),
                control_1_mm: control_1_bits.map(f64::from_bits),
                control_2_mm: control_2_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
        })
        .collect::<Vec<_>>();
    crate::document::is_valid_spatial_sweep_path(&segments, tolerance_mm)
}

pub(super) fn profile_segment(
    start_mm: [f64; 2],
    end_mm: [f64; 2],
    center_mm: Option<[f64; 2]>,
    clockwise: bool,
) -> Result<ExactBRepPlanarSegment, ExactBRepGraphError> {
    let start_bits = valid_point(start_mm)?.map(f64::to_bits);
    let end_bits = valid_point(end_mm)?.map(f64::to_bits);
    if start_bits == end_bits {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(if let Some(center_mm) = center_mm {
        ExactBRepPlanarSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits: valid_point(center_mm)?.map(f64::to_bits),
            clockwise,
        }
    } else {
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        }
    })
}

pub(super) fn cubic_bezier_segment(
    start_mm: [f64; 2],
    control_1_mm: [f64; 2],
    control_2_mm: [f64; 2],
    end_mm: [f64; 2],
    tolerance_mm: f64,
) -> Result<ExactBRepPlanarSegment, ExactBRepGraphError> {
    let start = valid_point(start_mm)?;
    let end = valid_point(end_mm)?;
    if (start[0] - end[0]).hypot(start[1] - end[1]) <= tolerance_mm {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(ExactBRepPlanarSegment::CubicBezier {
        start_bits: start.map(f64::to_bits),
        control_1_bits: valid_point(control_1_mm)?.map(f64::to_bits),
        control_2_bits: valid_point(control_2_mm)?.map(f64::to_bits),
        end_bits: end.map(f64::to_bits),
    })
}

pub(super) fn point_bits(points: &[[f64; 2]]) -> Result<Vec<[u64; 2]>, ExactBRepGraphError> {
    if points.len() < 2 {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    points
        .iter()
        .map(|point| Ok(valid_point(*point)?.map(f64::to_bits)))
        .collect()
}

pub(super) fn identity_frame() -> [u64; 12] {
    frame_bits(
        WorkplaneFrame::principal(ketchup_geometry::sketch::PrincipalPlane::Xy),
        [0.0, 0.0, 1.0],
    )
}

pub(super) fn frame_bits(frame: WorkplaneFrame, direction: [f64; 3]) -> [u64; 12] {
    Frame {
        origin: frame.origin_mm.into(),
        x: frame.x_axis.into(),
        y: frame.y_axis.into(),
        z: direction.into(),
    }
    .to_array()
    .map(f64::to_bits)
}

pub(super) fn linear_interval(
    direction: [f64; 3],
    start_mm: f64,
    end_mm: f64,
    tolerance_mm: f64,
) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
    let length = direction[0].hypot(direction[1]).hypot(direction[2]);
    if direction.iter().any(|component| !component.is_finite())
        || (length - 1.0).abs() > ROUNDING
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || start_mm.abs() > MAX_ABS_MM
        || end_mm.abs() > MAX_ABS_MM
        || end_mm - start_mm <= tolerance_mm
    {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(ExactBRepLinearInterval {
        direction_bits: direction.map(canonical_bits),
        start_bits: canonical_bits(start_mm),
        end_bits: canonical_bits(end_mm),
    })
}

pub(super) fn through_all_interval(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
    tolerance_mm: f64,
) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
    let [minimum, maximum] = projected_bounds(origin_mm, direction, target_bounds, tolerance_mm)?;
    linear_interval(direction, minimum - 1.0, maximum + 1.0, tolerance_mm)
}

pub(super) fn through_all_distance(
    origin_mm: [f64; 3],
    direction: [f64; 3],
    target_bounds: [[f64; 3]; 2],
    tolerance_mm: f64,
) -> Result<f64, ExactBRepGraphError> {
    let [_minimum, maximum] = projected_bounds(origin_mm, direction, target_bounds, tolerance_mm)?;
    let distance = maximum + 1.0;
    if distance > MAX_ABS_MM {
        return Err(ExactBRepGraphError::ResourceLimit);
    }
    Ok(distance)
}

pub(super) fn validate_blind_interval(
    origin_mm: [f64; 3],
    interval: ExactBRepLinearInterval,
    blind_end_mm: f64,
    target_bounds: [[f64; 3]; 2],
    tolerance_mm: f64,
) -> Result<(), ExactBRepGraphError> {
    const TOLERANCE_MM: f64 = APPROXIMATION;
    let [minimum, maximum] =
        projected_bounds(origin_mm, interval.direction(), target_bounds, tolerance_mm)?;
    if interval.start_mm() < minimum - TOLERANCE_MM
        || interval.end_mm() > maximum + TOLERANCE_MM
        || blind_end_mm <= minimum + TOLERANCE_MM
        || blind_end_mm >= maximum - TOLERANCE_MM
    {
        return Err(ExactBRepGraphError::InvalidParameter);
    }
    Ok(())
}
