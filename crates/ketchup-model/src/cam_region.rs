//! Planar regions a CAM operation machines, read from an exact face of the
//! part, and the cutter-centre paths that clear or follow them.
use crate::cam::{CamPath2d, CamPathSegment2d, CamPlannerError, CamSetup};
use crate::cam_offset::{
    LOOP_TOLERANCE_MM, Point, Segment, degenerate_lane, distance, intersections, lane_crossings,
    loops_meet, offset_right, oriented, reversed, signed_area,
};
use crate::exact_product::{ExactBRepGraphEdgeEvidence, ExactBRepGraphPackage};
use ketchup_geometry::linalg::{cross, dot};

/// Most boundary segments one region holds.
const MAX_REGION_SEGMENTS: usize = 1_024;
/// Most scan lanes one pocket clears.
const MAX_POCKET_LANES: usize = 50_000;

/// A closed region in the setup's XY plane: the outer boundary and the islands
/// inside it that the cutter must leave standing.
#[derive(Clone, Debug, PartialEq)]
pub struct CamRegion2d {
    pub outer: CamPath2d,
    pub islands: Vec<CamPath2d>,
}

/// Which side of a region's boundary a contour cutter runs on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CamContourSide {
    /// Around the outside of the outer boundary, as when profiling a part free.
    Outside,
    /// Inside the outer boundary and around each island, as when finishing pocket walls.
    Inside,
}

fn refused() -> CamPlannerError {
    CamPlannerError::UnsupportedRegion
}

impl CamRegion2d {
    /// The loops of one planar face of an evaluated part, in setup coordinates:
    /// the loop enclosing the others is the outer boundary, the rest are islands.
    /// The face must lie square to the setup's Z axis and be bounded by lines
    /// and whole circles; anything else is refused rather than approximated.
    pub fn from_planar_face(
        package: &ExactBRepGraphPackage,
        face_ordinal: u32,
        setup: &CamSetup,
    ) -> Result<Self, CamPlannerError> {
        let face = package
            .face_evidence
            .iter()
            .find(|face| face.face_ordinal == face_ordinal)
            .ok_or_else(refused)?;
        let z_axis = cross(setup.x_axis, setup.y_axis);
        if face.surface_kind != "plane" || dot(face.unit_normal, z_axis).abs() < 1.0 - 1.0e-9 {
            return Err(refused());
        }
        let to_setup = |point: [f64; 3]| {
            let relative = [
                point[0] - setup.origin_mm[0],
                point[1] - setup.origin_mm[1],
                point[2] - setup.origin_mm[2],
            ];
            [dot(relative, setup.x_axis), dot(relative, setup.y_axis)]
        };
        let mut lines = Vec::new();
        let mut loops = Vec::new();
        for edge in package
            .edge_evidence
            .iter()
            .filter(|edge| edge.adjacent_face_ordinals.contains(&face_ordinal))
        {
            match boundary_piece(edge, to_setup)? {
                Piece::Line(segment) => lines.push(segment),
                Piece::Circle(segments) => loops.push(segments),
            }
        }
        loops.extend(chain_lines(lines)?);
        if loops.is_empty() || loops.iter().map(Vec::len).sum::<usize>() > MAX_REGION_SEGMENTS {
            return Err(refused());
        }
        loops.sort_by(|left, right| {
            signed_area(right)
                .abs()
                .total_cmp(&signed_area(left).abs())
        });
        let outer = oriented(&loops[0], true);
        let mut islands = Vec::with_capacity(loops.len() - 1);
        for island in &loops[1..] {
            if !inside(&[outer.clone()], island[0].start()) {
                return Err(refused());
            }
            islands.push(path_of(&oriented(island, false)));
        }
        Ok(Self {
            outer: path_of(&outer),
            islands,
        })
    }

    fn loops(&self) -> Result<(Vec<Segment>, Vec<Vec<Segment>>), CamPlannerError> {
        let outer = oriented(&segments_of(&self.outer)?, true);
        let mut islands = Vec::with_capacity(self.islands.len());
        for island in &self.islands {
            islands.push(oriented(&segments_of(island)?, true));
        }
        if islands.len() + 1 > MAX_REGION_SEGMENTS {
            return Err(refused());
        }
        Ok((outer, islands))
    }

    /// Every point of the region's boundary, for checking it against the target.
    pub(crate) fn bounds(&self) -> Result<[Point; 2], CamPlannerError> {
        let (outer, islands) = self.loops()?;
        Ok(loop_bounds(std::iter::once(&outer).chain(&islands)))
    }
}

