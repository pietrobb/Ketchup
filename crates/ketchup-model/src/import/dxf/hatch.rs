//! Hatches of a drawing read as an underlay: the area a HATCH bounds, painted solid in its
//! color or drawn by the lines of its pattern clipped to that area.
//!
//! The area is what lies inside an odd number of its boundary paths: polylines (with
//! bulges) and edge paths of lines, arcs, elliptical arcs and splines, each joined into a
//! closed ring. Its style (group 75) and the flags of its paths decide which paths count:
//! all of them (normal), the outermost and the islands directly in them (outer), or the
//! outermost alone (ignore); a path flagged external or outermost is outermost.
//!
//! A pattern is drawn from the line definitions the hatch carries (groups 53, 43-46, 79
//! and 49), which a drawing writes already turned by the pattern's angle (52) and scaled
//! by its scale (41). Only a hatch that carries none is drawn from its name: ANSI31,
//! ANSI37 or a user-defined pattern of parallel (or, double, crossing) lines, turned and
//! scaled by 52 and 41. The dashes of all hatches together come out of the drawing's
//! entity budget ([`limits::DRAWING_ENTITIES`]).
//!
//! A hatch it cannot draw (a gradient, a boundary it cannot read, an area of nothing, a
//! pattern it has no lines of or one of more dashes than the budget has left) is left out
//! alone and named in the report by its layer, handle, why and where it lies.

use super::chain::report_mm;
use super::*;
use crate::tolerance::UNDERLAY_ARC_CHORD_MM;
use crate::underlay::fill::sweep_crossings;
use crate::underlay::{arc_chord_step, push_segment};
use ketchup_geometry::linalg::{cross2, dot2};
use std::collections::HashMap;

/// A hatch as an underlay draws it, in millimetres of the drawing (or of its block).
#[derive(Clone)]
pub(super) struct OrderedFill {
    pub(super) layer: String,
    pub(super) style: EntityStyle,
    /// Its handle, empty without one: the report names it where it is left out.
    pub(super) handle: String,
    /// The rings it paints solid, even-odd; empty for a pattern.
    pub(super) rings: Vec<Vec<[f64; 2]>>,
    /// The dashes of its pattern, clipped to its area.
    pub(super) pattern: Vec<[[f64; 2]; 2]>,
    /// The rings that bound its area, painted or not, until its object takes them.
    pub(super) outline: Vec<Vec<[f64; 2]>>,
}

/// The groups of a HATCH an underlay reads: its boundary, style, pattern and gradient.
const HATCH_GROUPS: [i16; 61] = [
    2, 5, 8, 10, 11, 12, 13, 20, 21, 22, 23, 30, 40, 41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 52,
    53, 62, 63, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 91, 92, 93, 94, 95, 96, 97, 98, 100, 210,
    220, 230, 330, 421, 450, 451, 452, 453, 460, 461, 462, 463, 470, 1001,
];

/// Points of one knot span of a spline edge.
const SPAN_CHORDS: u32 = 16;

/// How long a dot (a dash of length 0) of a pattern is drawn, as a share of how long the
/// pattern repeats: a dot of no length would not show.
const DOT_SHARE_OF_PERIOD: f64 = 0.02;

/// The spacing of ANSI31's lines, 1/8 inch: the same in the inch and the metric pattern
/// file.
const ANSI_SPACING_MM: f64 = 3.175;

/// One family of parallel pattern lines: their angle in degrees, a point one runs through,
/// the step from one to the next and the dashes each repeats from that point (> 0 a dash,
/// < 0 a gap, 0 a dot), all in millimetres.
struct PatternLine {
    angle_degrees: f64,
    base_mm: [f64; 2],
    offset_mm: [f64; 2],
    dashes_mm: Vec<f64>,
}

