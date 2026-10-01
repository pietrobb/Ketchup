use super::*;

pub(super) fn canonical_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

pub(super) fn finite_coordinate(value: f64) -> Result<f64, ExactBRepGraphError> {
    if value.is_finite() && value.abs() <= MAX_ABS_MM {
        Ok(value)
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

pub(super) fn valid_point(point: [f64; 2]) -> Result<[f64; 2], ExactBRepGraphError> {
    Ok([finite_coordinate(point[0])?, finite_coordinate(point[1])?])
}

pub(super) fn positive_distance(value: f64, tolerance_mm: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && value > tolerance_mm && value <= MAX_ABS_MM {
        Ok(value.to_bits())
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

pub(super) fn signed_distance(value: f64, tolerance_mm: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && value.abs() > tolerance_mm && value.abs() <= MAX_ABS_MM {
        Ok(value.to_bits())
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

pub(super) fn planar_offset_distance(value: f64) -> Result<u64, ExactBRepGraphError> {
    if value.is_finite() && (EXACT_MIN_LENGTH_MM..=MAX_ABS_MM).contains(&value.abs()) {
        Ok(value.to_bits())
    } else {
        Err(ExactBRepGraphError::InvalidParameter)
    }
}

pub(super) fn valid_frame(bits: [u64; 12]) -> bool {
    bits.map(f64::from_bits)
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
}

pub(super) fn loop_geometry(planar_loop: &ExactBRepPlanarLoop) -> ExactBRepPlanarGeometry {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments: segments.clone(),
        },
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => ExactBRepPlanarGeometry::Circle {
            center_bits: *center_bits,
            radius_bits: *radius_bits,
        },
    }
}

pub(super) fn validate_geometry(
    geometry: &ExactBRepPlanarGeometry,
    tolerance_mm: f64,
) -> Result<usize, ExactBRepGraphError> {
    let valid_bits = |bits: [u64; 2]| {
        bits.map(f64::from_bits)
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
    };
    match geometry {
        ExactBRepPlanarGeometry::Boundary { closed, segments } => {
            if segments.len() > MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            if segments.is_empty() || *closed && segments.len() < 2 {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            let endpoints = segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } if valid_bits(*start_bits)
                        && valid_bits(*end_bits)
                        && start_bits != end_bits =>
                    {
                        Ok((*start_bits, *end_bits))
                    }
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        ..
                    } if valid_bits(*start_bits)
                        && valid_bits(*end_bits)
                        && valid_bits(*center_bits)
                        && start_bits != end_bits =>
                    {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        let center = center_bits.map(f64::from_bits);
                        let start_radius = (start[0] - center[0]).hypot(start[1] - center[1]);
                        let end_radius = (end[0] - center[0]).hypot(end[1] - center[1]);
                        let tolerance = start_radius.max(end_radius).max(1.0) * ROUNDING;
                        if start_radius <= tolerance_mm
                            || (start_radius - end_radius).abs() > tolerance
                        {
                            Err(ExactBRepGraphError::InvalidGraph)
                        } else {
                            Ok((*start_bits, *end_bits))
                        }
                    }
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } if valid_bits(*start_bits)
                        && valid_bits(*control_1_bits)
                        && valid_bits(*control_2_bits)
                        && valid_bits(*end_bits) =>
                    {
                        let start = start_bits.map(f64::from_bits);
                        let end = end_bits.map(f64::from_bits);
                        if (start[0] - end[0]).hypot(start[1] - end[1]) <= tolerance_mm {
                            Err(ExactBRepGraphError::InvalidGraph)
                        } else {
                            Ok((*start_bits, *end_bits))
                        }
                    }
                    _ => Err(ExactBRepGraphError::InvalidGraph),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if endpoints.windows(2).any(|pair| pair[0].1 != pair[1].0)
                || (*closed && endpoints.last().unwrap().1 != endpoints[0].0)
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(segments.len())
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let radius = f64::from_bits(*radius_bits);
            if !valid_bits(*center_bits)
                || !radius.is_finite()
                || radius <= tolerance_mm
                || radius > MAX_ABS_MM
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(1)
        }
        ExactBRepPlanarGeometry::Spline { control_point_bits } => {
            if control_point_bits.len() < 4
                || control_point_bits.iter().any(|point| !valid_bits(*point))
            {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            Ok(control_point_bits.len())
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            if holes.is_empty() {
                return Err(ExactBRepGraphError::InvalidGraph);
            }
            if holes.len() > MAX_EXACT_BREP_REGION_HOLES {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            let segment_count = holes.iter().try_fold(
                validate_geometry(&loop_geometry(outer), tolerance_mm)?,
                |segment_count, hole| {
                    segment_count
                        .checked_add(validate_geometry(&loop_geometry(hole), tolerance_mm)?)
                        .ok_or(ExactBRepGraphError::ResourceLimit)
                },
            )?;
            if segment_count > MAX_EXACT_BREP_REGION_SEGMENTS {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            Ok(segment_count)
        }
    }
}

pub(super) fn valid_linear_interval(interval: ExactBRepLinearInterval, tolerance_mm: f64) -> bool {
    let direction = interval.direction();
    let length = direction[0].hypot(direction[1]).hypot(direction[2]);
    let start = interval.start_mm();
    let end = interval.end_mm();
    direction.iter().all(|component| component.is_finite())
        && (length - 1.0).abs() <= ROUNDING
        && start.is_finite()
        && end.is_finite()
        && start.abs() <= MAX_ABS_MM
        && end.abs() <= MAX_ABS_MM
        && end - start > tolerance_mm
}

pub(super) fn valid_topology_selectors(
    selectors: &[ExactBRepTopologySelector],
    expected_kind: ExactBRepTopologyKind,
    document_id: u64,
    definition_id: u64,
    target: ExactBRepNodeId,
    prior_nodes: &[ExactBRepNode],
) -> bool {
    let Some(target_node) = prior_nodes.get(target.0 as usize) else {
        return false;
    };
    if selectors.is_empty() || selectors.len() > MAX_EXACT_BREP_TOPOLOGY_SELECTORS {
        return false;
    }
    let references = selectors
        .iter()
        .map(|selector| {
            (selector.kind == expected_kind)
                .then(|| selector.reference().ok())
                .flatten()
        })
        .collect::<Option<Vec<_>>>();
    references.is_some_and(|references| {
        references.iter().all(|reference| {
            reference.document_id.0 == document_id
                && reference.definition_id.0 == definition_id
                && reference.producer_feature_id.0 == target_node.source_feature_id
        }) && references.windows(2).all(|pair| pair[0] < pair[1])
    })
}

pub(super) fn operation_requires_v7(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let ExactBRepOperation::PlanarOffset { profile, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(profile.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. })
            if segments.iter().any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
    )
}

pub(super) fn operation_requires_v8(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let ExactBRepOperation::PlanarOffset { profile, .. } = operation else {
        return false;
    };
    matches!(
        profiles
            .get(profile.0 as usize)
            .map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Region { .. })
    )
}

pub(super) fn operation_requires_v9(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. }) if segments.len() > 1
    )
}