enum Piece {
    Line(Segment),
    Circle(Vec<Segment>),
}

fn boundary_piece(
    edge: &ExactBRepGraphEdgeEvidence,
    to_setup: impl Fn([f64; 3]) -> Point,
) -> Result<Piece, CamPlannerError> {
    let origin = edge.axis_origin_mm.ok_or_else(refused)?;
    let direction = edge.unit_axis_direction.ok_or_else(refused)?;
    match (edge.curve_kind.as_str(), edge.closed, edge.circle_radius_mm) {
        ("line", false, None) => {
            let end = [0, 1, 2].map(|axis| origin[axis] + direction[axis] * edge.length_mm);
            Ok(Piece::Line(Segment::Line {
                start: to_setup(origin),
                end: to_setup(end),
            }))
        }
        // A whole circle, as two half arcs so each motion has distinct ends.
        ("circle", true, Some(radius)) if radius.is_finite() && radius > LOOP_TOLERANCE_MM => {
            let center = to_setup(origin);
            let east = [center[0] + radius, center[1]];
            let west = [center[0] - radius, center[1]];
            Ok(Piece::Circle(vec![
                Segment::Arc {
                    start: east,
                    end: west,
                    center,
                    ccw: true,
                },
                Segment::Arc {
                    start: west,
                    end: east,
                    center,
                    ccw: true,
                },
            ]))
        }
        // Partial arcs need their end points, which face evidence does not carry yet.
        _ => Err(refused()),
    }
}

/// Joins line pieces end to end into closed loops.
fn chain_lines(mut lines: Vec<Segment>) -> Result<Vec<Vec<Segment>>, CamPlannerError> {
    let mut loops = Vec::new();
    while let Some(first) = lines.pop() {
        let mut chain = vec![first];
        while distance(chain[chain.len() - 1].end(), chain[0].start()) > LOOP_TOLERANCE_MM {
            let tail = chain[chain.len() - 1].end();
            let index = lines
                .iter()
                .position(|segment| {
                    distance(segment.start(), tail) <= LOOP_TOLERANCE_MM
                        || distance(segment.end(), tail) <= LOOP_TOLERANCE_MM
                })
                .ok_or_else(refused)?;
            let next = lines.swap_remove(index);
            let next = if distance(next.start(), tail) <= LOOP_TOLERANCE_MM {
                next
            } else {
                next.reversed()
            };
            // Snap so consecutive segments share one exact point.
            chain.push(match next {
                Segment::Line { end, .. } => Segment::Line { start: tail, end },
                arc => arc,
            });
        }
        let last = chain.len() - 1;
        let start = chain[0].start();
        if let Segment::Line { start: from, .. } = chain[last] {
            chain[last] = Segment::Line { start: from, end: start };
        }
        if chain.len() < 3 {
            return Err(refused());
        }
        loops.push(chain);
    }
    Ok(loops)
}

fn path_of(segments: &[Segment]) -> CamPath2d {
    CamPath2d {
        start_mm: segments[0].start(),
        segments: segments
            .iter()
            .map(|segment| match segment {
                Segment::Line { end, .. } => CamPathSegment2d::Line { to_mm: *end },
                Segment::Arc {
                    end, center, ccw, ..
                } => CamPathSegment2d::Arc {
                    to_mm: *end,
                    center_mm: *center,
                    clockwise: !ccw,
                },
            })
            .collect(),
    }
}

fn segments_of(path: &CamPath2d) -> Result<Vec<Segment>, CamPlannerError> {
    if path.segments.is_empty() || path.segments.len() > MAX_REGION_SEGMENTS {
        return Err(refused());
    }
    let mut current = path.start_mm;
    let mut segments = Vec::with_capacity(path.segments.len());
    for segment in &path.segments {
        let piece = match segment {
            CamPathSegment2d::Line { to_mm } => Segment::Line {
                start: current,
                end: *to_mm,
            },
            CamPathSegment2d::Arc {
                to_mm,
                center_mm,
                clockwise,
            } => {
                let radius = distance(current, *center_mm);
                if radius <= LOOP_TOLERANCE_MM
                    || (distance(*to_mm, *center_mm) - radius).abs() > LOOP_TOLERANCE_MM
                {
                    return Err(refused());
                }
                Segment::Arc {
                    start: current,
                    end: *to_mm,
                    center: *center_mm,
                    ccw: !clockwise,
                }
            }
        };
        if current
            .into_iter()
            .chain(piece.end())
            .chain(match piece {
                Segment::Arc { center, .. } => center,
                Segment::Line { .. } => current,
            })
            .any(|value| !value.is_finite())
            || piece.length() <= LOOP_TOLERANCE_MM
        {
            return Err(refused());
        }
        current = piece.end();
        segments.push(piece);
    }
    if distance(current, path.start_mm) > LOOP_TOLERANCE_MM
        || crate::cam_offset::self_intersects(&segments)
    {
        return Err(refused());
    }
    Ok(segments)
}