/// Reads the HATCH `record` as an underlay draws it, with the handle `handle`; a hatch it
/// cannot draw is reported and `None`.
pub(super) fn read_hatch(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    handle: Option<&str>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedFill>, DxfImportError> {
    let layer = parse_layer(record)?;
    report_ignored_groups("HATCH", record, &HATCH_GROUPS, diagnostics)?;
    let mut fill = OrderedFill {
        layer,
        style: EntityStyle::default(),
        handle: handle.unwrap_or_default().to_owned(),
        rings: Vec::new(),
        pattern: Vec::new(),
        outline: Vec::new(),
    };
    let scale = units.millimetres_per_unit();
    let drawn = match hatch_rings(record, units, &fill.layer, diagnostics) {
        Ok((rings, mirrored)) => {
            // The pattern is laid in the hatch's own coordinates, then all is mirrored
            // as the drawing mirrors the hatch.
            let flip = |[x, y]: [f64; 2]| if mirrored { [0.0 - x, y] } else { [x, y] };
            let bounds = point_bounds(rings.iter().flatten().copied().map(flip));
            fill.outline = rings
                .iter()
                .map(|ring| ring.iter().copied().map(flip).collect())
                .collect();
            match fill_rings(record, rings, scale, diagnostics) {
                Ok(drawn) => drawn
                    .map(|(mut rings, mut pattern)| {
                        rings
                            .iter_mut()
                            .flatten()
                            .for_each(|point| *point = flip(*point));
                        pattern
                            .iter_mut()
                            .flatten()
                            .for_each(|point| *point = flip(*point));
                        (rings, pattern)
                    })
                    .map_err(|reason| (reason, bounds)),
                Err(error) if error.ends_reading() => return Err(error),
                Err(error) => Err((LeftOut::Unreadable(error), bounds)),
            }
        }
        Err(error) if error.ends_reading() => return Err(error),
        Err(error) => Err((LeftOut::Unreadable(error), record_bounds(record, scale))),
    };
    match drawn {
        Ok((rings, pattern)) => {
            fill.rings = rings;
            fill.pattern = pattern;
            if let Err(reason) = charge(&fill, diagnostics) {
                skip_hatch(&fill, &reason, fill_bounds(&fill), diagnostics)?;
                return Ok(None);
            }
            Ok(Some(fill))
        }
        Err((reason, bounds)) => {
            skip_hatch(&fill, &reason, bounds, diagnostics)?;
            Ok(None)
        }
    }
}

/// What a hatch draws: the rings it paints solid and the dashes of its pattern.
type Drawn = (Vec<Vec<[f64; 2]>>, Vec<[[f64; 2]; 2]>);

/// Why a hatch is left out, as the report says it.
enum LeftOut {
    /// Its boundary could not be read.
    Unreadable(DxfImportError),
    Gradient,
    NoArea,
    /// A pattern of this name that carries no line definitions and is none it knows.
    UnknownPattern(String),
    /// Pattern lines with no step between them.
    StackedLines,
    /// More lines than dashes are left of the budget.
    TooManyLines {
        lines: f64,
        left: usize,
    },
    /// More dashes than are left of the budget.
    TooManyDashes {
        left: usize,
    },
    /// A solid fill whose filling crosses edges more often than is left of the budget.
    FillTooLarge {
        crossings: usize,
        left: usize,
    },
}

impl fmt::Display for LeftOut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let budget = limits::DRAWING_ENTITIES;
        match self {
            Self::Unreadable(error) => formatter.write_str(&error.reason()),
            Self::Gradient => formatter.write_str("a gradient fill"),
            Self::NoArea => formatter.write_str("a boundary that encloses no area"),
            Self::UnknownPattern(name) => {
                write!(
                    formatter,
                    "pattern {name} without line definitions in the file"
                )
            }
            Self::StackedLines => formatter.write_str("pattern lines that lie on one another"),
            Self::TooManyLines { lines, left } => write!(
                formatter,
                "a pattern of more lines ({lines}) than the {left} dashes left of the \
                 drawing's {budget} for entities"
            ),
            Self::TooManyDashes { left } => write!(
                formatter,
                "a pattern of more dashes than the {left} left of the drawing's {budget} for \
                 entities"
            ),
            Self::FillTooLarge { crossings, left } => write!(
                formatter,
                "a solid fill whose filling crosses its edges {crossings} times, more than \
                 the {left} left of the drawing's {budget} for entities"
            ),
        }
    }
}

