use super::*;

pub(super) fn feature_supports_parameter_target(
    kind: &FeatureKind,
    target: &FeatureParameterTarget,
) -> bool {
    kind.parameter_descriptors().iter().any(|descriptor| {
        descriptor.path() == &target.path && descriptor.value_type() == target.value_type
    })
}

pub(super) fn audit_feature_parameter_binding(
    product: &ProductModel,
    binding: &FeatureParameterBinding,
    identity: &EvaluationIdentity,
    report: &EvaluationReport,
) -> FeatureParameterFreshness {
    let Some(provenance) = product.feature_parameter_provenance.get(&binding.target) else {
        return FeatureParameterFreshness::Stale(FeatureParameterStaleReason::NeverComputed);
    };
    let reason = if provenance.identity.evaluator != identity.evaluator {
        Some(FeatureParameterStaleReason::EvaluatorChanged)
    } else if provenance.identity.schema != identity.schema {
        Some(FeatureParameterStaleReason::SchemaChanged)
    } else if provenance.identity.tolerance != identity.tolerance {
        Some(FeatureParameterStaleReason::ToleranceChanged)
    } else if provenance.identity.backend != identity.backend {
        Some(FeatureParameterStaleReason::BackendChanged)
    } else if let Some(output) = report.outputs.get(&binding.derived_from) {
        if provenance.input_digest != output.input_digest {
            Some(FeatureParameterStaleReason::InputChanged)
        } else if provenance.result_digest != output.result_digest {
            Some(FeatureParameterStaleReason::ResultChanged)
        } else if feature_parameter_value_bits(product, &binding.target)
            .is_none_or(|value_bits| value_bits != provenance.applied_value_bits)
        {
            Some(FeatureParameterStaleReason::AppliedValueChanged)
        } else {
            None
        }
    } else {
        Some(FeatureParameterStaleReason::EvaluationFailed)
    };
    reason.map_or(
        FeatureParameterFreshness::Current,
        FeatureParameterFreshness::Stale,
    )
}

pub(super) fn feature_parameter_dimension(
    product: &ProductModel,
    target: &FeatureParameterTarget,
) -> Option<Dimension> {
    let feature = product.features.get(&target.feature_id)?;
    if !feature_supports_parameter_target(&feature.kind, target) {
        return None;
    }
    let value = feature_kind_parameter_value(&feature.kind, target.path.as_str())?;
    Dimension::new(value.to_string(), value).ok()
}

pub(super) fn feature_parameter_value_bits(
    product: &ProductModel,
    target: &FeatureParameterTarget,
) -> Option<u64> {
    feature_parameter_dimension(product, target).map(|value| value.millimetres().to_bits())
}

