use super::*;

pub(super) fn region_signed_area(edges: &[SolvedSketchRegionEdge]) -> Result<f64, SketchError> {
    edges
        .iter()
        .map(|edge| match edge {
            SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                Ok(0.5 * (start_mm[0] * end_mm[1] - end_mm[0] * start_mm[1]))
            }
            SolvedSketchRegionEdge::Arc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let radius = distance2(*start_mm, *center_mm);
                let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
                let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
                let mut sweep = end_angle - start_angle;
                if *clockwise {
                    if sweep >= 0.0 {
                        sweep -= std::f64::consts::TAU;
                    }
                } else if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                Ok(0.5
                    * (radius * center_mm[0] * (end_angle.sin() - start_angle.sin())
                        - radius * center_mm[1] * (end_angle.cos() - start_angle.cos())
                        + radius * radius * sweep))
            }
            SolvedSketchRegionEdge::CubicBezier {
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => Ok(
                flatten_cubic([*start_mm, *control_1_mm, *control_2_mm, *end_mm])?
                    .windows(2)
                    .map(|pair| 0.5 * (pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1]))
                    .sum(),
            ),
        })
        .sum()
}

pub(super) fn profile_curves(
    profile: &SolvedSketchRegionProfile,
) -> Result<Vec<RegionCurve>, SketchError> {
    let mut curves = Vec::new();
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => curves.extend(
            points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .take(points.len())
                .map(|(start, end)| RegionCurve::Line {
                    start: *start,
                    end: *end,
                }),
        ),
        SolvedSketchRegionProfile::Boundary(edges) => {
            for edge in edges {
                match edge {
                    SolvedSketchRegionEdge::Line { start_mm, end_mm } => {
                        curves.push(RegionCurve::Line {
                            start: *start_mm,
                            end: *end_mm,
                        })
                    }
                    SolvedSketchRegionEdge::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => curves.push(RegionCurve::Arc {
                        start: *start_mm,
                        end: *end_mm,
                        center: *center_mm,
                        clockwise: *clockwise,
                    }),
                    SolvedSketchRegionEdge::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                    } => curves.extend(
                        flatten_cubic([*start_mm, *control_1_mm, *control_2_mm, *end_mm])?
                            .windows(2)
                            .map(|pair| RegionCurve::Line {
                                start: pair[0],
                                end: pair[1],
                            }),
                    ),
                }
            }
        }
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => {
            let right = [center_mm[0] + radius_mm, center_mm[1]];
            let left = [center_mm[0] - radius_mm, center_mm[1]];
            curves.extend([
                RegionCurve::Arc {
                    start: right,
                    end: left,
                    center: *center_mm,
                    clockwise: false,
                },
                RegionCurve::Arc {
                    start: left,
                    end: right,
                    center: *center_mm,
                    clockwise: false,
                },
            ]);
        }
    }
    Ok(curves)
}

pub(super) fn profile_point(profile: &SolvedSketchRegionProfile) -> [f64; 2] {
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => points[0],
        SolvedSketchRegionProfile::Boundary(edges) => edges[0].start_mm(),
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => [center_mm[0] + radius_mm, center_mm[1]],
    }
}

pub(super) fn profiles_intersect(
    left: &SolvedSketchRegionProfile,
    right: &SolvedSketchRegionProfile,
) -> Result<bool, SketchError> {
    let left = profile_curves(left)?;
    let right = profile_curves(right)?;
    Ok(left
        .iter()
        .any(|left| right.iter().any(|right| curves_intersect(*left, *right))))
}

pub(super) fn curves_intersect(left: RegionCurve, right: RegionCurve) -> bool {
    match (left, right) {
        (
            RegionCurve::Line {
                start: left_start,
                end: left_end,
            },
            RegionCurve::Line {
                start: right_start,
                end: right_end,
            },
        ) => line_segments_intersect(left_start, left_end, right_start, right_end),
        (RegionCurve::Line { start, end }, arc @ RegionCurve::Arc { .. })
        | (arc @ RegionCurve::Arc { .. }, RegionCurve::Line { start, end }) => {
            line_arc_intersects(start, end, arc)
        }
        (left @ RegionCurve::Arc { .. }, right @ RegionCurve::Arc { .. }) => {
            arcs_intersect(left, right)
        }
    }
}