/// What `rings` of the hatch `record` draw, or why they draw nothing.
fn fill_rings(
    record: &[DxfPair<'_>],
    rings: Vec<Vec<[f64; 2]>>,
    scale: f64,
    diagnostics: &DiagnosticCounts,
) -> Result<Result<Drawn, LeftOut>, DxfImportError> {
    if parse_optional_i64(record, 450)? == Some(1) {
        return Ok(Err(LeftOut::Gradient));
    }
    let area = rings
        .iter()
        .map(|ring| signed_area(ring).abs())
        .sum::<f64>();
    if area <= DXF_GEOMETRY_EPSILON_MM {
        return Ok(Err(LeftOut::NoArea));
    }
    if parse_required_i64(record, 70)? == 1 {
        return Ok(Ok((rings, Vec::new())));
    }
    let lines = match pattern_lines(record, scale)? {
        Ok(lines) => lines,
        Err(reason) => return Ok(Err(reason)),
    };
    Ok(pattern_pieces(&rings, &lines, diagnostics.fill_pieces_left)
        .map(|pieces| (Vec::new(), pieces)))
}

/// Reports `fill` left out for `reason`, where `bounds` (in millimetres of the drawing,
/// or of the block being read) lie.
fn skip_hatch(
    fill: &OrderedFill,
    reason: &LeftOut,
    bounds: Option<[[f64; 2]; 2]>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    let place = bounds.map_or_else(
        || " at no point it could read".to_owned(),
        |[low, high]| {
            format!(
                " within x {}..{}, y {}..{} mm",
                report_mm(low[0]),
                report_mm(high[0]),
                report_mm(low[1]),
                report_mm(high[1])
            )
        },
    );
    let block = diagnostics
        .block
        .as_ref()
        .map_or(String::new(), |name| format!(" of block {name}"));
    diagnostics.skip(
        "dxf.hatch-skipped",
        format!("HATCH, layer {}: {reason},{place}{block}", fill.layer),
        (!fill.handle.is_empty()).then_some(("handles", fill.handle.as_str())),
        None,
    )
}

/// `fills` of a block where an insert on `layer` places it, each point where `place`
/// puts it; a fill whose dashes or crossings the budget has no room for any more is
/// reported and left out.
pub(super) fn place_fills(
    fills: &[OrderedFill],
    layer: &str,
    place: impl Fn([f64; 2]) -> Result<[f64; 2], DxfImportError>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedFill>, DxfImportError> {
    let mut placed = Vec::with_capacity(fills.len());
    for fill in fills {
        let mut fill = OrderedFill {
            layer: if fill.layer == "0" {
                layer.to_owned()
            } else {
                fill.layer.clone()
            },
            style: fill.style.placed_on(layer),
            ..fill.clone()
        };
        for point in fill.rings.iter_mut().flatten() {
            *point = place(*point)?;
        }
        for piece in &mut fill.pattern {
            *piece = [place(piece[0])?, place(piece[1])?];
        }
        if let Err(reason) = charge(&fill, diagnostics) {
            skip_hatch(&fill, &reason, fill_bounds(&fill), diagnostics)?;
            continue;
        }
        placed.push(fill);
    }
    Ok(placed)
}

/// Takes the dashes of `fill` and the edge crossings of filling it from what is left of
/// the drawing's budget, or tells why there is no room for them.
fn charge(fill: &OrderedFill, diagnostics: &mut DiagnosticCounts) -> Result<(), LeftOut> {
    let left = diagnostics.fill_pieces_left;
    let crossings = sweep_crossings(fill.rings.iter().map(Vec::as_slice));
    let pieces = fill.pattern.len().saturating_add(crossings);
    if pieces > left {
        return Err(if fill.pattern.is_empty() {
            LeftOut::FillTooLarge { crossings, left }
        } else {
            LeftOut::TooManyDashes { left }
        });
    }
    diagnostics.fill_pieces_left = left - pieces;
    Ok(())
}

/// The smallest and largest corner of what `fill` draws.
fn fill_bounds(fill: &OrderedFill) -> Option<[[f64; 2]; 2]> {
    let rings = fill.rings.iter().flatten();
    point_bounds(rings.chain(fill.pattern.iter().flatten()).copied())
}

/// The smallest and largest corner of `points`.
fn point_bounds(mut points: impl Iterator<Item = [f64; 2]>) -> Option<[[f64; 2]; 2]> {
    let first = points.next()?;
    Some(points.fold([first, first], |[low, high], point| {
        [
            [low[0].min(point[0]), low[1].min(point[1])],
            [high[0].max(point[0]), high[1].max(point[1])],
        ]
    }))
}

/// The box of every point (group 10 and the 20 after it) the boundary of `record` names,
/// for a hatch whose boundary cannot be read.
fn record_bounds(record: &[DxfPair<'_>], scale: f64) -> Option<[[f64; 2]; 2]> {
    let start = record.iter().position(|pair| pair.code == 92)?;
    let points = record[start..].windows(2).filter_map(|pair| {
        let x = (pair[0].code == 10).then(|| pair[0].value.trim().parse::<f64>().ok())??;
        let y = (pair[1].code == 20).then(|| pair[1].value.trim().parse::<f64>().ok())??;
        Some([x * scale, y * scale])
    });
    point_bounds(points.filter(|point| point.iter().all(|value| value.is_finite())))
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(start, end)| cross2(*start, *end))
        .sum::<f64>()
        / 2.0
}

/// Whether `point` lies inside `ring` (an odd number of its edges cross the ray from it
/// to +x).
fn inside(point: [f64; 2], ring: &[[f64; 2]]) -> bool {
    #[cfg(test)]
    tests::INSIDE_TESTS.set(tests::INSIDE_TESTS.get() + 1);
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .filter(|(start, end)| {
            (start[1] > point[1]) != (end[1] > point[1])
                && point[0]
                    < start[0] + (end[0] - start[0]) * (point[1] - start[1]) / (end[1] - start[1])
        })
        .count()
        % 2
        == 1
}

/// A closed boundary path as its corners, the last joined back to the first.
type Ring = Vec<[f64; 2]>;

/// The rings of the boundary paths of `record`, in millimetres of the hatch's own
/// coordinates, those its style leaves out not among them; and whether the drawing
/// mirrors those coordinates (extrusion (0, 0, -1)).
fn hatch_rings(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    layer: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(Vec<Ring>, bool), DxfImportError> {
    let mirrored = ensure_planar_ocs(record)?;
    let path_count = parse_required_i64(record, 91)?;
    let starts = record
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.code == 92)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if path_count < 1 || usize::try_from(path_count) != Ok(starts.len()) {
        return Err(DxfImportError::MalformedPairs);
    }
    let paths_end = record.len() - polygon_trailing_record(record)?.len();
    let mut rings = Vec::with_capacity(starts.len());
    let mut outermost = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(paths_end);
        let path_type = parse_integer::<i64>(record[start].value)?;
        let path = &record[start + 1..end];
        // The handles of the entities it was made from close the path.
        let path = &path[..path
            .iter()
            .rposition(|pair| pair.code == 97)
            .ok_or(DxfImportError::MalformedPairs)?];
        let mut ring = if path_type & 2 != 0 {
            polyline_ring(path, layer, units, diagnostics)?
        } else {
            edge_ring(path, units.millimetres_per_unit())?
        };
        ring.dedup();
        while ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() < 3 {
            return Err(DxfImportError::DegenerateGeometry);
        }
        rings.push(ring);
        outermost.push(path_type & (1 | 16) != 0);
    }
    let style = parse_optional_i64(record, 75)?.unwrap_or(0);
    Ok((counted_rings(rings, &outermost, style), mirrored))
}

/// The rings of `rings` a hatch of `style` paints, even-odd: all (normal, 0), the
/// outermost and the islands directly in them (outer, 1), or the outermost alone
/// (ignore, 2). The outermost are those `outermost` flags, or without any flagged those
/// inside no other.
fn counted_rings(rings: Vec<Vec<[f64; 2]>>, outermost: &[bool], style: i64) -> Vec<Vec<[f64; 2]>> {
    if !matches!(style, 1 | 2) {
        return rings;
    }
    let flagged = outermost.iter().any(|flagged| *flagged);
    let containers = if style == 1 || !flagged {
        containing_rings(&rings)
    } else {
        Vec::new()
    };
    let outer = if flagged {
        outermost.to_vec()
    } else {
        containers.iter().map(Vec::is_empty).collect()
    };
    let kept = (0..rings.len())
        .map(|ring| {
            outer[ring]
                || (style == 1
                    && containers[ring].iter().any(|other| outer[*other])
                    && containers[ring].iter().all(|other| outer[*other]))
        })
        .collect::<Vec<_>>();
    rings
        .into_iter()
        .zip(kept)
        .filter_map(|(ring, kept)| kept.then_some(ring))
        .collect()
}

/// For each of `rings`, the others its first corner lies inside. A point outside a
/// ring's box is outside the ring, so only rings whose box covers the grid cell of the
/// corner, and the corner itself, are tested; the grid has about one cell per ring.
fn containing_rings(rings: &[Ring]) -> Vec<Vec<usize>> {
    let boxes: Vec<_> = rings
        .iter()
        .map(|ring| point_bounds(ring.iter().copied()))
        .collect();
    let Some([low, high]) = point_bounds(boxes.iter().flatten().flatten().copied()) else {
        return vec![Vec::new(); rings.len()];
    };
    let across = f64::from(u32::try_from(rings.len().isqrt()).unwrap_or(u32::MAX)).max(1.0);
    let size = [0, 1].map(|axis| ((high[axis] - low[axis]) / across).max(f64::MIN_POSITIVE));
    // Whole-number column and row, at most `across`, so no cell lies off the grid.
    let cell = |point: [f64; 2]| {
        [0, 1].map(|axis| {
            ((point[axis] - low[axis]) / size[axis])
                .floor()
                .clamp(0.0, across)
                + 0.0
        })
    };
    let mut grid: HashMap<[u64; 2], Vec<usize>> = HashMap::new();
    for (index, bounds) in boxes.iter().enumerate() {
        let Some([first, last]) = bounds.map(|bounds| bounds.map(cell)) else {
            continue;
        };
        let mut column = first[0];
        while column <= last[0] {
            let mut row = first[1];
            while row <= last[1] {
                grid.entry([column.to_bits(), row.to_bits()])
                    .or_default()
                    .push(index);
                row += 1.0;
            }
            column += 1.0;
        }
    }
    rings
        .iter()
        .enumerate()
        .map(|(ring, points)| {
            let point = points[0];
            let [column, row] = cell(point);
            let candidates = grid.get(&[column.to_bits(), row.to_bits()]);
            candidates
                .into_iter()
                .flatten()
                .copied()
                .filter(|other| {
                    *other != ring
                        && boxes[*other].is_some_and(|[low, high]| {
                            (0..2).all(|axis| low[axis] <= point[axis] && point[axis] <= high[axis])
                        })
                        && inside(point, &rings[*other])
                })
                .collect()
        })
        .collect()
}

/// The ring of a polyline path: its vertices (group 93 counts them), joined by straight
/// lines or, where a vertex has a bulge (42), by arcs.
fn polyline_ring(
    path: &[DxfPair<'_>],
    layer: &str,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<[f64; 2]>, DxfImportError> {
    let declared = parse_required_i64(path, 93)?;
    let start = path
        .iter()
        .position(|pair| pair.code == 93)
        .ok_or(DxfImportError::MalformedPairs)?;
    let mut vertices = Vec::new();
    for pair in &path[start + 1..] {
        match pair.code {
            10 => vertices.push(PolylineVertex {
                x: parse_number(pair.value)?,
                y: None,
                bulge: None,
            }),
            20 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                vertex.y = Some(parse_number(pair.value)?);
            }
            42 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                vertex.bulge = Some(parse_number(pair.value)?);
            }
            _ => {}
        }
    }
    if usize::try_from(declared) != Ok(vertices.len())
        || vertices.iter().any(|vertex| vertex.y.is_none())
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let profile = build_polyline_profile(layer.to_owned(), 0, vertices, true, units, diagnostics)?;
    let mut points = Vec::new();
    for segment in &profile.segments {
        if points.is_empty() {
            points.push(segment.start_mm());
        }
        push_segment(&mut points, segment);
    }
    Ok(points)
}

/// The ring of an edge path: its edges (group 72 starts each) in order, each turned to
/// meet the one before; a gap between two is closed by a straight line.
fn edge_ring(path: &[DxfPair<'_>], scale: f64) -> Result<Vec<[f64; 2]>, DxfImportError> {
    let declared = parse_required_i64(path, 93)?;
    let start = path
        .iter()
        .position(|pair| pair.code == 93)
        .ok_or(DxfImportError::MalformedPairs)?;
    let edges = &path[start + 1..];
    let starts = edges
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.code == 72)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if declared < 1 || usize::try_from(declared) != Ok(starts.len()) || starts.first() != Some(&0) {
        return Err(DxfImportError::MalformedPairs);
    }
    let meets = |left: [f64; 2], right: [f64; 2]| {
        (left[0] - right[0]).hypot(left[1] - right[1]) <= UNDERLAY_ARC_CHORD_MM
    };
    let mut ring: Vec<[f64; 2]> = Vec::new();
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(edges.len());
        let edge = &edges[start..end];
        let mut points = match parse_required_i64(edge, 72)? {
            1 => vec![
                parse_xy(edge, 10, 20, scale)?,
                parse_xy(edge, 11, 21, scale)?,
            ],
            kind @ (2 | 3) => elliptical_edge(edge, kind == 3, scale)?,
            4 => spline_edge(edge, scale)?,
            _ => return Err(DxfImportError::MalformedPairs),
        };
        if let (Some(&last), Some(&first), Some(&end)) =
            (ring.last(), points.first(), points.last())
            && !meets(last, first)
            && meets(last, end)
        {
            points.reverse();
        }
        let joined = ring.last().is_some_and(|last| meets(*last, points[0]));
        ring.extend(points.into_iter().skip(usize::from(joined)));
    }
    Ok(ring)
}