pub(super) fn feature_kind_parameter_value(kind: &FeatureKind, path: &str) -> Option<f64> {
    let parts = path.split('.').collect::<Vec<_>>();
    match kind {
        FeatureKind::Workplane(spec) => match (&spec.support, parts.as_slice()) {
            (WorkplaneSupport::Free, ["frame", "origin", axis]) => {
                point_coordinate_3d(spec.frame.origin_mm, axis)
            }
            (WorkplaneSupport::Offset { distance, .. }, ["support", "offset", "distance"]) => {
                Some(distance.millimetres())
            }
            _ => None,
        },
        FeatureKind::Sketch(spec) => sketch_parameter_value(spec, &parts),
        FeatureKind::Profile { segments, .. } => match parts.as_slice() {
            ["bounds", dimension] => {
                let points = kind
                    .polygon_points()
                    .filter(|points| is_axis_aligned_rectangle(points))?;
                match *dimension {
                    "width" => Some(points[1][0] - points[0][0]),
                    "height" => Some(points[3][1] - points[0][1]),
                    _ => None,
                }
            }
            _ => segment_parameter_value(segments, &parts),
        },
        FeatureKind::Pad(spec) => {
            extent_parameter_value(&spec.extent, path.strip_prefix("extent.")?)
        }
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => match parts.as_slice() {
            ["axis_start", axis] => point_coordinate(*axis_start_mm, axis),
            ["axis_end", axis] => point_coordinate(*axis_end_mm, axis),
            ["angle"] => Some(*angle_degrees),
            _ => None,
        },
        FeatureKind::Shell { thickness, .. } | FeatureKind::SurfaceThicken { thickness, .. }
            if path == "thickness" =>
        {
            Some(thickness.millimetres())
        }
        FeatureKind::EdgeFinish {
            amount,
            fillet_radius_stations,
            chamfer_mode,
            ..
        } => match parts.as_slice() {
            ["amount"] => Some(amount.millimetres()),
            ["chamfer_second_distance"] => match chamfer_mode {
                ChamferMode::TwoDistance { second_distance } => Some(second_distance.millimetres()),
                _ => None,
            },
            ["chamfer_angle"] => match chamfer_mode {
                ChamferMode::DistanceAngle { angle_degrees } => Some(*angle_degrees),
                _ => None,
            },
            ["fillet_radius_stations", index, "radius"] => Some(
                fillet_radius_stations
                    .get(index.parse::<usize>().ok()?)?
                    .radius
                    .millimetres(),
            ),
            _ => None,
        },
        FeatureKind::FaceOffset { distance, .. } | FeatureKind::PlanarOffset { distance, .. }
            if path == "distance" =>
        {
            Some(distance.millimetres())
        }
        FeatureKind::WeldmentMember(spec) if path == "orientation" => {
            Some(spec.orientation_degrees)
        }
        FeatureKind::SheetMetal(spec) => match parts.as_slice() {
            ["width"] => Some(spec.width.millimetres()),
            ["depth"] => Some(spec.depth.millimetres()),
            ["thickness"] => Some(spec.thickness.millimetres()),
            ["k_factor"] => Some(spec.k_factor),
            ["flanges", index, "length"] => Some(
                spec.flanges
                    .get(index.parse::<usize>().ok()?)?
                    .length
                    .millimetres(),
            ),
            ["flanges", index, "angle"] => Some(
                spec.flanges
                    .get(index.parse::<usize>().ok()?)?
                    .angle_degrees,
            ),
            ["flanges", index, "inner_radius"] => Some(
                spec.flanges
                    .get(index.parse::<usize>().ok()?)?
                    .inner_radius
                    .millimetres(),
            ),
            _ => None,
        },
        FeatureKind::Loft { sections, .. } => match parts.as_slice() {
            ["sections", index, "elevation"] => {
                Some(sections.get(index.parse::<usize>().ok()?)?.elevation_mm)
            }
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn sketch_parameter_value(spec: &SketchSpec, parts: &[&str]) -> Option<f64> {
    match parts {
        ["bounds", size @ ("width" | "height")] => {
            let [minimum, maximum] = spec.rectangle_bounds()?;
            let axis = usize::from(*size == "height");
            Some(maximum[axis] - minimum[axis])
        }
        ["entities", id, point, axis] => {
            let id = id.parse::<u64>().ok()?;
            let entity = spec.entities.iter().find(|entity| entity.id().0 == id)?;
            match (entity, *point) {
                (
                    SketchEntity::Line { start_mm, .. }
                    | SketchEntity::Arc { start_mm, .. }
                    | SketchEntity::CubicBezier { start_mm, .. },
                    "start",
                ) => point_coordinate(*start_mm, axis),
                (
                    SketchEntity::Line { end_mm, .. }
                    | SketchEntity::Arc { end_mm, .. }
                    | SketchEntity::CubicBezier { end_mm, .. },
                    "end",
                ) => point_coordinate(*end_mm, axis),
                (SketchEntity::CubicBezier { control_1_mm, .. }, "control_1") => {
                    point_coordinate(*control_1_mm, axis)
                }
                (SketchEntity::CubicBezier { control_2_mm, .. }, "control_2") => {
                    point_coordinate(*control_2_mm, axis)
                }
                (
                    SketchEntity::Arc { center_mm, .. } | SketchEntity::Circle { center_mm, .. },
                    "center",
                ) => point_coordinate(*center_mm, axis),
                _ => None,
            }
        }
        ["entities", id, "radius"] => {
            let id = id.parse::<u64>().ok()?;
            spec.entities
                .iter()
                .find(|entity| entity.id().0 == id)
                .and_then(|entity| match entity {
                    SketchEntity::Circle { radius_mm, .. } => Some(*radius_mm),
                    _ => None,
                })
        }
        ["constraints", id, "value"] => {
            let id = id.parse::<u64>().ok()?;
            spec.constraints
                .iter()
                .find(|constraint| constraint.id.0 == id)
                .and_then(|constraint| match &constraint.kind {
                    SketchConstraintKind::Distance { value, .. }
                    | SketchConstraintKind::Radius { value, .. } => Some(value.millimetres()),
                    _ => None,
                })
        }
        ["constraints", id, "angle"] => {
            let id = id.parse::<u64>().ok()?;
            spec.constraints
                .iter()
                .find(|constraint| constraint.id.0 == id)
                .and_then(|constraint| match &constraint.kind {
                    SketchConstraintKind::Angle { angle_degrees, .. } => Some(*angle_degrees),
                    _ => None,
                })
        }
        ["constraints", id, "position", axis] => {
            let id = id.parse::<u64>().ok()?;
            spec.constraints
                .iter()
                .find(|constraint| constraint.id.0 == id)
                .and_then(|constraint| match &constraint.kind {
                    SketchConstraintKind::FixedPoint { position_mm, .. } => {
                        point_coordinate(*position_mm, axis)
                    }
                    _ => None,
                })
        }
        _ => None,
    }
}

pub(super) fn segment_parameter_value(segments: &[ProfileSegment], parts: &[&str]) -> Option<f64> {
    let (index, point, axis) = match parts {
        ["segments", index, point, axis] => (index, SegmentPoint::parse(point, None)?, axis),
        ["segments", index, "points", point, axis] => (
            index,
            SegmentPoint::parse("points", Some(point.parse().ok()?))?,
            axis,
        ),
        _ => return None,
    };
    let segment = segments.get(index.parse::<usize>().ok()?)?;
    let coordinate = match (segment, point) {
        (_, SegmentPoint::Start) => segment.start_mm(),
        (_, SegmentPoint::End) => segment.end_mm(),
        (ProfileSegment::CircularArc { center_mm, .. }, SegmentPoint::Center) => *center_mm,
        (ProfileSegment::Spline { points_mm }, SegmentPoint::Inner(point))
            if point > 0 && point + 1 < points_mm.len() =>
        {
            points_mm[point]
        }
        _ => return None,
    };
    point_coordinate(coordinate, axis)
}

/// A point of a profile segment named by a parameter path.
#[derive(Clone, Copy)]
pub(super) enum SegmentPoint {
    Start,
    End,
    Center,
    /// A spline point between its ends (`points.N`).
    Inner(usize),
}

impl SegmentPoint {
    pub(super) fn parse(name: &str, index: Option<usize>) -> Option<Self> {
        match (name, index) {
            ("start", None) => Some(Self::Start),
            ("end", None) => Some(Self::End),
            ("center", None) => Some(Self::Center),
            ("points", Some(index)) => Some(Self::Inner(index)),
            _ => None,
        }
    }
}

pub(super) fn extent_parameter_value(extent: &FeatureExtent, path: &str) -> Option<f64> {
    match (extent, path) {
        (FeatureExtent::Blind(distance) | FeatureExtent::Symmetric(distance), "distance") => {
            Some(distance.millimetres())
        }
        (FeatureExtent::Bidirectional { along, .. }, "along.distance") => {
            extent_end_parameter_value(along)
        }
        (FeatureExtent::Bidirectional { opposite, .. }, "opposite.distance") => {
            extent_end_parameter_value(opposite)
        }
        _ => None,
    }
}

pub(super) fn extent_end_parameter_value(extent: &FeatureExtentEnd) -> Option<f64> {
    match extent {
        FeatureExtentEnd::Blind(distance) => Some(distance.millimetres()),
        FeatureExtentEnd::ThroughAll | FeatureExtentEnd::UpToFace(_) => None,
    }
}

pub(super) fn point_coordinate(point: [f64; 2], axis: &str) -> Option<f64> {
    match axis {
        "x" => Some(point[0]),
        "y" => Some(point[1]),
        _ => None,
    }
}

pub(super) fn point_coordinate_3d(point: [f64; 3], axis: &str) -> Option<f64> {
    match axis {
        "x" => Some(point[0]),
        "y" => Some(point[1]),
        "z" => Some(point[2]),
        _ => None,
    }
}

pub(super) fn recompute_feature_parameters(
    product: &mut ProductModel,
    identity: &EvaluationIdentity,
    affected_nodes: Option<&BTreeSet<NodeId>>,
    previous: Option<&EvaluationReport>,
) -> Result<EvaluationReport, CanonicalError> {
    let all_nodes;
    let affected = if let Some(affected) = affected_nodes {
        affected
    } else {
        all_nodes = product.evaluator_nodes.keys().cloned().collect();
        &all_nodes
    };
    let report = evaluate_affected(&product.evaluator_nodes, identity, previous, affected)
        .map_err(CanonicalError::Graph)?;
    let bindings = product
        .feature_parameter_bindings
        .values()
        .map(|binding| binding.as_ref().clone())
        .collect::<Vec<_>>();
    for binding in bindings {
        if affected_nodes
            .is_some_and(|affected| !affected.contains(&binding.derived_from.root_rule_node_id))
        {
            continue;
        }
        let output =
            report
                .outputs
                .get(&binding.derived_from)
                .ok_or(CanonicalError::FailedEvaluation(
                    binding.derived_from.root_rule_node_id,
                ))?;
        let dimension = Dimension::new(output.value.to_string(), output.value)?;
        set_feature_parameter(product, &binding.target, dimension)?;
        product.feature_parameter_provenance.insert(
            binding.target.clone(),
            Arc::new(FeatureParameterProvenance {
                identity: identity.clone(),
                input_digest: output.input_digest.clone(),
                result_digest: output.result_digest.clone(),
                applied_value_bits: output.value.to_bits(),
            }),
        );
    }
    Ok(report)
}

pub(super) fn set_feature_parameter(
    product: &mut ProductModel,
    target: &FeatureParameterTarget,
    dimension: Dimension,
) -> Result<(), CanonicalError> {
    let feature = product
        .features
        .get(&target.feature_id)
        .ok_or(CanonicalError::FeatureNotFound(target.feature_id))?;
    if !feature_supports_parameter_target(&feature.kind, target) {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    let mut kind = feature.kind.clone();
    if !set_feature_kind_parameter(&mut kind, target, &dimension, product.tolerance.linear_mm())? {
        return Err(CanonicalError::InvalidFeatureParameterBinding(
            target.clone(),
        ));
    }
    product.features.insert(
        target.feature_id,
        Arc::new(Feature {
            id: feature.id,
            definition_id: feature.definition_id,
            name: feature.name.clone(),
            kind,
        }),
    );
    recompute_offset_workplane_frames(product)?;
    Ok(())
}

pub(super) fn recompute_offset_workplane_frames(
    product: &mut ProductModel,
) -> Result<(), CanonicalError> {
    let ordered_feature_ids = product
        .definitions
        .values()
        .flat_map(|definition| definition.feature_ids.iter().copied())
        .collect::<Vec<_>>();
    for feature_id in ordered_feature_ids {
        let Some(feature) = product.features.get(&feature_id) else {
            continue;
        };
        let FeatureKind::Workplane(spec) = &feature.kind else {
            continue;
        };
        let WorkplaneSupport::Offset { base, distance } = &spec.support else {
            continue;
        };
        let base_frame = product
            .features
            .get(base)
            .and_then(|feature| match &feature.kind {
                FeatureKind::Workplane(spec) => Some(spec.frame),
                _ => None,
            })
            .ok_or(CanonicalError::Sketch(
                SketchError::MissingWorkplaneSupport(*base),
            ))?;
        let frame = base_frame.offset(distance.millimetres());
        if spec.frame == frame {
            continue;
        }
        product.features.insert(
            feature_id,
            Arc::new(Feature {
                id: feature.id,
                definition_id: feature.definition_id,
                name: feature.name.clone(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: spec.support.clone(),
                    frame,
                }),
            }),
        );
    }
    Ok(())
}

pub(super) fn set_feature_kind_parameter(
    kind: &mut FeatureKind,
    target: &FeatureParameterTarget,
    dimension: &Dimension,
    tolerance_mm: f64,
) -> Result<bool, CanonicalError> {
    let path = target.path.as_str();
    let parts = path.split('.').collect::<Vec<_>>();
    let value = dimension.millimetres();
    let updated = match kind {
        FeatureKind::Workplane(spec) => match (&mut spec.support, parts.as_slice()) {
            (WorkplaneSupport::Free, ["frame", "origin", axis]) => {
                set_point_coordinate_3d(&mut spec.frame.origin_mm, axis, value)
            }
            (WorkplaneSupport::Offset { distance, .. }, ["support", "offset", "distance"]) => {
                *distance = dimension.clone();
                true
            }
            _ => false,
        },
        FeatureKind::Sketch(spec) => set_sketch_parameter(spec, &parts, dimension),
        FeatureKind::Profile { segments, closed } => match parts.as_slice() {
            ["bounds", "width"] | ["bounds", "height"] => {
                let points = kind.polygon_points().unwrap_or_default();
                *kind = FeatureKind::polygon(&resize_axis_aligned_rectangle(
                    &points,
                    target,
                    value,
                    tolerance_mm,
                )?);
                true
            }
            _ => set_segment_parameter(segments, *closed, &parts, value),
        },
        FeatureKind::Pad(spec) => path
            .strip_prefix("extent.")
            .is_some_and(|path| set_extent_parameter(&mut spec.extent, path, dimension)),
        FeatureKind::Revolve {
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            ..
        } => match parts.as_slice() {
            ["axis_start", axis] => set_point_coordinate(axis_start_mm, axis, value),
            ["axis_end", axis] => set_point_coordinate(axis_end_mm, axis, value),
            ["angle"] => {
                *angle_degrees = value;
                true
            }
            _ => false,
        },
        FeatureKind::Shell { thickness, .. } | FeatureKind::SurfaceThicken { thickness, .. }
            if path == "thickness" =>
        {
            *thickness = dimension.clone();
            true
        }
        FeatureKind::EdgeFinish {
            amount,
            fillet_radius_stations,
            chamfer_mode,
            ..
        } => match parts.as_slice() {
            ["amount"] => {
                *amount = dimension.clone();
                true
            }
            ["chamfer_second_distance"] => {
                if let ChamferMode::TwoDistance { second_distance } = chamfer_mode {
                    *second_distance = dimension.clone();
                    true
                } else {
                    false
                }
            }
            ["chamfer_angle"] => {
                if let ChamferMode::DistanceAngle { angle_degrees } = chamfer_mode {
                    *angle_degrees = value;
                    true
                } else {
                    false
                }
            }
            ["fillet_radius_stations", index, "radius"] => fillet_radius_stations
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|station| {
                    station.radius = dimension.clone();
                    true
                }),
            _ => false,
        },
        FeatureKind::FaceOffset { distance, .. } | FeatureKind::PlanarOffset { distance, .. }
            if path == "distance" =>
        {
            *distance = dimension.clone();
            true
        }
        FeatureKind::SurfaceKnit { tolerance, .. } if path == "tolerance" => {
            *tolerance = dimension.clone();
            true
        }
        FeatureKind::WeldmentMember(spec) if path == "orientation" => {
            spec.orientation_degrees = value;
            true
        }
        FeatureKind::SheetMetal(spec) => match parts.as_slice() {
            ["width"] => {
                spec.width = dimension.clone();
                true
            }
            ["depth"] => {
                spec.depth = dimension.clone();
                true
            }
            ["thickness"] => {
                spec.thickness = dimension.clone();
                true
            }
            ["k_factor"] => {
                spec.k_factor = value;
                true
            }
            ["flanges", index, "length"] => spec
                .flanges
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|flange| {
                    flange.length = dimension.clone();
                    true
                }),
            ["flanges", index, "angle"] => spec
                .flanges
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|flange| {
                    flange.angle_degrees = value;
                    true
                }),
            ["flanges", index, "inner_radius"] => spec
                .flanges
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|flange| {
                    flange.inner_radius = dimension.clone();
                    true
                }),
            _ => false,
        },
        FeatureKind::Loft { sections, .. } => match parts.as_slice() {
            ["sections", index, "elevation"] => sections
                .get_mut(index.parse::<usize>().ok().unwrap_or(usize::MAX))
                .is_some_and(|section| {
                    section.elevation_mm = value;
                    true
                }),
            _ => false,
        },
        _ => false,
    };
    Ok(updated)
}