pub(super) fn operation_requires_v10(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. }) if segments.len() > 2
    )
}

pub(super) fn operation_requires_v11(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let ExactBRepOperation::Sweep { path, .. } = operation else {
        return false;
    };
    matches!(
        profiles.get(path.0 as usize).map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Boundary { segments, .. })
            if segments.iter().any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
    )
}

pub(super) fn operation_requires_v12(operation: &ExactBRepOperation) -> bool {
    matches!(operation, ExactBRepOperation::SpatialSweep { .. })
}

pub(super) fn operation_requires_v15(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::Loft { guide: Some(_), .. }
            | ExactBRepOperation::LoftSurface { guide: Some(_), .. }
            | ExactBRepOperation::Loft {
                continuity: ExactBRepLoftContinuity::Tangent | ExactBRepLoftContinuity::Curvature,
                ..
            }
            | ExactBRepOperation::LoftSurface {
                continuity: ExactBRepLoftContinuity::Tangent | ExactBRepLoftContinuity::Curvature,
                ..
            }
    )
}

pub(super) fn operation_requires_v23(operation: &ExactBRepOperation) -> bool {
    matches!(operation, ExactBRepOperation::SurfaceThicken { .. })
}

pub(super) fn operation_requires_v22(operation: &ExactBRepOperation) -> bool {
    matches!(operation, ExactBRepOperation::SurfaceKnit { .. })
}