/// The points of an arc edge, or with `elliptical` of an elliptical one, in the direction
/// it runs. A clockwise edge (73 = 0) gives its angles as those of its mirror image, 360°
/// less each.
fn elliptical_edge(
    edge: &[DxfPair<'_>],
    elliptical: bool,
    scale: f64,
) -> Result<Vec<[f64; 2]>, DxfImportError> {
    let center = parse_xy(edge, 10, 20, scale)?;
    let (major, ratio) = if elliptical {
        let ratio = parse_required_number(edge, 40)?;
        (parse_xy(edge, 11, 21, scale)?, ratio)
    } else {
        (
            [
                scale_coordinate(parse_required_number(edge, 40)?, scale)?,
                0.0,
            ],
            1.0,
        )
    };
    let radius = major[0].hypot(major[1]);
    if radius <= DXF_GEOMETRY_EPSILON_MM || ratio <= 0.0 || ratio > 1.0 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let counterclockwise = parse_optional_i64(edge, 73)?.unwrap_or(1) != 0;
    let [start, end] = [50, 51].map(|code| parse_required_number(edge, code));
    let (mut start, mut end) = (start?, end?);
    if !counterclockwise {
        (start, end) = (360.0 - end, 360.0 - start);
    }
    // An angle of an ellipse's point is not the parameter that reaches it.
    let parameter = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        (sin / ratio).atan2(cos)
    };
    let tau = std::f64::consts::TAU;
    let from = parameter(start);
    let mut sweep = (parameter(end) - from).rem_euclid(tau);
    if sweep <= DXF_GEOMETRY_EPSILON_MM || (end - start).abs() >= 360.0 {
        sweep = tau;
    }
    let mut points = ellipse_points(center, major, ratio, from, sweep);
    if !counterclockwise {
        points.reverse();
    }
    Ok(points)
}