pub(super) fn set_sketch_parameter(
    spec: &mut SketchSpec,
    parts: &[&str],
    dimension: &Dimension,
) -> bool {
    let value = dimension.millimetres();
    match parts {
        ["bounds", size @ ("width" | "height")] => {
            let Some([minimum, maximum]) = spec.rectangle_bounds() else {
                return false;
            };
            if value <= PROFILE_EPSILON_MM {
                return false;
            }
            let axis = usize::from(*size == "height");
            for entity in &mut spec.entities {
                if let SketchEntity::Line {
                    start_mm, end_mm, ..
                } = entity
                {
                    for point in [start_mm, end_mm] {
                        if point[axis] == maximum[axis] {
                            point[axis] = minimum[axis] + value;
                        }
                    }
                }
            }
            true
        }
        ["entities", id, point, axis] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            let Some(entity) = spec.entities.iter_mut().find(|entity| entity.id().0 == id) else {
                return false;
            };
            match (entity, *point) {
                (
                    SketchEntity::Line { start_mm, .. }
                    | SketchEntity::Arc { start_mm, .. }
                    | SketchEntity::CubicBezier { start_mm, .. },
                    "start",
                ) => set_point_coordinate(start_mm, axis, value),
                (
                    SketchEntity::Line { end_mm, .. }
                    | SketchEntity::Arc { end_mm, .. }
                    | SketchEntity::CubicBezier { end_mm, .. },
                    "end",
                ) => set_point_coordinate(end_mm, axis, value),
                (SketchEntity::CubicBezier { control_1_mm, .. }, "control_1") => {
                    set_point_coordinate(control_1_mm, axis, value)
                }
                (SketchEntity::CubicBezier { control_2_mm, .. }, "control_2") => {
                    set_point_coordinate(control_2_mm, axis, value)
                }
                (
                    SketchEntity::Arc { center_mm, .. } | SketchEntity::Circle { center_mm, .. },
                    "center",
                ) => set_point_coordinate(center_mm, axis, value),
                _ => false,
            }
        }
        ["entities", id, "radius"] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.entities
                .iter_mut()
                .find(|entity| entity.id().0 == id)
                .is_some_and(|entity| match entity {
                    SketchEntity::Circle { radius_mm, .. } => {
                        *radius_mm = value;
                        true
                    }
                    _ => false,
                })
        }
        ["constraints", id, "value"] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.constraints
                .iter_mut()
                .find(|constraint| constraint.id.0 == id)
                .is_some_and(|constraint| match &mut constraint.kind {
                    SketchConstraintKind::Distance { value, .. }
                    | SketchConstraintKind::Radius { value, .. } => {
                        *value = dimension.clone();
                        true
                    }
                    _ => false,
                })
        }
        ["constraints", id, "angle"] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.constraints
                .iter_mut()
                .find(|constraint| constraint.id.0 == id)
                .is_some_and(|constraint| match &mut constraint.kind {
                    SketchConstraintKind::Angle { angle_degrees, .. } => {
                        *angle_degrees = value;
                        true
                    }
                    _ => false,
                })
        }
        ["constraints", id, "position", axis] => {
            let Ok(id) = id.parse::<u64>() else {
                return false;
            };
            spec.constraints
                .iter_mut()
                .find(|constraint| constraint.id.0 == id)
                .is_some_and(|constraint| match &mut constraint.kind {
                    SketchConstraintKind::FixedPoint { position_mm, .. } => {
                        set_point_coordinate(position_mm, axis, value)
                    }
                    _ => false,
                })
        }
        _ => false,
    }
}

