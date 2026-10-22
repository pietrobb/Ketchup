//! The BTLx writer's processings: drillings and contours in the BTLx part
//! frame.

use super::*;

pub(super) fn btlx_component_identifiers(
    quantity: usize,
    zero_based_index: usize,
) -> Option<(i32, u32)> {
    let count = i32::try_from(quantity).ok().filter(|count| *count >= 1)?;
    let single_member_number = u32::try_from(zero_based_index.checked_add(1)?).ok()?;
    Some((count, single_member_number))
}

pub(super) fn format_btlx_positive_number(value: f64) -> Option<String> {
    let formatted = format_number(value);
    formatted
        .parse::<f64>()
        .ok()
        .filter(|rounded| rounded.is_finite() && *rounded > 0.0)
        .map(|_| formatted)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum BtlxProcessing {
    Drilling(BtlxDrilling),
    FreeContour(BtlxFreeContour),
    SawContour(BtlxSawContour),
    MillContour(BtlxFreeContour),
}

impl BtlxProcessing {
    pub(super) fn reference_plane(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        match self {
            Self::Drilling(drilling) => (
                drilling.reference_point_mm,
                drilling.x_vector,
                drilling.y_vector,
            ),
            Self::FreeContour(contour) | Self::MillContour(contour) => (
                contour.reference_point_mm,
                contour.x_vector,
                contour.y_vector,
            ),
            Self::SawContour(contour) => (
                contour.reference_point_mm,
                contour.x_vector,
                contour.y_vector,
            ),
        }
    }

    pub(super) fn xml(&self, process_id: u32, reference_plane_id: u32) -> String {
        match self {
            Self::Drilling(drilling) => {
                let (name, thread) = drilling.thread.as_ref().map_or_else(
                    || ("Ketchup circular drilling\"".to_owned(), String::new()),
                    |thread| {
                        (
                            format!(
                                "Ketchup tapped drilling\" Comment=\"{}\"",
                                xml_escape(&thread.label())
                            ),
                            format!(
                                "            <UserAttributes>\n              <UserAttribute Name=\"ThreadSize\" Value=\"{}\"/>\n              <UserAttribute Name=\"ThreadPitch\" Value=\"{}\" Type=\"real\"/>\n              <UserAttribute Name=\"ThreadDepth\" Value=\"{}\" Type=\"real\"/>\n            </UserAttributes>\n",
                                xml_escape(&thread.size),
                                format_number(thread.pitch_mm),
                                format_number(thread.depth_mm)
                            ),
                        )
                    },
                );
                format!(
                    "          <Drilling Name=\"{name} ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\">\n{thread}            <StartX>{}</StartX>\n            <StartY>{}</StartY>\n            <Angle>0</Angle>\n            <Inclination>90</Inclination>\n            <DepthLimited>{}</DepthLimited>\n            <Depth>{}</Depth>\n            <Diameter>{}</Diameter>\n          </Drilling>\n",
                    drilling.start_x,
                    drilling.start_y,
                    if drilling.depth_limited { "yes" } else { "no" },
                    drilling.depth,
                    drilling.diameter
                )
            }
            Self::FreeContour(contour) => {
                let mut xml = format!(
                    "          <FreeContour Name=\"Ketchup profile cut\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n",
                    contour.tool_position,
                    contour.depth,
                    contour.start_point[0],
                    contour.start_point[1]
                );
                for segment in &contour.segments {
                    xml.push_str(&segment.xml());
                }
                xml.push_str("            </Contour>\n          </FreeContour>\n");
                xml
            }
            Self::SawContour(contour) => format!(
                "          <SawContour Name=\"Ketchup groove edge pre-cut\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n              <Line><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/></Line>\n            </Contour>\n          </SawContour>\n",
                contour.tool_position,
                contour.depth,
                contour.start_point[0],
                contour.start_point[1],
                contour.end_point[0],
                contour.end_point[1]
            ),
            Self::MillContour(contour) => {
                let mut xml = format!(
                    "          <MillContour Name=\"Ketchup groove interior milling\" ProcessID=\"{process_id}\" ReferencePlaneID=\"{reference_plane_id}\" ToolPosition=\"{}\">\n            <Contour DepthBounded=\"yes\" Depth=\"{}\" Inclination=\"0\">\n              <StartPoint X=\"{}\" Y=\"{}\" Z=\"0\"/>\n",
                    contour.tool_position,
                    contour.depth,
                    contour.start_point[0],
                    contour.start_point[1]
                );
                for segment in &contour.segments {
                    xml.push_str(&segment.xml());
                }
                xml.push_str("            </Contour>\n          </MillContour>\n");
                xml
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct BtlxDrilling {
    pub(super) reference_point_mm: [f64; 3],
    pub(super) x_vector: [f64; 3],
    pub(super) y_vector: [f64; 3],
    pub(super) start_x: String,
    pub(super) start_y: String,
    /// False when drilled through: `depth` is then where the axis leaves the stock.
    pub(super) depth_limited: bool,
    pub(super) depth: String,
    pub(super) diameter: String,
    /// BTLx has no tapping processing: the thread goes with the drilling as
    /// its user attributes and comment.
    pub(super) thread: Option<HoleThread>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct BtlxSawContour {
    reference_point_mm: [f64; 3],
    x_vector: [f64; 3],
    y_vector: [f64; 3],
    tool_position: &'static str,
    start_point: [String; 2],
    end_point: [String; 2],
    depth: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum BtlxContourSegment {
    Line {
        end_point: [String; 2],
    },
    Arc {
        end_point: [String; 2],
        point_on_arc: [String; 2],
    },
}

impl BtlxContourSegment {
    fn xml(&self) -> String {
        match self {
            Self::Line { end_point } => format!(
                "              <Line><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/></Line>\n",
                end_point[0], end_point[1]
            ),
            Self::Arc {
                end_point,
                point_on_arc,
            } => format!(
                "              <Arc><EndPoint X=\"{}\" Y=\"{}\" Z=\"0\"/><PointOnArc X=\"{}\" Y=\"{}\" Z=\"0\"/></Arc>\n",
                end_point[0], end_point[1], point_on_arc[0], point_on_arc[1]
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct BtlxFreeContour {
    pub(super) reference_point_mm: [f64; 3],
    pub(super) x_vector: [f64; 3],
    pub(super) y_vector: [f64; 3],
    pub(super) tool_position: &'static str,
    pub(super) start_point: [String; 2],
    pub(super) segments: Vec<BtlxContourSegment>,
    pub(super) depth: String,
}

pub(super) fn btlx_processings(
    operation: &GeneralManufacturingOperation,
    options: BtlxExportOptions,
    blank: StockBlank,
    part: &str,
) -> Result<Vec<BtlxProcessing>, GeneralFabricationError> {
    match operation.kind {
        GeneralManufacturingKind::CircularDrill => btlx_drilling(&operation.machining, blank)
            .map(|drilling| {
                vec![BtlxProcessing::Drilling(BtlxDrilling {
                    thread: operation.thread.clone(),
                    ..drilling
                })]
            })
            .map_err(|reason| GeneralFabricationError::BtlxDrillingRefused {
                part: part.to_owned(),
                reason,
            }),
        GeneralManufacturingKind::ThroughCut
        | GeneralManufacturingKind::ProfileCut
        | GeneralManufacturingKind::BooleanCut => {
            let contour = btlx_free_contour(&operation.machining, blank)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
            match options.profile_processing_request {
                BtlxProfileProcessingRequest::PortableFreeContour => {
                    Ok(vec![BtlxProcessing::FreeContour(contour)])
                }
                BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                    intermediate_saw_cuts,
                } => {
                    let saw_contours = btlx_edge_saw_contours(
                        &operation.machining,
                        &contour,
                        intermediate_saw_cuts,
                    )
                    .ok_or_else(|| {
                        GeneralFabricationError::BtlxProfileRequestUnsupported {
                            part: part.to_owned(),
                        }
                    })?;
                    let mut processings = saw_contours
                        .into_iter()
                        .map(BtlxProcessing::SawContour)
                        .collect::<Vec<_>>();
                    processings.push(BtlxProcessing::MillContour(contour));
                    Ok(processings)
                }
            }
        }
        _ => Err(GeneralFabricationError::ExportBlocked),
    }
}

fn btlx_edge_saw_contours(
    geometry: &GeneralMachiningGeometry,
    contour: &BtlxFreeContour,
    intermediate_saw_cuts: u8,
) -> Option<Vec<BtlxSawContour>> {
    if intermediate_saw_cuts > 32 {
        return None;
    }
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = geometry else {
        return None;
    };
    let edges = segments
        .iter()
        .map(|segment| {
            let GeneralMachiningSegment::Line { start_mm, end_mm } = segment else {
                return None;
            };
            Some((*start_mm, *end_mm))
        })
        .collect::<Option<Vec<_>>>()?;
    let [first, second, third, fourth] = edges.as_slice() else {
        return None;
    };
    let vector = |(start, end): &([f64; 2], [f64; 2])| [end[0] - start[0], end[1] - start[1]];
    let dot = ketchup_geometry::linalg::dot2;
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let vectors = [vector(first), vector(second), vector(third), vector(fourth)];
    let close = |left: f64, right: f64| (left - right).abs() <= ROUNDING;
    if !close(dot(vectors[0], vectors[1]), 0.0)
        || !close(dot(vectors[1], vectors[2]), 0.0)
        || !close(cross(vectors[0], vectors[2]), 0.0)
        || !close(cross(vectors[1], vectors[3]), 0.0)
        || !close(dot(vectors[0], vectors[0]), dot(vectors[2], vectors[2]))
        || !close(dot(vectors[1], vectors[1]), dot(vectors[3], vectors[3]))
    {
        return None;
    }
    let [first_boundary, second_boundary] =
        if dot(vectors[0], vectors[0]) >= dot(vectors[1], vectors[1]) {
            [*first, *third]
        } else {
            [*second, *fourth]
        };
    let saw_contour = |(start, end): ([f64; 2], [f64; 2]), tool_position| BtlxSawContour {
        reference_point_mm: contour.reference_point_mm,
        x_vector: contour.x_vector,
        y_vector: contour.y_vector,
        tool_position,
        start_point: start.map(format_number),
        end_point: end.map(format_number),
        depth: contour.depth.clone(),
    };
    let mut result = Vec::with_capacity(usize::from(intermediate_saw_cuts) + 2);
    result.push(saw_contour(first_boundary, contour.tool_position));
    for index in 1..=intermediate_saw_cuts {
        let fraction = f64::from(index) / (f64::from(intermediate_saw_cuts) + 1.0);
        let interpolate = |start: [f64; 2], end: [f64; 2]| {
            std::array::from_fn(|axis| start[axis] + (end[axis] - start[axis]) * fraction)
        };
        result.push(saw_contour(
            (
                interpolate(first_boundary.0, second_boundary.1),
                interpolate(first_boundary.1, second_boundary.0),
            ),
            "center",
        ));
    }
    result.push(saw_contour(second_boundary, contour.tool_position));
    Some(result)
}

/// Why a hole cannot be written as a BTLx drilling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BtlxDrillRefusal {
    /// The hole's frame, centre, size, depth or the stock is not a valid drilling.
    InvalidGeometry,
    /// The hole's axis does not run through the stock.
    AxisOutsideStock,
    /// The hole's wall breaks out through a side face of the stock.
    BreaksOutThroughSide,
    /// The hole is cut through but its cut does not span the stock.
    ThroughCutShort,
    /// A blind hole starts or ends outside the stock, or its tip breaks through
    /// the far face.
    BlindHoleLeavesStock,
    /// A coordinate, depth or diameter is beyond what BTLx writes.
    OutOfBtlxRange,
}

impl fmt::Display for BtlxDrillRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidGeometry => "its frame, centre, size or depth is not a valid drilling",
            Self::AxisOutsideStock => "its axis does not run through the stock",
            Self::BreaksOutThroughSide => {
                "its wall breaks out through a side face of the stock; move it inward or make it smaller"
            }
            Self::ThroughCutShort => "it is cut through, but the cut does not span the stock",
            Self::BlindHoleLeavesStock => {
                "it starts or ends outside the stock, or its tip breaks through the far face; drill it through or less deep"
            }
            Self::OutOfBtlxRange => "a coordinate, its depth or its diameter is beyond what BTLx writes",
        })
    }
}

pub(super) fn btlx_drilling(
    geometry: &GeneralMachiningGeometry,
    blank: StockBlank,
) -> Result<BtlxDrilling, BtlxDrillRefusal> {
    let GeneralMachiningGeometry::CircularDrill {
        frame,
        center_mm,
        diameter_mm,
        through,
        start_mm,
        end_mm,
    } = geometry
    else {
        return Err(BtlxDrillRefusal::InvalidGeometry);
    };
    if !machining_frame_is_right_handed(frame)
        || center_mm.iter().any(|value| !value.is_finite())
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
        || !diameter_mm.is_finite()
        || *diameter_mm <= 0.0
        || blank
            .extents_mm
            .iter()
            .any(|extent| !extent.is_finite() || *extent <= 0.0)
    {
        return Err(BtlxDrillRefusal::InvalidGeometry);
    }
    // Check the directed distance to the stock exit, not its smallest dimension
    // or the occurrence's world bounds. This also covers offset/angled frames.
    let tolerance = blank.extents_mm.into_iter().fold(1.0_f64, f64::max) * ROUNDING;
    let axis_point = |depth: f64| {
        blank.local(std::array::from_fn(|axis| {
            frame.origin_mm[axis]
                + frame.x_axis[axis] * center_mm[0]
                + frame.y_axis[axis] * center_mm[1]
                + frame.normal[axis] * depth
        }))
    };
    // Where the drill axis enters and leaves the stock, along the normal.
    let at_zero = axis_point(0.0);
    let (mut stock_entry, mut stock_exit) = (f64::NEG_INFINITY, f64::INFINITY);
    for (axis, extent) in blank.extents_mm.into_iter().enumerate() {
        let direction = frame.normal[axis];
        if direction == 0.0 {
            if !(-tolerance..=extent + tolerance).contains(&at_zero[axis]) {
                return Err(BtlxDrillRefusal::AxisOutsideStock);
            }
            continue;
        }
        let low = -at_zero[axis] / direction;
        let high = (extent - at_zero[axis]) / direction;
        stock_entry = stock_entry.max(low.min(high));
        stock_exit = stock_exit.min(low.max(high));
    }
    if !(stock_entry.is_finite() && stock_exit.is_finite() && stock_entry < stock_exit) {
        return Err(BtlxDrillRefusal::AxisOutsideStock);
    }
    // A hole that reaches the far face is drilled through, from where its axis
    // enters the stock to where it leaves; ThroughAll's padded Boolean cutter
    // interval only has to span the stock and never becomes the machine depth.
    let drilled_through = *through || *end_mm >= stock_exit - tolerance;
    let (start_mm, depth_mm) = if drilled_through {
        if *start_mm > stock_entry + tolerance || *end_mm < stock_exit - tolerance {
            return Err(BtlxDrillRefusal::ThroughCutShort);
        }
        (stock_entry, stock_exit - stock_entry)
    } else {
        let depth_mm = end_mm - start_mm;
        let entry_point = axis_point(*start_mm);
        for (axis, extent) in blank.extents_mm.into_iter().enumerate() {
            let entry = entry_point[axis];
            let end = entry + frame.normal[axis] * depth_mm;
            let tip_radius = diameter_mm / 2.0 * frame.x_axis[axis].hypot(frame.y_axis[axis]);
            if !entry.is_finite()
                || !end.is_finite()
                || entry < -tolerance
                || entry > extent + tolerance
                || end < -tolerance
                || end > extent + tolerance
                || (frame.normal[axis] > 0.0 && end + tip_radius >= extent - tolerance)
                || (frame.normal[axis] < 0.0 && end - tip_radius <= tolerance)
            {
                return Err(BtlxDrillRefusal::BlindHoleLeavesStock);
            }
        }
        (*start_mm, depth_mm)
    };
    // The wall stays inside every face the axis does not open through: at both
    // ends of the drilled length it reaches the radius times the share of the
    // axis's cross-section along that face's normal.
    for depth in [start_mm, start_mm + depth_mm] {
        let point = axis_point(depth);
        for (axis, extent) in blank.extents_mm.into_iter().enumerate() {
            let opens = frame.normal[axis] != 0.0
                && (point[axis] <= tolerance || point[axis] >= extent - tolerance);
            let reach = diameter_mm / 2.0 * frame.x_axis[axis].hypot(frame.y_axis[axis]);
            if !opens
                && (point[axis] - reach < -tolerance || point[axis] + reach > extent + tolerance)
            {
                return Err(BtlxDrillRefusal::BreaksOutThroughSide);
            }
        }
    }
    let reference_point_mm = btlx_part_coordinate(
        blank,
        blank.local(std::array::from_fn(|axis| {
            frame.origin_mm[axis] + frame.normal[axis] * start_mm
        })),
    );
    let in_range = |value: f64, minimum: f64, maximum: f64| {
        format_btlx_number_in_range(value, minimum, maximum).ok_or(BtlxDrillRefusal::OutOfBtlxRange)
    };
    Ok(BtlxDrilling {
        reference_point_mm,
        x_vector: btlx_part_coordinate(blank, frame.x_axis),
        y_vector: btlx_part_coordinate(blank, frame.y_axis),
        start_x: in_range(center_mm[0], -100_000.0, 100_000.0)?,
        start_y: in_range(center_mm[1], -50_000.0, 50_000.0)?,
        depth_limited: !drilled_through,
        depth: in_range(depth_mm, f64::MIN_POSITIVE, 50_000.0)?,
        diameter: in_range(*diameter_mm, f64::MIN_POSITIVE, 50_000.0)?,
        thread: None,
    })
}

pub(super) fn btlx_arc_points(
    start: [f64; 2],
    end: [f64; 2],
    center: [f64; 2],
    clockwise: bool,
) -> Option<Vec<[f64; 2]>> {
    if start
        .into_iter()
        .chain(end)
        .chain(center)
        .any(|coordinate| !coordinate.is_finite())
        || start == end
    {
        return None;
    }
    let start_vector = [start[0] - center[0], start[1] - center[1]];
    let end_vector = [end[0] - center[0], end[1] - center[1]];
    let start_radius = start_vector[0].hypot(start_vector[1]);
    let end_radius = end_vector[0].hypot(end_vector[1]);
    let radius_tolerance = ROUNDING * start_radius.max(end_radius).max(1.0);
    if start_radius <= 0.0 || (start_radius - end_radius).abs() > radius_tolerance {
        return None;
    }
    let start_angle = start_vector[1].atan2(start_vector[0]);
    let mut sweep = end_vector[1].atan2(end_vector[0]) - start_angle;
    if clockwise {
        if sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else if sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    let mut subdivisions = (sweep.abs() / (std::f64::consts::PI / 18.0))
        .ceil()
        .max(2.0) as usize;
    subdivisions += subdivisions % 2;
    Some(
        (1..=subdivisions)
            .map(|index| {
                let angle = start_angle + sweep * index as f64 / subdivisions as f64;
                if index == subdivisions {
                    end
                } else {
                    [
                        center[0] + start_radius * angle.cos(),
                        center[1] + start_radius * angle.sin(),
                    ]
                }
            })
            .collect(),
    )
}

pub(super) fn btlx_free_contour(
    geometry: &GeneralMachiningGeometry,
    blank: StockBlank,
) -> Option<BtlxFreeContour> {
    let GeneralMachiningGeometry::ProfileCut {
        frame,
        segments,
        start_mm,
        end_mm,
    } = geometry
    else {
        return None;
    };
    if !machining_frame_is_right_handed(frame)
        || !start_mm.is_finite()
        || !end_mm.is_finite()
        || end_mm <= start_mm
        || segments.len() < 2
    {
        return None;
    }
    let endpoints = segments
        .iter()
        .map(|segment| match segment {
            GeneralMachiningSegment::Line { start_mm, end_mm } => (start_mm
                .iter()
                .chain(end_mm)
                .all(|coordinate| coordinate.is_finite())
                && start_mm != end_mm)
                .then_some((*start_mm, *end_mm)),
            GeneralMachiningSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => btlx_arc_points(*start_mm, *end_mm, *center_mm, *clockwise)
                .map(|_| (*start_mm, *end_mm)),
        })
        .collect::<Option<Vec<_>>>()?;
    if !endpoints
        .iter()
        .zip(endpoints.iter().cycle().skip(1))
        .all(|((_, end), (next_start, _))| end == next_start)
    {
        return None;
    }
    let start_point = endpoints[0].0.map(format_number);
    let parse_point =
        |point: &[String; 2]| Some([point[0].parse::<f64>().ok()?, point[1].parse::<f64>().ok()?]);
    let mut rounded_points = vec![parse_point(&start_point)?];
    let mut btlx_segments = Vec::with_capacity(segments.len());
    for segment in segments {
        match segment {
            GeneralMachiningSegment::Line { end_mm, .. } => {
                let end_point = end_mm.map(format_number);
                rounded_points.push(parse_point(&end_point)?);
                btlx_segments.push(BtlxContourSegment::Line { end_point });
            }
            GeneralMachiningSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let arc_points = btlx_arc_points(*start_mm, *end_mm, *center_mm, *clockwise)?;
                let point_on_arc = arc_points[arc_points.len() / 2 - 1].map(format_number);
                let end_point = end_mm.map(format_number);
                if point_on_arc == start_mm.map(format_number) || point_on_arc == end_point {
                    return None;
                }
                rounded_points.extend(
                    arc_points
                        .iter()
                        .map(|point| parse_point(&point.map(format_number)))
                        .collect::<Option<Vec<_>>>()?,
                );
                btlx_segments.push(BtlxContourSegment::Arc {
                    end_point,
                    point_on_arc,
                });
            }
        }
    }
    let rounded_edges = rounded_points
        .windows(2)
        .map(|points| (points[0], points[1]))
        .collect::<Vec<_>>();
    let signed_double_area = rounded_edges
        .iter()
        .map(|(start, end)| start[0] * end[1] - end[0] * start[1])
        .sum::<f64>();
    if rounded_edges.iter().any(|(start, end)| start == end)
        || rounded_points.first() != rounded_points.last()
        || !btlx_polygon_is_simple(&rounded_edges)
        || !signed_double_area.is_finite()
        || signed_double_area == 0.0
    {
        return None;
    }
    let depth_mm = end_mm - start_mm;
    Some(BtlxFreeContour {
        reference_point_mm: btlx_part_coordinate(
            blank,
            blank.local(std::array::from_fn(|axis| {
                frame.origin_mm[axis] + frame.normal[axis] * start_mm
            })),
        ),
        x_vector: btlx_part_coordinate(blank, frame.x_axis),
        y_vector: btlx_part_coordinate(blank, frame.y_axis),
        tool_position: if signed_double_area > 0.0 {
            "left"
        } else {
            "right"
        },
        start_point,
        segments: btlx_segments,
        depth: format_btlx_number_in_range(depth_mm, f64::MIN_POSITIVE, 100_000.0)?,
    })
}

fn btlx_polygon_is_simple(edges: &[([f64; 2], [f64; 2])]) -> bool {
    let cross = |origin: [f64; 2], first: [f64; 2], second: [f64; 2]| {
        (first[0] - origin[0]) * (second[1] - origin[1])
            - (first[1] - origin[1]) * (second[0] - origin[0])
    };
    let point_on_segment = |point: [f64; 2], start: [f64; 2], end: [f64; 2]| {
        cross(start, end, point) == 0.0
            && point[0] >= start[0].min(end[0])
            && point[0] <= start[0].max(end[0])
            && point[1] >= start[1].min(end[1])
            && point[1] <= start[1].max(end[1])
    };
    let intersects = |(left_start, left_end): ([f64; 2], [f64; 2]),
                      (right_start, right_end): ([f64; 2], [f64; 2])| {
        let left_start_side = cross(left_start, left_end, right_start);
        let left_end_side = cross(left_start, left_end, right_end);
        let right_start_side = cross(right_start, right_end, left_start);
        let right_end_side = cross(right_start, right_end, left_end);
        (left_start_side * left_end_side < 0.0 && right_start_side * right_end_side < 0.0)
            || point_on_segment(right_start, left_start, left_end)
            || point_on_segment(right_end, left_start, left_end)
            || point_on_segment(left_start, right_start, right_end)
            || point_on_segment(left_end, right_start, right_end)
    };

    let edge_count = edges.len();
    if (0..edge_count).any(|index| {
        let previous = edges[(index + edge_count - 1) % edge_count].0;
        let current = edges[index].0;
        let next = edges[(index + 1) % edge_count].0;
        cross(previous, current, next) == 0.0
    }) {
        return false;
    }
    for left in 0..edge_count {
        for right in (left + 1)..edge_count {
            if right == left + 1 || (left == 0 && right == edge_count - 1) {
                continue;
            }
            if intersects(edges[left], edges[right]) {
                return false;
            }
        }
    }
    true
}

pub(super) fn format_btlx_number_in_range(
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Option<String> {
    let formatted = format_number(value);
    formatted
        .parse::<f64>()
        .ok()
        .filter(|rounded| rounded.is_finite() && *rounded >= minimum && *rounded <= maximum)
        .map(|_| formatted)
}

/// A definition point or vector in the BTLx part frame: X along the part's
/// length, Y and Z the next definition axes in cyclic order, so the frame is
/// a rotation of the definition's.
pub(super) fn btlx_part_coordinate(blank: StockBlank, definition_coordinate: [f64; 3]) -> [f64; 3] {
    let length = blank.btlx_length_axis();
    std::array::from_fn(|axis| definition_coordinate[(length + axis) % 3])
}