pub(super) fn operation_requires_v21(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::SurfaceTrim { .. } | ExactBRepOperation::SurfaceExtend { .. }
    )
}

pub(super) fn operation_requires_v20(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::PlanarSurface { .. } | ExactBRepOperation::LoftSurface { .. }
    )
}

pub(super) fn operation_requires_v19(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::SheetMetal { .. } | ExactBRepOperation::WeldmentJoint { .. }
    )
}

pub(super) fn operation_requires_v18(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::EdgeFinish {
            chamfer_mode: ExactBRepChamferMode::TwoDistance { .. }
                | ExactBRepChamferMode::DistanceAngle { .. },
            ..
        }
    )
}

pub(super) fn operation_requires_v17(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::EdgeFinish {
            fillet_radius_stations,
            ..
        } if !fillet_radius_stations.is_empty()
    )
}

pub(super) fn operation_requires_v16(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::Shell {
            removed_faces,
            direction: ExactBRepShellDirection::Outward | ExactBRepShellDirection::Symmetric,
            ..
        } if removed_faces.len() <= MAX_EXACT_BREP_TOPOLOGY_SELECTORS
    ) || matches!(
        operation,
        ExactBRepOperation::Shell {
            removed_faces,
            direction: ExactBRepShellDirection::Inward,
            ..
        } if removed_faces.is_empty()
    )
}

pub(super) fn operation_requires_v14(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
) -> bool {
    let sections = match operation {
        ExactBRepOperation::Loft { sections, .. }
        | ExactBRepOperation::LoftSurface { sections, .. } => sections,
        _ => return false,
    };
    let mut has_spline = false;
    let mut has_planar = false;
    sections.iter().any(|section| {
        profiles
            .get(section.profile.0 as usize)
            .is_some_and(|profile| {
                match profile.geometry {
                    ExactBRepPlanarGeometry::Spline { .. } => has_spline = true,
                    _ => has_planar = true,
                }
                matches!(profile.geometry, ExactBRepPlanarGeometry::Region { .. })
                    || profile.frame_bits != identity_frame()
            })
    }) || (has_spline && has_planar)
}

pub(super) fn valid_operation_profiles(
    operation: &ExactBRepOperation,
    profiles: &[ExactBRepProfile],
    tolerance_mm: f64,
) -> bool {
    match operation {
        ExactBRepOperation::Loft { sections, .. }
        | ExactBRepOperation::LoftSurface { sections, .. } => {
            let mut hole_count = None;
            sections.iter().all(|section| {
                profiles
                    .get(section.profile.0 as usize)
                    .is_some_and(|profile| {
                        let current_hole_count = match &profile.geometry {
                            ExactBRepPlanarGeometry::Spline { control_point_bits }
                                if (4..=MAX_EXACT_BREP_LOFT_CONTROL_POINTS)
                                    .contains(&control_point_bits.len()) =>
                            {
                                0
                            }
                            ExactBRepPlanarGeometry::Boundary {
                                closed: true,
                                segments,
                            } if (2..=MAX_EXACT_BREP_PLANAR_LOOP_SEGMENTS)
                                .contains(&segments.len()) =>
                            {
                                0
                            }
                            ExactBRepPlanarGeometry::Circle { .. } => 0,
                            ExactBRepPlanarGeometry::Region { holes, .. } => holes.len(),
                            _ => return false,
                        };
                        match hole_count {
                            Some(expected) => expected == current_hole_count,
                            None => {
                                hole_count = Some(current_hole_count);
                                true
                            }
                        }
                    })
            })
        }
        ExactBRepOperation::PlanarOffset {
            profile,
            distance_bits,
        } => profiles.get(profile.0 as usize).is_some_and(|profile| {
            planar_offset_profile_bounds(profile, f64::from_bits(*distance_bits)).is_ok()
        }),
        ExactBRepOperation::PlanarSurface { profile } => {
            profiles.get(profile.0 as usize).is_some_and(|profile| {
                let supported = match &profile.geometry {
                    ExactBRepPlanarGeometry::Boundary { closed: true, .. }
                    | ExactBRepPlanarGeometry::Circle { .. } => true,
                    ExactBRepPlanarGeometry::Region { holes, .. } => holes.is_empty(),
                    _ => false,
                };
                supported && planar_surface_bounds(profile).is_ok()
            })
        }
        ExactBRepOperation::Sweep { profile, path } => {
            profile != path
                && matches!(
                    profiles
                        .get(profile.0 as usize)
                        .map(|profile| &profile.geometry),
                    Some(
                        ExactBRepPlanarGeometry::Boundary { closed: true, .. }
                            | ExactBRepPlanarGeometry::Circle { .. }
                            | ExactBRepPlanarGeometry::Region { .. }
                    )
                )
                && profiles
                    .get(profile.0 as usize)
                    .zip(profiles.get(path.0 as usize))
                    .is_some_and(|(profile, path)| {
                        sweep_profile_bounds(profile, path, tolerance_mm).is_ok()
                    })
        }
        ExactBRepOperation::SpatialSweep { profile, path } => {
            profiles.get(profile.0 as usize).is_some_and(|profile| {
                matches!(
                    profile.geometry,
                    ExactBRepPlanarGeometry::Boundary { closed: true, .. }
                        | ExactBRepPlanarGeometry::Circle { .. }
                        | ExactBRepPlanarGeometry::Region { .. }
                ) && spatial_sweep_bounds(profile, path, tolerance_mm).is_ok()
            })
        }
        _ => true,
    }
}