/// Sets one coordinate of a segment point. A segment end shared with the neighbouring
/// segment is one vertex: both segments move with it.
pub(super) fn set_segment_parameter(
    segments: &mut [ProfileSegment],
    closed: bool,
    parts: &[&str],
    value: f64,
) -> bool {
    let parsed = match parts {
        ["segments", index, point, axis] => {
            SegmentPoint::parse(point, None).map(|p| (index, p, axis))
        }
        ["segments", index, "points", point, axis] => point
            .parse()
            .ok()
            .and_then(|point| SegmentPoint::parse("points", Some(point)))
            .map(|p| (index, p, axis)),
        _ => None,
    };
    let Some((index, point, axis)) = parsed else {
        return false;
    };
    let Ok(index) = index.parse::<usize>() else {
        return false;
    };
    if index >= segments.len() {
        return false;
    }
    let count = segments.len();
    let previous = (index > 0 || closed).then(|| (index + count - 1) % count);
    let next = (index + 1 < count || closed).then(|| (index + 1) % count);
    let set = |point: Option<&mut [f64; 2]>| {
        point.is_some_and(|point| set_point_coordinate(point, axis, value))
    };
    match point {
        SegmentPoint::Start => {
            let shared = previous
                .filter(|previous| segments[*previous].end_mm() == segments[index].start_mm());
            if !set(segments[index].start_mut()) {
                return false;
            }
            if let Some(previous) = shared {
                set(segments[previous].end_mut());
            }
            true
        }
        SegmentPoint::End => {
            let shared = next.filter(|next| segments[*next].start_mm() == segments[index].end_mm());
            if !set(segments[index].end_mut()) {
                return false;
            }
            if let Some(next) = shared {
                set(segments[next].start_mut());
            }
            true
        }
        SegmentPoint::Center => match &mut segments[index] {
            ProfileSegment::CircularArc { center_mm, .. } => set(Some(center_mm)),
            _ => false,
        },
        SegmentPoint::Inner(point) => match &mut segments[index] {
            ProfileSegment::Spline { points_mm } if point > 0 && point + 1 < points_mm.len() => {
                set(points_mm.get_mut(point))
            }
            _ => false,
        },
    }
}