pub(super) fn cross2(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

pub(super) fn subtract2(left: [f64; 2], right: [f64; 2]) -> [f64; 2] {
    [left[0] - right[0], left[1] - right[1]]
}

pub(super) fn line_segments_intersect(
    left_start: [f64; 2],
    left_end: [f64; 2],
    right_start: [f64; 2],
    right_end: [f64; 2],
) -> bool {
    let left = subtract2(left_end, left_start);
    let right = subtract2(right_end, right_start);
    let offset = subtract2(right_start, left_start);
    let denominator = cross2(left, right);
    let near = 3.0 * CUBIC_FLATTEN_TOLERANCE_MM;
    if denominator.abs() <= EPSILON_MM {
        if cross2(offset, left).abs() > EPSILON_MM {
            return point_segment_distance(left_start, right_start, right_end) <= near
                || point_segment_distance(left_end, right_start, right_end) <= near
                || point_segment_distance(right_start, left_start, left_end) <= near
                || point_segment_distance(right_end, left_start, left_end) <= near;
        }
        let axis = usize::from(left[1].abs() > left[0].abs());
        let (left_min, left_max) = if left_start[axis] <= left_end[axis] {
            (left_start[axis], left_end[axis])
        } else {
            (left_end[axis], left_start[axis])
        };
        let (right_min, right_max) = if right_start[axis] <= right_end[axis] {
            (right_start[axis], right_end[axis])
        } else {
            (right_end[axis], right_start[axis])
        };
        return left_min <= right_max + EPSILON_MM && right_min <= left_max + EPSILON_MM;
    }
    let along_left = cross2(offset, right) / denominator;
    let along_right = cross2(offset, left) / denominator;
    if (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&along_left)
        && (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&along_right)
    {
        return true;
    }
    point_segment_distance(left_start, right_start, right_end) <= near
        || point_segment_distance(left_end, right_start, right_end) <= near
        || point_segment_distance(right_start, left_start, left_end) <= near
        || point_segment_distance(right_end, left_start, left_end) <= near
}

pub(super) fn point_segment_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let direction = subtract2(end, start);
    let length_squared = direction[0] * direction[0] + direction[1] * direction[1];
    if length_squared <= EPSILON_MM * EPSILON_MM {
        return distance2(point, start);
    }
    let parameter = ((point[0] - start[0]) * direction[0] + (point[1] - start[1]) * direction[1])
        / length_squared;
    let parameter = parameter.clamp(0.0, 1.0);
    distance2(
        point,
        [
            start[0] + parameter * direction[0],
            start[1] + parameter * direction[1],
        ],
    )
}

pub(super) fn arc_parts(curve: RegionCurve) -> ([f64; 2], [f64; 2], [f64; 2], bool) {
    let RegionCurve::Arc {
        start,
        end,
        center,
        clockwise,
    } = curve
    else {
        unreachable!()
    };
    (start, end, center, clockwise)
}