pub(super) fn valid_operation(
    operation: &ExactBRepOperation,
    document_id: u64,
    definition_id: u64,
    prior_nodes: &[ExactBRepNode],
    tolerance_mm: f64,
) -> bool {
    let positive = |bits| {
        let value = f64::from_bits(bits);
        value.is_finite() && value > tolerance_mm && value <= MAX_ABS_MM
    };
    let signed = |bits| {
        let value = f64::from_bits(bits);
        value.is_finite() && value.abs() > tolerance_mm && value.abs() <= MAX_ABS_MM
    };
    let is_surface_node = |id: ExactBRepNodeId| {
        prior_nodes.get(id.0 as usize).is_some_and(|node| {
            matches!(
                node.operation,
                ExactBRepOperation::PlanarSurface { .. }
                    | ExactBRepOperation::LoftSurface { .. }
                    | ExactBRepOperation::SurfaceTrim { .. }
                    | ExactBRepOperation::SurfaceExtend { .. }
                    | ExactBRepOperation::SurfaceKnit {
                        make_solid: false,
                        ..
                    }
            )
        })
    };
    match operation {
        ExactBRepOperation::Extrude {
            distance_bits,
            interval,
            ..
        } => {
            valid_linear_interval(*interval, tolerance_mm)
                && positive(*distance_bits)
                && *distance_bits == interval.length_mm().to_bits()
        }
        ExactBRepOperation::ProfileCut {
            depth_bits,
            interval,
            support_lineage_digest,
            ..
        } => {
            valid_linear_interval(*interval, tolerance_mm)
                && depth_bits
                    .is_none_or(|depth| positive(depth) && depth == interval.length_mm().to_bits())
                && support_lineage_digest
                    .as_ref()
                    .is_none_or(|digest| !digest.is_empty() && digest.len() <= 256)
        }
        ExactBRepOperation::Boolean { target, tool, .. } => target != tool,
        ExactBRepOperation::SurfaceTrim { target, cutter } => {
            target != cutter && is_surface_node(*target) && is_surface_node(*cutter)
        }
        ExactBRepOperation::SurfaceExtend {
            target,
            distance_bits,
        } => positive(*distance_bits) && is_surface_node(*target),
        ExactBRepOperation::SurfaceKnit {
            surfaces,
            tolerance_bits,
            ..
        } => {
            let tolerance = f64::from_bits(*tolerance_bits);
            (2..=256).contains(&surfaces.len())
                && surfaces.windows(2).all(|pair| pair[0] < pair[1])
                && surfaces.iter().copied().all(is_surface_node)
                && tolerance.is_finite()
                && (tolerance_mm..=10.0).contains(&tolerance)
        }
        ExactBRepOperation::SurfaceThicken {
            target,
            thickness_bits,
            ..
        } => positive(*thickness_bits) && is_surface_node(*target),
        ExactBRepOperation::WeldmentJoint {
            first,
            second,
            joint_point_bits,
            first_direction_bits,
            second_direction_bits,
            ..
        } => {
            let point = joint_point_bits.map(f64::from_bits);
            let first_direction = first_direction_bits.map(f64::from_bits);
            let second_direction = second_direction_bits.map(f64::from_bits);
            let length_squared = |direction: [f64; 3]| {
                direction[0] * direction[0]
                    + direction[1] * direction[1]
                    + direction[2] * direction[2]
            };
            let direction_dot = first_direction[0] * second_direction[0]
                + first_direction[1] * second_direction[1]
                + first_direction[2] * second_direction[2];
            first != second
                && point
                    .into_iter()
                    .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
                && first_direction
                    .into_iter()
                    .chain(second_direction)
                    .all(|value| value.is_finite() && value.abs() <= 1.0)
                && (length_squared(first_direction) - 1.0).abs() <= ROUNDING
                && (length_squared(second_direction) - 1.0).abs() <= ROUNDING
                && direction_dot.is_finite()
                && direction_dot.abs() <= 0.996_194_698_091_745_5
        }
        ExactBRepOperation::RigidTransform { matrix_bits, .. } => {
            let matrix = matrix_bits.map(f64::from_bits);
            Transform::from_matrix(matrix)
                .ok()
                .and_then(Transform::rigid_inverse)
                .is_some()
                && [matrix[3], matrix[7], matrix[11]]
                    .into_iter()
                    .all(|value| value.abs() <= MAX_ABS_MM)
        }
        ExactBRepOperation::Shell {
            target,
            removed_faces,
            thickness_bits,
            direction,
            profile_faces,
            name,
        } if !profile_faces.is_empty() => {
            positive(*thickness_bits)
                && removed_faces.is_empty()
                && *direction == ExactBRepShellDirection::Inward
                && profile_faces.len() <= MAX_EXACT_BREP_TOPOLOGY_SELECTORS
                && profile_faces.iter().all(|face| match face {
                    ExactBRepProfileFaceReference::Start | ExactBRepProfileFaceReference::End => {
                        true
                    }
                    ExactBRepProfileFaceReference::Segment {
                        entity_id,
                        source_name,
                    } => *entity_id != 0 && !source_name.is_empty(),
                    ExactBRepProfileFaceReference::NamedResult { name } => !name.is_empty(),
                })
                && name.as_ref().is_some_and(|name| !name.is_empty())
                && prior_nodes.get(target.0 as usize).is_some()
        }
        ExactBRepOperation::Shell {
            target,
            removed_faces,
            thickness_bits,
            ..
        } => {
            positive(*thickness_bits)
                && ((removed_faces.is_empty() && prior_nodes.get(target.0 as usize).is_some())
                    || valid_topology_selectors(
                        removed_faces,
                        ExactBRepTopologyKind::Face,
                        document_id,
                        definition_id,
                        *target,
                        prior_nodes,
                    ))
        }
        ExactBRepOperation::EdgeFinish {
            target,
            edges,
            profile_edges,
            kind,
            amount_bits,
            fillet_radius_stations,
            chamfer_mode,
            chamfer_edge_sides,
        } => {
            let mut previous_position = 0.0;
            let valid_stations = fillet_radius_stations.is_empty()
                || (*kind == ExactBRepEdgeFinishKind::Fillet
                    && fillet_radius_stations.len() <= 32
                    && fillet_radius_stations
                        .last()
                        .is_some_and(|station| f64::from_bits(station.position_bits) == 1.0)
                    && fillet_radius_stations.iter().all(|station| {
                        let position = f64::from_bits(station.position_bits);
                        let valid = position.is_finite()
                            && position > previous_position
                            && position <= 1.0
                            && positive(station.radius_bits);
                        previous_position = position;
                        valid
                    }));
            let valid_chamfer_mode = match chamfer_mode {
                ExactBRepChamferMode::Symmetric => chamfer_edge_sides.is_empty(),
                ExactBRepChamferMode::TwoDistance {
                    second_distance_bits,
                } => {
                    *kind == ExactBRepEdgeFinishKind::Chamfer
                        && positive(*second_distance_bits)
                        && !chamfer_edge_sides.is_empty()
                }
                ExactBRepChamferMode::DistanceAngle { angle_degrees_bits } => {
                    let angle = f64::from_bits(*angle_degrees_bits);
                    *kind == ExactBRepEdgeFinishKind::Chamfer
                        && angle.is_finite()
                        && angle > 0.1
                        && angle < 89.9
                        && !chamfer_edge_sides.is_empty()
                }
            };
            let valid_edge_sides = chamfer_edge_sides.len() <= MAX_EXACT_BREP_TOPOLOGY_SELECTORS
                && chamfer_edge_sides.iter().all(|selection| {
                    valid_topology_selectors(
                        std::slice::from_ref(&selection.edge),
                        ExactBRepTopologyKind::Edge,
                        document_id,
                        definition_id,
                        *target,
                        prior_nodes,
                    ) && valid_topology_selectors(
                        std::slice::from_ref(&selection.side_face),
                        ExactBRepTopologyKind::Face,
                        document_id,
                        definition_id,
                        *target,
                        prior_nodes,
                    )
                });
            let paired_edges_match = chamfer_edge_sides.is_empty()
                || (edges.len() == chamfer_edge_sides.len()
                    && edges
                        .iter()
                        .zip(chamfer_edge_sides)
                        .all(|(edge, selection)| edge == &selection.edge));
            let valid_profile_edges = profile_edges.len() <= MAX_EXACT_BREP_TOPOLOGY_SELECTORS
                && profile_edges.iter().all(|edge| {
                    edge.first != edge.second
                        && [&edge.first, &edge.second]
                            .into_iter()
                            .all(|face| match face {
                                ExactBRepProfileFaceReference::Start
                                | ExactBRepProfileFaceReference::End => true,
                                ExactBRepProfileFaceReference::Segment {
                                    entity_id,
                                    source_name,
                                } => *entity_id != 0 && !source_name.is_empty(),
                                ExactBRepProfileFaceReference::NamedResult { name } => {
                                    !name.is_empty()
                                }
                            })
                });
            positive(*amount_bits)
                && valid_stations
                && (*kind == ExactBRepEdgeFinishKind::Fillet || fillet_radius_stations.is_empty())
                && (*kind == ExactBRepEdgeFinishKind::Chamfer
                    || matches!(chamfer_mode, ExactBRepChamferMode::Symmetric))
                && valid_chamfer_mode
                && valid_edge_sides
                && paired_edges_match
                && (edges.is_empty() != profile_edges.is_empty())
                && valid_profile_edges
                && if profile_edges.is_empty() {
                    valid_topology_selectors(
                        edges,
                        ExactBRepTopologyKind::Edge,
                        document_id,
                        definition_id,
                        *target,
                        prior_nodes,
                    )
                } else {
                    fillet_radius_stations.is_empty()
                        && matches!(chamfer_mode, ExactBRepChamferMode::Symmetric)
                        && chamfer_edge_sides.is_empty()
                        && prior_nodes.get(target.0 as usize).is_some()
                }
        }
        ExactBRepOperation::FaceOffset {
            target,
            face,
            profile_face,
            distance_bits,
        } => {
            let valid_profile_face = profile_face.as_ref().is_some_and(|face| match face {
                ExactBRepProfileFaceReference::Start | ExactBRepProfileFaceReference::End => true,
                ExactBRepProfileFaceReference::Segment {
                    entity_id,
                    source_name,
                } => *entity_id != 0 && !source_name.is_empty(),
                ExactBRepProfileFaceReference::NamedResult { name } => !name.is_empty(),
            });
            signed(*distance_bits)
                && (face.is_some() != profile_face.is_some())
                && if let Some(face) = face {
                    valid_topology_selectors(
                        std::slice::from_ref(face),
                        ExactBRepTopologyKind::Face,
                        document_id,
                        definition_id,
                        *target,
                        prior_nodes,
                    )
                } else {
                    valid_profile_face && prior_nodes.get(target.0 as usize).is_some()
                }
        }
        ExactBRepOperation::Revolve {
            axis_start_bits,
            axis_end_bits,
            angle_degrees_bits,
            ..
        } => {
            let start = axis_start_bits.map(f64::from_bits);
            let end = axis_end_bits.map(f64::from_bits);
            let angle = f64::from_bits(*angle_degrees_bits);
            start
                .into_iter()
                .chain(end)
                .all(|value| value.is_finite() && value.abs() <= MAX_ABS_MM)
                && start != end
                && angle.is_finite()
                && angle > 0.0
                && angle <= 360.0
        }
        ExactBRepOperation::PlanarOffset { distance_bits, .. } => {
            planar_offset_distance(f64::from_bits(*distance_bits)).is_ok()
        }
        ExactBRepOperation::PlanarSurface { .. } => true,
        ExactBRepOperation::Sweep { profile, path } => profile != path,
        ExactBRepOperation::SpatialSweep { path, .. } => valid_spatial_path(path, tolerance_mm),
        ExactBRepOperation::Loft {
            sections,
            guide,
            continuity,
        }
        | ExactBRepOperation::LoftSurface {
            sections,
            guide,
            continuity,
        } => {
            guide
                .as_ref()
                .is_none_or(|path| valid_spatial_path(path, tolerance_mm))
                && !(guide.is_some() && *continuity == ExactBRepLoftContinuity::Curvature)
                && (2..=MAX_EXACT_BREP_LOFT_SECTIONS).contains(&sections.len())
                && sections.windows(2).all(|pair| {
                    let lower = f64::from_bits(pair[0].elevation_bits);
                    let upper = f64::from_bits(pair[1].elevation_bits);
                    lower.is_finite()
                        && upper.is_finite()
                        && lower.abs() <= MAX_ABS_MM
                        && upper.abs() <= MAX_ABS_MM
                        && lower < upper
                        && pair[0].profile != pair[1].profile
                })
        }
        ExactBRepOperation::SheetMetal {
            width_bits,
            depth_bits,
            thickness_bits,
            k_factor_bits,
            flanges,
        } => {
            let width = f64::from_bits(*width_bits);
            let depth = f64::from_bits(*depth_bits);
            let thickness = f64::from_bits(*thickness_bits);
            let k_factor = f64::from_bits(*k_factor_bits);
            let edge_rank = |edge| match edge {
                ExactBRepSheetMetalEdge::MinX => 0,
                ExactBRepSheetMetalEdge::MaxX => 1,
                ExactBRepSheetMetalEdge::MinY => 2,
                ExactBRepSheetMetalEdge::MaxY => 3,
            };
            positive(*width_bits)
                && positive(*depth_bits)
                && positive(*thickness_bits)
                && thickness < width.min(depth)
                && k_factor.is_finite()
                && (0.0..=1.0).contains(&k_factor)
                && flanges.len() <= 4
                && flanges
                    .windows(2)
                    .all(|pair| edge_rank(pair[0].edge) < edge_rank(pair[1].edge))
                && flanges.iter().all(|flange| {
                    let angle = f64::from_bits(flange.angle_degrees_bits);
                    positive(flange.length_bits)
                        && positive(flange.inner_radius_bits)
                        && angle.is_finite()
                        && (0.1..=179.9).contains(&angle.abs())
                        && f64::from_bits(flange.inner_radius_bits) + thickness <= MAX_ABS_MM
                })
        }
        ExactBRepOperation::ImportedExact {
            source_sha256,
            source_byte_len,
            result_fingerprint,
        } => {
            source_sha256.iter().any(|byte| *byte != 0)
                && (1..=32 * 1024 * 1024).contains(source_byte_len)
                && !result_fingerprint.is_empty()
                && result_fingerprint.len() <= 128
        }
    }
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