fn loop_bounds<'a>(loops: impl IntoIterator<Item = &'a Vec<Segment>>) -> [Point; 2] {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for segment in loops.into_iter().flatten() {
        let [low, high] = segment.bounds();
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(low[axis]);
            maximum[axis] = maximum[axis].max(high[axis]);
        }
    }
    [minimum, maximum]
}

/// A lane height near `y` where crossings are unambiguous, nudged toward `toward`.
fn clear_lane(loops: &[Vec<Segment>], y: f64, toward: f64) -> f64 {
    let step = (toward - y).signum() * LOOP_TOLERANCE_MM * 0.1;
    let mut lane = y;
    for _ in 0..16 {
        if !degenerate_lane(loops, lane) {
            break;
        }
        lane += step;
    }
    lane
}

/// Even-odd containment of `point` in the area the loops bound together.
fn inside(loops: &[Vec<Segment>], point: Point) -> bool {
    let lane = clear_lane(loops, point[1], point[1] + 1.0);
    loops
        .iter()
        .flat_map(|segments| lane_crossings(segments, lane))
        .filter(|x| *x > point[0])
        .count()
        % 2
        == 1
}

fn on_or_inside(loops: &[Vec<Segment>], point: Point) -> bool {
    loops
        .iter()
        .flatten()
        .any(|segment| segment.distance_to(point) <= LOOP_TOLERANCE_MM)
        || inside(loops, point)
}

/// Whether a straight move between two allowed cutter centres stays allowed.
fn move_stays_inside(loops: &[Vec<Segment>], from: Point, to: Point) -> bool {
    let link = Segment::Line {
        start: from,
        end: to,
    };
    let mut stops: Vec<f64> = loops
        .iter()
        .flatten()
        .flat_map(|segment| intersections(&link, segment))
        .map(|point| distance(point, from))
        .chain([0.0, distance(from, to)])
        .collect();
    stops.sort_by(f64::total_cmp);
    let length = distance(from, to);
    stops.windows(2).all(|pair| {
        let t = (pair[0] + pair[1]) * 0.5 / length;
        on_or_inside(
            loops,
            [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t],
        )
    })
}

/// The loops the cutter centre may not cross when it keeps `clearance_mm` off
/// the region's walls and islands, each run with the wall on the cutter's right
/// (climb milling with a clockwise spindle).
fn pocket_boundaries(
    region: &CamRegion2d,
    clearance_mm: f64,
) -> Result<Vec<Vec<Segment>>, CamPlannerError> {
    let (outer, islands) = region.loops()?;
    // A clockwise outer loop shrinks when offset to its right.
    let mut boundaries = vec![reversed(&offset_right(&reversed(&outer), clearance_mm)?)];
    for island in &islands {
        boundaries.push(reversed(&offset_right(island, clearance_mm)?));
    }
    for (index, boundary) in boundaries.iter().enumerate() {
        if boundaries[index + 1..]
            .iter()
            .any(|other| loops_meet(boundary, other))
            || (index > 0 && !inside(&boundaries[..1], boundary[0].start()))
        {
            return Err(refused());
        }
    }
    Ok(boundaries)
}