pub(super) fn point_on_arc(point: [f64; 2], curve: RegionCurve) -> bool {
    let (start, end, center, clockwise) = arc_parts(curve);
    let radius = distance2(start, center);
    if (distance2(point, center) - radius).abs() > radius.max(1.0) * ACCUMULATED_ROUNDING {
        return false;
    }
    let start_angle = (start[1] - center[1]).atan2(start[0] - center[0]);
    let end_angle = (end[1] - center[1]).atan2(end[0] - center[0]);
    let point_angle = (point[1] - center[1]).atan2(point[0] - center[0]);
    let total = if clockwise {
        (start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    };
    let offset = if clockwise {
        (start_angle - point_angle).rem_euclid(std::f64::consts::TAU)
    } else {
        (point_angle - start_angle).rem_euclid(std::f64::consts::TAU)
    };
    offset <= total + ROUNDING
}

pub(super) fn line_arc_intersects(start: [f64; 2], end: [f64; 2], arc: RegionCurve) -> bool {
    let (_, _, center, _) = arc_parts(arc);
    let direction = subtract2(end, start);
    let offset = subtract2(start, center);
    let radius = distance2(arc_parts(arc).0, center);
    let a = direction[0] * direction[0] + direction[1] * direction[1];
    let b = 2.0 * (offset[0] * direction[0] + offset[1] * direction[1]);
    let c = offset[0] * offset[0] + offset[1] * offset[1] - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < -EPSILON_MM {
        return false;
    }
    let root = discriminant.max(0.0).sqrt();
    [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
        .into_iter()
        .any(|parameter| {
            (-EPSILON_MM..=1.0 + EPSILON_MM).contains(&parameter)
                && point_on_arc(
                    [
                        start[0] + direction[0] * parameter,
                        start[1] + direction[1] * parameter,
                    ],
                    arc,
                )
        })
}

pub(super) fn arcs_intersect(left: RegionCurve, right: RegionCurve) -> bool {
    let (left_start, _, left_center, _) = arc_parts(left);
    let (right_start, _, right_center, _) = arc_parts(right);
    let left_radius = distance2(left_start, left_center);
    let right_radius = distance2(right_start, right_center);
    let center_distance = distance2(left_center, right_center);
    let tolerance = left_radius.max(right_radius).max(1.0) * ACCUMULATED_ROUNDING;
    if center_distance <= tolerance && (left_radius - right_radius).abs() <= tolerance {
        return true;
    }
    if center_distance > left_radius + right_radius + tolerance
        || center_distance < (left_radius - right_radius).abs() - tolerance
        || center_distance <= tolerance
    {
        return false;
    }
    let along = (left_radius * left_radius - right_radius * right_radius
        + center_distance * center_distance)
        / (2.0 * center_distance);
    let height_squared = left_radius * left_radius - along * along;
    if height_squared < -tolerance {
        return false;
    }
    let direction = [
        (right_center[0] - left_center[0]) / center_distance,
        (right_center[1] - left_center[1]) / center_distance,
    ];
    let base = [
        left_center[0] + along * direction[0],
        left_center[1] + along * direction[1],
    ];
    let height = height_squared.max(0.0).sqrt();
    let perpendicular = [-direction[1], direction[0]];
    [
        [
            base[0] + height * perpendicular[0],
            base[1] + height * perpendicular[1],
        ],
        [
            base[0] - height * perpendicular[0],
            base[1] - height * perpendicular[1],
        ],
    ]
    .into_iter()
    .any(|point| point_on_arc(point, left) && point_on_arc(point, right))
}

pub(super) fn point_in_profile(
    point: [f64; 2],
    profile: &SolvedSketchRegionProfile,
) -> Result<bool, SketchError> {
    let mut winding = 0_i32;
    for curve in profile_curves(profile)? {
        match curve {
            RegionCurve::Line { start, end } => {
                if start[1] <= point[1]
                    && point[1] < end[1]
                    && cross2(subtract2(end, start), subtract2(point, start)) > 0.0
                {
                    winding += 1;
                } else if end[1] <= point[1]
                    && point[1] < start[1]
                    && cross2(subtract2(end, start), subtract2(point, start)) < 0.0
                {
                    winding -= 1;
                }
            }
            arc @ RegionCurve::Arc {
                center, clockwise, ..
            } => {
                let radius = distance2(arc_parts(arc).0, center);
                let relative_y = (point[1] - center[1]) / radius;
                if relative_y.abs() > 1.0 {
                    continue;
                }
                let angle = relative_y.clamp(-1.0, 1.0).asin();
                for candidate in [angle, std::f64::consts::PI - angle] {
                    let crossing = [
                        center[0] + radius * candidate.cos(),
                        center[1] + radius * candidate.sin(),
                    ];
                    if crossing[0] <= point[0] + EPSILON_MM || !point_on_arc(crossing, arc) {
                        continue;
                    }
                    let derivative = candidate.cos() * if clockwise { -1.0 } else { 1.0 };
                    if derivative > EPSILON_MM {
                        winding += 1;
                    } else if derivative < -EPSILON_MM {
                        winding -= 1;
                    }
                }
            }
        }
    }
    Ok(winding != 0)
}

pub(super) fn cubic_control_polygon_length(points: [[f64; 2]; 4]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance2(pair[0], pair[1]))
        .sum()
}

pub(super) fn midpoint2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

pub(super) fn point_line_distance(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let chord = subtract2(end, start);
    let length = chord[0].hypot(chord[1]);
    if length <= EPSILON_MM {
        distance2(point, start)
    } else {
        cross2(chord, subtract2(point, start)).abs() / length
    }
}

pub(super) fn flatten_cubic(points: [[f64; 2]; 4]) -> Result<Vec<[f64; 2]>, SketchError> {
    let tolerance = CUBIC_FLATTEN_TOLERANCE_MM;
    let mut output = vec![points[0]];
    let mut stack = vec![(points, 0_u8)];
    while let Some((curve, depth)) = stack.pop() {
        let flatness = point_line_distance(curve[1], curve[0], curve[3])
            .max(point_line_distance(curve[2], curve[0], curve[3]));
        let excess_length = cubic_control_polygon_length(curve) - distance2(curve[0], curve[3]);
        if flatness <= tolerance && excess_length <= tolerance {
            output.push(curve[3]);
            if output.len() > MAX_CUBIC_FLATTEN_SEGMENTS + 1 {
                return Err(SketchError::ResourceLimit);
            }
            continue;
        }
        if depth >= MAX_CUBIC_FLATTEN_DEPTH
            || stack.len() + output.len() >= MAX_CUBIC_FLATTEN_SEGMENTS
        {
            return Err(SketchError::ResourceLimit);
        }
        let p01 = midpoint2(curve[0], curve[1]);
        let p12 = midpoint2(curve[1], curve[2]);
        let p23 = midpoint2(curve[2], curve[3]);
        let p012 = midpoint2(p01, p12);
        let p123 = midpoint2(p12, p23);
        let middle = midpoint2(p012, p123);
        stack.push(([middle, p123, p23, curve[3]], depth + 1));
        stack.push(([curve[0], p01, p012, middle], depth + 1));
    }
    if output
        .windows(2)
        .any(|pair| distance2(pair[0], pair[1]) <= EPSILON_MM)
    {
        return Err(SketchError::InvalidRegionIdentity);
    }
    Ok(output)
}

pub(super) fn adjacent_curves_overlap(previous: RegionCurve, next: RegionCurve) -> bool {
    let (
        RegionCurve::Line {
            start: previous_start,
            end: joint,
        },
        RegionCurve::Line {
            start: next_start,
            end: next_end,
        },
    ) = (previous, next)
    else {
        return false;
    };
    if distance2(joint, next_start) > EPSILON_MM {
        return true;
    }
    let incoming = subtract2(previous_start, joint);
    let outgoing = subtract2(next_end, joint);
    let scale = distance2(previous_start, joint)
        .max(distance2(next_end, joint))
        .max(1.0);
    cross2(incoming, outgoing).abs() <= EPSILON_MM * scale
        && incoming[0] * outgoing[0] + incoming[1] * outgoing[1] > 0.0
}

pub(super) fn validate_profile_topology(
    profile: &SolvedSketchRegionProfile,
) -> Result<(), SketchError> {
    let curves = profile_curves(profile)?;
    let count = curves.len();
    if count < 2 {
        return Err(SketchError::OpenRegion);
    }
    for index in 0..count {
        if (count > 2 || index == 0)
            && adjacent_curves_overlap(curves[index], curves[(index + 1) % count])
        {
            return Err(SketchError::InvalidRegionIdentity);
        }
    }
    // A flattened curve has thousands of pieces: only pieces whose boxes
    // overlap can cross, so sweep them sorted by their left edge.
    let bounds: Vec<_> = curves
        .iter()
        .map(|curve| region_curve_bounds(*curve))
        .collect();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|a, b| bounds[*a][0][0].total_cmp(&bounds[*b][0][0]));
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right][0][0] > bounds[left][1][0] + EPSILON_MM {
                break;
            }
            let adjacent = left.abs_diff(right) == 1 || left.abs_diff(right) == count - 1;
            if !adjacent
                && bounds[right][0][1] <= bounds[left][1][1] + EPSILON_MM
                && bounds[left][0][1] <= bounds[right][1][1] + EPSILON_MM
                && curves_intersect(curves[left], curves[right])
            {
                return Err(SketchError::InvalidRegionIdentity);
            }
        }
    }
    Ok(())
}

/// A box around a region curve (a whole circle's for an arc).
pub(super) fn region_curve_bounds(curve: RegionCurve) -> [[f64; 2]; 2] {
    match curve {
        RegionCurve::Line { start, end } => [
            [start[0].min(end[0]), start[1].min(end[1])],
            [start[0].max(end[0]), start[1].max(end[1])],
        ],
        RegionCurve::Arc { start, center, .. } => {
            let radius = distance2(start, center);
            [
                [center[0] - radius, center[1] - radius],
                [center[0] + radius, center[1] + radius],
            ]
        }
    }
}

pub(super) fn stable_region_id(entity_ids: &[SketchEntityId]) -> SketchRegionId {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for id in entity_ids {
        for byte in id.0.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    SketchRegionId(hash.max(1))
}