pub(super) fn set_extent_parameter(
    extent: &mut FeatureExtent,
    path: &str,
    dimension: &Dimension,
) -> bool {
    match (extent, path) {
        (FeatureExtent::Blind(distance) | FeatureExtent::Symmetric(distance), "distance") => {
            *distance = dimension.clone();
            true
        }
        (FeatureExtent::Bidirectional { along, .. }, "along.distance") => {
            set_extent_end_parameter(along, dimension)
        }
        (FeatureExtent::Bidirectional { opposite, .. }, "opposite.distance") => {
            set_extent_end_parameter(opposite, dimension)
        }
        _ => false,
    }
}

pub(super) fn set_extent_end_parameter(
    extent: &mut FeatureExtentEnd,
    dimension: &Dimension,
) -> bool {
    match extent {
        FeatureExtentEnd::Blind(distance) => {
            *distance = dimension.clone();
            true
        }
        FeatureExtentEnd::ThroughAll | FeatureExtentEnd::UpToFace(_) => false,
    }
}

pub(super) fn set_point_coordinate(point: &mut [f64; 2], axis: &str, value: f64) -> bool {
    match axis {
        "x" => point[0] = value,
        "y" => point[1] = value,
        _ => return false,
    }
    true
}

pub(super) fn set_point_coordinate_3d(point: &mut [f64; 3], axis: &str, value: f64) -> bool {
    match axis {
        "x" => point[0] = value,
        "y" => point[1] = value,
        "z" => point[2] = value,
        _ => return false,
    }
    true
}