/// Cutter-centre paths that clear `region` at one depth: zig-zag lanes
/// `stepover_mm` apart across the area the cutter centre may reach, then one
/// pass along every wall and round every island. Lanes are joined while the
/// joining move stays clear of the walls.
pub(crate) fn pocket_paths(
    region: &CamRegion2d,
    clearance_mm: f64,
    stepover_mm: f64,
) -> Result<Vec<CamPath2d>, CamPlannerError> {
    let boundaries = pocket_boundaries(region, clearance_mm)?;
    let [minimum, maximum] = loop_bounds(&boundaries[..1]);
    let span = maximum[1] - minimum[1];
    let lanes = (span / stepover_mm).ceil().max(1.0);
    if !lanes.is_finite() || lanes > MAX_POCKET_LANES as f64 {
        return Err(refused());
    }
    let lanes = lanes as usize;
    let middle = (minimum[1] + maximum[1]) * 0.5;
    let mut rows: Vec<(f64, Vec<[f64; 2]>)> = Vec::with_capacity(lanes + 1);
    for index in 0..=lanes {
        let nominal = minimum[1] + span * index as f64 / lanes as f64;
        let y = clear_lane(&boundaries, nominal, middle);
        let mut crossings: Vec<f64> = boundaries
            .iter()
            .flat_map(|segments| lane_crossings(segments, y))
            .collect();
        crossings.sort_by(f64::total_cmp);
        let spans = crossings
            .chunks_exact(2)
            .filter(|pair| pair[1] - pair[0] > LOOP_TOLERANCE_MM)
            .map(|pair| [pair[0], pair[1]])
            .collect();
        rows.push((y, spans));
    }
    let mut paths = zigzag(&boundaries, &mut rows);
    paths.extend(boundaries.iter().map(|boundary| path_of(boundary)));
    Ok(paths)
}

fn zigzag(boundaries: &[Vec<Segment>], rows: &mut [(f64, Vec<[f64; 2]>)]) -> Vec<CamPath2d> {
    let mut paths = Vec::new();
    for first_row in 0..rows.len() {
        while let Some(span) = rows[first_row].1.pop() {
            let y = rows[first_row].0;
            let mut path = CamPath2d {
                start_mm: [span[0], y],
                segments: vec![CamPathSegment2d::Line { to_mm: [span[1], y] }],
            };
            let mut at_right = true;
            let mut tail = [span[1], y];
            for (next_y, spans) in rows[first_row + 1..].iter_mut() {
                let side = usize::from(at_right);
                let found = spans
                    .iter()
                    .enumerate()
                    .filter(|(_, span)| {
                        move_stays_inside(boundaries, tail, [span[side], *next_y])
                    })
                    .min_by(|(_, left), (_, right)| {
                        (left[side] - tail[0])
                            .abs()
                            .total_cmp(&(right[side] - tail[0]).abs())
                    })
                    .map(|(index, _)| index);
                let Some(index) = found else { break };
                let span = spans.remove(index);
                let entry = [span[side], *next_y];
                let exit = [span[1 - side], *next_y];
                path.segments.push(CamPathSegment2d::Line { to_mm: entry });
                path.segments.push(CamPathSegment2d::Line { to_mm: exit });
                tail = exit;
                at_right = !at_right;
            }
            paths.push(path);
        }
    }
    paths
}

/// The closed cutter-centre loops of a contour `clearance_mm` off the region's
/// boundary on `side`, each run with the part on the cutter's right.
pub(crate) fn contour_paths(
    region: &CamRegion2d,
    side: CamContourSide,
    clearance_mm: f64,
) -> Result<Vec<CamPath2d>, CamPlannerError> {
    match side {
        CamContourSide::Outside => {
            let (outer, _) = region.loops()?;
            Ok(vec![path_of(&reversed(&offset_right(&outer, clearance_mm)?))])
        }
        CamContourSide::Inside => Ok(pocket_boundaries(region, clearance_mm)?
            .iter()
            .map(|boundary| path_of(boundary))
            .collect()),
    }
}

/// Bounding box of cutter-centre paths.
pub(crate) fn paths_bounds(paths: &[CamPath2d]) -> Result<[Point; 2], CamPlannerError> {
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for path in paths {
        let mut current = path.start_mm;
        for segment in &path.segments {
            let piece = match segment {
                CamPathSegment2d::Line { to_mm } => Segment::Line {
                    start: current,
                    end: *to_mm,
                },
                CamPathSegment2d::Arc {
                    to_mm,
                    center_mm,
                    clockwise,
                } => Segment::Arc {
                    start: current,
                    end: *to_mm,
                    center: *center_mm,
                    ccw: !clockwise,
                },
            };
            let [low, high] = piece.bounds();
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(low[axis]);
                maximum[axis] = maximum[axis].max(high[axis]);
            }
            current = piece.end();
        }
    }
    if minimum.iter().chain(&maximum).any(|value| !value.is_finite()) {
        return Err(refused());
    }
    Ok([minimum, maximum])
}