/// The points of the ellipse around `center` with the major semi-axis `major` and the
/// minor one `ratio` of it, from parameter `from` on by `sweep`, counterclockwise: its
/// chords stay within [`UNDERLAY_ARC_CHORD_MM`] of the major circle's arc.
pub(super) fn ellipse_points(
    center: [f64; 2],
    major: [f64; 2],
    ratio: f64,
    from: f64,
    sweep: f64,
) -> Vec<[f64; 2]> {
    let minor = [-major[1] * ratio, major[0] * ratio];
    let chords = (sweep / arc_chord_step(major[0].hypot(major[1])))
        .ceil()
        .max(1.0);
    let mut points = Vec::new();
    let mut index = 0.0;
    while index <= chords {
        let (sin, cos) = (from + sweep * index / chords).sin_cos();
        points.push([
            center[0] + major[0] * cos + minor[0] * sin,
            center[1] + major[1] * cos + minor[1] * sin,
        ]);
        index += 1.0;
    }
    points
}

/// The points of a spline edge: its B-spline (rational with weights, group 42) evaluated
/// span by span, or without control points the line through its fit points.
fn spline_edge(edge: &[DxfPair<'_>], scale: f64) -> Result<Vec<[f64; 2]>, DxfImportError> {
    let degree = usize::try_from(parse_required_i64(edge, 94)?)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let scaled = |points: Vec<[f64; 2]>| {
        points
            .into_iter()
            .map(|[x, y]| Ok([scale_coordinate(x, scale)?, scale_coordinate(y, scale)?]))
            .collect::<Result<Vec<_>, DxfImportError>>()
    };
    let controls = scaled(parse_spline_points(edge, 10, 20, 30)?)?;
    if controls.is_empty() {
        let fit = scaled(parse_spline_points(edge, 11, 21, 31)?)?;
        if fit.len() < 2 {
            return Err(DxfImportError::MalformedPairs);
        }
        return Ok(fit);
    }
    let knots = edge
        .iter()
        .filter(|pair| pair.code == 40)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    let weights = edge
        .iter()
        .filter(|pair| pair.code == 42)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    bspline_points(degree, &controls, &knots, &weights)
}

/// The points of the B-spline of `degree` through `controls` over `knots`, rational
/// with `weights` (none: polynomial), evaluated span by span.
pub(super) fn bspline_points(
    degree: usize,
    controls: &[[f64; 2]],
    knots: &[f64],
    weights: &[f64],
) -> Result<Vec<[f64; 2]>, DxfImportError> {
    if degree < 1
        || controls.len() <= degree
        || knots.len() != controls.len() + degree + 1
        || knots.windows(2).any(|pair| pair[0] > pair[1])
        || knots[degree] >= knots[controls.len()]
        || (!weights.is_empty() && weights.len() != controls.len())
        || weights.iter().any(|weight| *weight <= 0.0)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    // Each control point weighted, with its weight: a rational spline is the polynomial
    // one of these, divided by the weight.
    let weighted = controls
        .iter()
        .enumerate()
        .map(|(index, [x, y])| {
            let weight = weights.get(index).copied().unwrap_or(1.0);
            [x * weight, y * weight, weight]
        })
        .collect::<Vec<_>>();
    let at = |span: usize, t: f64| {
        let mut points = weighted[span - degree..=span].to_vec();
        for level in 1..=degree {
            for index in (level..=degree).rev() {
                let knot = span - degree + index;
                let width = knots[knot + degree + 1 - level] - knots[knot];
                let share = if width > 0.0 {
                    (t - knots[knot]) / width
                } else {
                    0.0
                };
                let before = points[index - 1];
                points[index] = std::array::from_fn(|axis| {
                    before[axis] + (points[index][axis] - before[axis]) * share
                });
            }
        }
        let [x, y, weight] = points[degree];
        [x / weight, y / weight]
    };
    let mut points = vec![at(degree, knots[degree])];
    for span in degree..controls.len() {
        let (low, high) = (knots[span], knots[span + 1]);
        if high <= low {
            continue;
        }
        for step in 1..=SPAN_CHORDS {
            let share = f64::from(step) / f64::from(SPAN_CHORDS);
            points.push(at(span, low + (high - low) * share));
        }
    }
    Ok(points)
}

/// The pattern lines of the hatch `record`, in millimetres: those it carries, or of the
/// pattern it names when it carries none; or why it has none.
fn pattern_lines(
    record: &[DxfPair<'_>],
    scale: f64,
) -> Result<Result<Vec<PatternLine>, LeftOut>, DxfImportError> {
    let trailing = polygon_trailing_record(record)?;
    let mut lines: Vec<PatternLine> = Vec::new();
    for pair in trailing {
        if pair.code == 53 {
            lines.push(PatternLine {
                angle_degrees: parse_number(pair.value)?,
                base_mm: [0.0; 2],
                offset_mm: [0.0; 2],
                dashes_mm: Vec::new(),
            });
            continue;
        }
        let Some(line) = lines.last_mut() else {
            continue;
        };
        let value = || Ok::<_, DxfImportError>(parse_number(pair.value)? * scale);
        match pair.code {
            43 => line.base_mm[0] = value()?,
            44 => line.base_mm[1] = value()?,
            45 => line.offset_mm[0] = value()?,
            46 => line.offset_mm[1] = value()?,
            49 => line.dashes_mm.push(value()?),
            _ => {}
        }
    }
    if !lines.is_empty() {
        return Ok(Ok(lines));
    }
    let name = parse_optional_text(record, 2)?.unwrap_or_default();
    let angle = parse_optional_number(trailing, 52)?.unwrap_or(0.0);
    let factor = parse_optional_number(trailing, 41)?.unwrap_or(1.0);
    // Lines of a pattern file, at angle 0 and scale 1: angle, then the step to the next
    // line along and across it.
    let (defined, unit): (&[(f64, [f64; 2])], f64) = if name.eq_ignore_ascii_case("ANSI31") {
        (&[(45.0, [0.0, ANSI_SPACING_MM])], 1.0)
    } else if name.eq_ignore_ascii_case("ANSI37") {
        (
            &[
                (45.0, [0.0, ANSI_SPACING_MM]),
                (135.0, [0.0, ANSI_SPACING_MM]),
            ],
            1.0,
        )
    } else if parse_optional_i64(trailing, 76)? == Some(0) {
        // A user-defined pattern: lines 41 drawing units apart, crossed with 77 = 1.
        let double = parse_optional_i64(trailing, 77)? == Some(1);
        let lines: &[(f64, [f64; 2])] = if double {
            &[(0.0, [0.0, 1.0]), (90.0, [0.0, 1.0])]
        } else {
            &[(0.0, [0.0, 1.0])]
        };
        (lines, scale)
    } else {
        return Ok(Err(LeftOut::UnknownPattern(name.to_owned())));
    };
    Ok(Ok(defined
        .iter()
        .map(|(own, [along, across])| {
            let degrees = own + angle;
            let (sin, cos) = degrees.to_radians().sin_cos();
            let step = factor * unit;
            PatternLine {
                angle_degrees: degrees,
                base_mm: [0.0; 2],
                offset_mm: [
                    (along * cos - across * sin) * step,
                    (along * sin + across * cos) * step,
                ],
                dashes_mm: Vec::new(),
            }
        })
        .collect()))
}

/// The dashes of `lines` inside the even-odd area of `rings`, at most `budget` of them;
/// or why there are none to draw. The lines of a family are swept across the area from
/// its lowest edge up, each meeting only the edges that span its height.
fn pattern_pieces(
    rings: &[Vec<[f64; 2]>],
    lines: &[PatternLine],
    budget: usize,
) -> Result<Vec<[[f64; 2]; 2]>, LeftOut> {
    let budget_lines = f64::from(u32::try_from(budget).unwrap_or(u32::MAX));
    let mut pieces = Vec::new();
    for line in lines {
        let (sin, cos) = line.angle_degrees.to_radians().sin_cos();
        // Along the lines, and across them towards the next.
        let along = [cos, sin];
        let mut across = [-sin, cos];
        let mut spacing = dot2(across, line.offset_mm);
        if spacing.abs() <= DXF_GEOMETRY_EPSILON_MM {
            return Err(LeftOut::StackedLines);
        }
        if spacing < 0.0 {
            across = [0.0 - across[0], 0.0 - across[1]];
            spacing = 0.0 - spacing;
        }
        // Each edge in (along, across) coordinates, from its lower end across.
        let mut edges = rings
            .iter()
            .flat_map(|ring| ring.iter().zip(ring.iter().cycle().skip(1)))
            .map(|(start, end)| {
                let [start, end] =
                    [start, end].map(|point| [dot2(along, *point), dot2(across, *point)]);
                if start[1] <= end[1] {
                    (start, end)
                } else {
                    (end, start)
                }
            })
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| left.0[1].total_cmp(&right.0[1]));
        let low = edges.first().map_or(0.0, |edge| edge.0[1]);
        let high = edges.iter().map(|edge| edge.1[1]).fold(low, f64::max);
        let base_across = dot2(across, line.base_mm);
        let base_along = dot2(along, line.base_mm);
        let step_along = dot2(along, line.offset_mm);
        // The lines over the area, by how many steps they lie from the base.
        let mut index = ((low - base_across) / spacing).ceil();
        let last = ((high - base_across) / spacing).floor();
        if last - index + 1.0 > budget_lines {
            return Err(LeftOut::TooManyLines {
                lines: last - index + 1.0,
                left: budget,
            });
        }
        let period = line.dashes_mm.iter().map(|dash| dash.abs()).sum::<f64>();
        let continuous = period <= DXF_GEOMETRY_EPSILON_MM;
        let dot = period * DOT_SHARE_OF_PERIOD;
        // A line of gaps alone draws nothing.
        let drawn = continuous || line.dashes_mm.iter().any(|dash| *dash >= 0.0);
        let mut active = Vec::<usize>::new();
        let mut next = 0;
        let mut crossings = Vec::new();
        while drawn && index <= last {
            let height = base_across + index * spacing;
            // Where along it its dashes start.
            let origin = base_along + index * step_along;
            index += 1.0;
            while next < edges.len() && edges[next].0[1] <= height {
                active.push(next);
                next += 1;
            }
            active.retain(|edge| edges[*edge].1[1] > height);
            crossings.clear();
            crossings.extend(active.iter().map(|edge| {
                let (start, end) = edges[*edge];
                start[0] + (end[0] - start[0]) * (height - start[1]) / (end[1] - start[1])
            }));
            crossings.sort_by(f64::total_cmp);
            let point = |at: f64| {
                [
                    along[0] * at + across[0] * height,
                    along[1] * at + across[1] * height,
                ]
            };
            for inside in crossings.chunks_exact(2) {
                let (from, to) = (inside[0], inside[1]);
                if continuous {
                    pieces.push([point(from), point(to)]);
                } else {
                    let mut start = origin + ((from - origin) / period).floor() * period;
                    while start < to {
                        let mut dash_start = start;
                        for dash in &line.dashes_mm {
                            if *dash >= 0.0 {
                                let dash_end = dash_start + dash.max(dot);
                                let (clipped_start, clipped_end) =
                                    (dash_start.max(from), dash_end.min(to));
                                if clipped_end > clipped_start {
                                    pieces.push([point(clipped_start), point(clipped_end)]);
                                }
                            }
                            dash_start += dash.abs();
                        }
                        start += period;
                        if pieces.len() > budget {
                            return Err(LeftOut::TooManyDashes { left: budget });
                        }
                    }
                }
                if pieces.len() > budget {
                    return Err(LeftOut::TooManyDashes { left: budget });
                }
            }
        }
    }
    Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        /// How often [`inside`] tested a point against a ring on this thread.
        pub(super) static INSIDE_TESTS: Cell<usize> = const { Cell::new(0) };
    }

    fn square(low: [f64; 2], size: f64) -> Ring {
        let [x, y] = low;
        vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]]
    }

    /// Review 2026-10-24 UL-26: an outer or ignore hatch tested every ring against every
    /// other before the budget saw it, so 10 000 islands took seconds.
    #[test]
    fn islands_are_tested_only_against_rings_around_them() {
        // A floor with 40 x 40 tiles, each with a small hole in it.
        let mut rings = vec![square([0.0, 0.0], 10_000.0)];
        for column in 0..40 {
            for row in 0..40 {
                let low = [
                    100.0 + 200.0 * f64::from(column),
                    100.0 + 200.0 * f64::from(row),
                ];
                rings.push(square(low, 100.0));
                rings.push(square([low[0] + 40.0, low[1] + 40.0], 20.0));
            }
        }
        let count = rings.len();
        for (style, kept) in [(1, 1 + 1600), (2, 1)] {
            INSIDE_TESTS.set(0);
            let counted = counted_rings(rings.clone(), &vec![false; count], style);
            assert_eq!(counted.len(), kept, "style {style}");
            // A tile lies in the floor, a hole in its tile and the floor.
            assert!(
                INSIDE_TESTS.get() <= 2 * count,
                "{} tests",
                INSIDE_TESTS.get()
            );
        }
        // Outer keeps the tiles, not the holes in them.
        let counted = counted_rings(rings, &vec![false; count], 1);
        assert!(
            counted
                .iter()
                .all(|ring| ring.len() == 4 && ring[1][0] - ring[0][0] >= 100.0)
        );
    }
}
