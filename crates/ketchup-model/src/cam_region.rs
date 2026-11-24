//! Planar regions a CAM operation machines, read from an exact face of the
//! part, and the cutter-centre paths that clear or follow them.
use crate::cam::{
    CamOperation, CamPath2d, CamPathSegment2d, CamPlan, CamPlannerError, CamSetup, CamTool,
    CamToolKind, setup_bounds,
};
use crate::cam_offset::{
    LOOP_TOLERANCE_MM, Point, Segment, degenerate_lane, distance, intersections, lane_crossings,
    loops_meet, offset_right, oriented, reversed, signed_area,
};
use crate::exact_product::{ExactBRepGraphEdgeEvidence, ExactBRepGraphPackage};
use crate::tolerance::ROUNDING;
use ketchup_geometry::linalg::{cross, dot};

/// Most boundary segments one region holds.
const MAX_REGION_SEGMENTS: usize = 1_024;
/// Most scan lanes one pocket clears.
const MAX_POCKET_LANES: u32 = 50_000;
/// How far, in stepovers, pocket lanes stay off the wall pass. A stepover is
/// at most the cutter diameter, so the gap stays within the band the wall
/// pass sweeps.
const LANE_WALL_GAP_STEPOVERS: f64 = 0.1;

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
    /// The face must lie square to the setup's Z axis and be bounded by lines,
    /// circular arcs and whole circles; anything else is refused rather than
    /// approximated.
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
        if face.surface_kind != "plane" || dot(face.unit_normal, z_axis).abs() < 1.0 - ROUNDING {
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
        let mut pieces = Vec::new();
        let mut loops = Vec::new();
        for edge in package
            .edge_evidence
            .iter()
            .filter(|edge| edge.adjacent_face_ordinals.contains(&face_ordinal))
        {
            match boundary_piece(edge, to_setup)? {
                Piece::Open(segment) => pieces.push(segment),
                Piece::Circle(segments) => loops.push(segments),
            }
        }
        loops.extend(chain_pieces(pieces)?);
        if loops.is_empty() || loops.iter().map(Vec::len).sum::<usize>() > MAX_REGION_SEGMENTS {
            return Err(refused());
        }
        loops.sort_by(|left, right| signed_area(right).abs().total_cmp(&signed_area(left).abs()));
        let outer = oriented(&loops[0], true);
        let mut islands = Vec::with_capacity(loops.len() - 1);
        for island in &loops[1..] {
            if !inside(std::slice::from_ref(&outer), island[0].start()) {
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

/// A straight round hole along the setup's Z axis, read from an exact part.
#[derive(Clone, Debug, PartialEq)]
pub struct CamHole {
    /// Axis position in setup XY.
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
    /// Setup Z of the rim the drill enters through.
    pub top_z_mm: f64,
    /// Setup Z of the far rim: the floor of a blind hole, the exit of a through one.
    pub bottom_z_mm: f64,
    /// Whether the far rim is a hole in a flat face the drill comes out through,
    /// rather than the edge of the hole's own floor.
    pub through: bool,
    /// The included angle of the point a blind hole ends in below its far rim:
    /// 180 for a flat floor. `None` for a through hole.
    pub floor_point_angle_deg: Option<f64>,
}

impl CamHole {
    /// The hole whose wall is the cylindrical face `face_ordinal`. Its two
    /// rims must be whole circles square to the setup's Z axis, and the upper
    /// rim must be an island of the flat face around it; a round boss or
    /// anything else is refused.
    pub fn from_cylindrical_face(
        package: &ExactBRepGraphPackage,
        face_ordinal: u32,
        setup: &CamSetup,
    ) -> Result<Self, CamPlannerError> {
        let refused = || CamPlannerError::UnsupportedHole;
        let face = package
            .face_evidence
            .iter()
            .find(|face| face.face_ordinal == face_ordinal)
            .ok_or_else(refused)?;
        if face.surface_kind != "cylinder" {
            return Err(refused());
        }
        let z_axis = cross(setup.x_axis, setup.y_axis);
        let square = |direction: [f64; 3]| dot(direction, z_axis).abs() >= 1.0 - ROUNDING;
        let in_setup = |point: [f64; 3]| {
            let relative = [0, 1, 2].map(|axis| point[axis] - setup.origin_mm[axis]);
            [
                dot(relative, setup.x_axis),
                dot(relative, setup.y_axis),
                dot(relative, z_axis),
            ]
        };
        let mut rims = Vec::new();
        for edge in package
            .edge_evidence
            .iter()
            .filter(|edge| edge.adjacent_face_ordinals.contains(&face_ordinal))
        {
            match (edge.curve_kind.as_str(), edge.circle_radius_mm) {
                ("circle", Some(radius)) => {
                    let (Some(origin), Some(direction)) =
                        (edge.axis_origin_mm, edge.unit_axis_direction)
                    else {
                        return Err(refused());
                    };
                    if !edge.closed || !square(direction) {
                        return Err(refused());
                    }
                    rims.push((in_setup(origin), radius, edge));
                }
                // The seam where the cylinder closes on itself.
                ("line", None) => {}
                _ => return Err(refused()),
            }
        }
        if rims.len() != 2 {
            return Err(refused());
        }
        rims.sort_by(|left, right| left.0[2].total_cmp(&right.0[2]));
        let (lower, upper) = (&rims[0], &rims[1]);
        let center = [upper.0[0], upper.0[1]];
        if distance(center, [lower.0[0], lower.0[1]]) > LOOP_TOLERANCE_MM
            || (upper.1 - lower.1).abs() > LOOP_TOLERANCE_MM
            || upper.0[2] - lower.0[2] <= LOOP_TOLERANCE_MM
            || face.unit_axis_direction.is_some_and(|axis| !square(axis))
        {
            return Err(refused());
        }
        // Material lies outside a hole's wall, so its upper rim is a hole in
        // the flat face around it; a boss's upper rim bounds its own cap.
        // A rim on anything but a machinable flat face is not a drillable hole.
        if !rim_is_island_of_flat_face(package, face_ordinal, upper.2, center, upper.1, setup)? {
            return Err(refused());
        }
        // Likewise a far rim that is a hole in the flat face below is the exit of a
        // through hole; the edge of a flat or pointed floor is not.
        let through =
            rim_is_island_of_flat_face(package, face_ordinal, lower.2, center, lower.1, setup)?;
        let floor_point_angle_deg = if through {
            None
        } else {
            Some(floor_point_angle(
                package,
                face_ordinal,
                lower.2,
                lower.0[2],
                in_setup,
                square,
            )?)
        };
        Ok(Self {
            center_mm: center,
            radius_mm: upper.1,
            top_z_mm: upper.0[2],
            bottom_z_mm: lower.0[2],
            through,
            floor_point_angle_deg,
        })
    }
}

/// The included angle of the floor below `rim`, the far rim of the blind hole
/// wall `wall_ordinal` at setup Z `rim_z`: 180 for a flat floor, the cone's angle
/// for a floor that narrows to a point below the rim. A cone cut off short of its
/// point, or any other floor, is refused.
fn floor_point_angle(
    package: &ExactBRepGraphPackage,
    wall_ordinal: u32,
    rim: &ExactBRepGraphEdgeEvidence,
    rim_z: f64,
    in_setup: impl Fn([f64; 3]) -> [f64; 3],
    square: impl Fn([f64; 3]) -> bool,
) -> Result<f64, CamPlannerError> {
    let refused = || CamPlannerError::UnsupportedHole;
    let floor = rim
        .adjacent_face_ordinals
        .iter()
        .find(|ordinal| **ordinal != wall_ordinal)
        .and_then(|ordinal| {
            package
                .face_evidence
                .iter()
                .find(|face| face.face_ordinal == *ordinal)
        })
        .ok_or_else(refused)?;
    match (floor.surface_kind.as_str(), floor.cone) {
        ("plane", None) if square(floor.unit_normal) => Ok(180.0),
        // OCCT reports a cone's surface kind as "other"; its cone evidence names it.
        (_, Some(cone)) => {
            let rims = package
                .edge_evidence
                .iter()
                .filter(|edge| {
                    edge.adjacent_face_ordinals.contains(&floor.face_ordinal)
                        && edge.curve_kind == "circle"
                })
                .count();
            if rims != 1
                || !square(cone.unit_axis_direction)
                || in_setup(floor.centroid_mm)[2] >= rim_z - LOOP_TOLERANCE_MM
            {
                return Err(refused());
            }
            // The normal leans off the cone's side by the half angle's complement.
            let half = dot(cone.unit_normal, cone.unit_axis_direction)
                .abs()
                .clamp(0.0, 1.0)
                .asin();
            Ok(2.0 * half.to_degrees())
        }
        _ => Err(refused()),
    }
}

/// Whether `rim`, a circle of `radius` about `center` bounding the hole wall
/// `wall_ordinal`, is an island of the flat face on its other side. A rim on a face
/// that is not flat and square to the setup is no island.
fn rim_is_island_of_flat_face(
    package: &ExactBRepGraphPackage,
    wall_ordinal: u32,
    rim: &ExactBRepGraphEdgeEvidence,
    center: Point,
    radius: f64,
    setup: &CamSetup,
) -> Result<bool, CamPlannerError> {
    let Some(around) = rim
        .adjacent_face_ordinals
        .iter()
        .find(|ordinal| **ordinal != wall_ordinal)
    else {
        return Ok(false);
    };
    let region = match CamRegion2d::from_planar_face(package, *around, setup) {
        Ok(region) => region,
        Err(CamPlannerError::UnsupportedRegion) => return Ok(false),
        Err(other) => return Err(other),
    };
    Ok(region.islands.iter().any(|island| {
        (distance(island.start_mm, center) - radius).abs() <= LOOP_TOLERANCE_MM
            && island.segments.iter().all(|segment| {
                matches!(segment, CamPathSegment2d::Arc { center_mm, .. }
                    if distance(*center_mm, center) <= LOOP_TOLERANCE_MM)
            })
    }))
}

impl CamOperation {
    /// Whether the operation names a face of the target, which must be read
    /// from the evaluated part with [`Self::resolve_face`] before planning.
    #[must_use]
    pub const fn names_face(&self) -> bool {
        matches!(
            self,
            Self::FacePocket { .. } | Self::FaceContour { .. } | Self::FaceDrill { .. }
        )
    }

    /// The operation with its named face replaced by the region or hole read
    /// from `package`, the evaluated target of `plan`. Pockets and contours cut
    /// from the stock top down to their face; a pocket floor must face the
    /// tool. Operations that name no face come back unchanged.
    pub fn resolve_face(
        &self,
        package: &ExactBRepGraphPackage,
        plan: &CamPlan,
    ) -> Result<Self, CamPlannerError> {
        if !self.names_face() {
            return Ok(self.clone());
        }
        if package.graph.graph_digest != plan.target().exact_graph_digest {
            return Err(CamPlannerError::StaleToolpath);
        }
        let setup = plan.setup();
        let stock = plan.stock();
        let top_z_mm = setup_bounds([stock.minimum_mm, stock.maximum_mm], setup)[1][2];
        match self {
            Self::FacePocket { id, face_ordinal } => {
                let (region, floor_z, faces_tool) = planar_face(package, *face_ordinal, setup)?;
                if !faces_tool {
                    return Err(refused());
                }
                Ok(Self::RegionPocket {
                    id: *id,
                    region,
                    top_z_mm,
                    bottom_z_mm: floor_z,
                })
            }
            Self::FaceContour {
                id,
                face_ordinal,
                side,
            } => {
                let (region, face_z, _) = planar_face(package, *face_ordinal, setup)?;
                Ok(Self::RegionContour {
                    id: *id,
                    region,
                    side: *side,
                    top_z_mm,
                    bottom_z_mm: face_z,
                })
            }
            Self::FaceDrill { id, face_ordinal } => {
                let hole = CamHole::from_cylindrical_face(package, *face_ordinal, setup)?;
                Self::drill_hole(*id, &hole, plan.tool())
            }
            _ => Ok(self.clone()),
        }
    }

    /// Drills `hole` from its rim with `tool`, which must be a drill of the hole's
    /// diameter: through the far rim if the hole is through, else until its full
    /// diameter reaches the far rim (R1120ac-CAM-901). Its point then makes the
    /// floor, so it must be the floor's: flat for a flat floor, the cone's angle
    /// for a pointed one.
    pub fn drill_hole(id: u64, hole: &CamHole, tool: &CamTool) -> Result<Self, CamPlannerError> {
        if tool.kind != CamToolKind::Drill
            || (tool.diameter_mm * 0.5 - hole.radius_mm).abs() > LOOP_TOLERANCE_MM
        {
            return Err(CamPlannerError::UnsupportedHole);
        }
        let bottom_z_mm = match hole.floor_point_angle_deg {
            None => hole.bottom_z_mm,
            Some(angle) => {
                let floor = if angle >= 180.0 {
                    0.0
                } else {
                    hole.radius_mm / (angle.to_radians() * 0.5).tan()
                };
                let point = tool.tip_length_mm();
                if (point - floor).abs() > LOOP_TOLERANCE_MM {
                    return Err(CamPlannerError::DrillPointDoesNotFitFloor {
                        operation: id,
                        center: crate::cam::place_mm(hole.center_mm),
                    });
                }
                hole.bottom_z_mm - point
            }
        };
        Ok(Self::Drill {
            id,
            points_mm: vec![hole.center_mm],
            top_z_mm: hole.top_z_mm,
            bottom_z_mm,
            through: hole.through,
        })
    }
}

/// The region of planar face `face_ordinal`, its setup Z, and whether its
/// outward normal points at the tool.
fn planar_face(
    package: &ExactBRepGraphPackage,
    face_ordinal: u32,
    setup: &CamSetup,
) -> Result<(CamRegion2d, f64, bool), CamPlannerError> {
    let region = CamRegion2d::from_planar_face(package, face_ordinal, setup)?;
    let face = package
        .face_evidence
        .iter()
        .find(|face| face.face_ordinal == face_ordinal)
        .ok_or_else(refused)?;
    let z_axis = cross(setup.x_axis, setup.y_axis);
    let relative = [0, 1, 2].map(|axis| face.centroid_mm[axis] - setup.origin_mm[axis]);
    Ok((
        region,
        dot(relative, z_axis),
        dot(face.unit_normal, z_axis) > 0.0,
    ))
}

enum Piece {
    /// A line or partial arc, joined to its neighbours by [`chain_pieces`].
    Open(Segment),
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
            Ok(Piece::Open(Segment::Line {
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
        ("circle", false, Some(radius)) if radius.is_finite() && radius > LOOP_TOLERANCE_MM => {
            partial_arc(edge, origin, radius, &to_setup).map(Piece::Open)
        }
        // B-spline, ellipse and other curves are refused, not approximated.
        _ => Err(refused()),
    }
}

/// A partial circle edge as an arc in the setup plane. Its ends follow from
/// its exact length and centroid: the arc sweeps `length / radius`, and its
/// centroid lies on the bisector `radius * sin(h) / h` from the centre, `h`
/// being half the sweep. A centroid elsewhere is refused.
fn partial_arc(
    edge: &ExactBRepGraphEdgeEvidence,
    origin: [f64; 3],
    radius: f64,
    to_setup: impl Fn([f64; 3]) -> Point,
) -> Result<Segment, CamPlannerError> {
    let half = edge.length_mm / radius * 0.5;
    if !(half > 0.0 && half < std::f64::consts::PI) {
        return Err(refused());
    }
    let center = to_setup(origin);
    let centroid = to_setup(edge.centroid_mm);
    let offset = [centroid[0] - center[0], centroid[1] - center[1]];
    let reach = offset[0].hypot(offset[1]);
    if (reach - radius * half.sin() / half).abs() > LOOP_TOLERANCE_MM || reach <= LOOP_TOLERANCE_MM
    {
        return Err(refused());
    }
    let bisector = [offset[0] / reach, offset[1] / reach];
    let at = |angle: f64| {
        let (sin, cos) = angle.sin_cos();
        [
            center[0] + radius * (bisector[0] * cos - bisector[1] * sin),
            center[1] + radius * (bisector[0] * sin + bisector[1] * cos),
        ]
    };
    Ok(Segment::Arc {
        start: at(-half),
        end: at(half),
        center,
        ccw: true,
    })
}

/// Joins line and arc pieces end to end into closed loops.
fn chain_pieces(mut pieces: Vec<Segment>) -> Result<Vec<Vec<Segment>>, CamPlannerError> {
    let mut loops = Vec::new();
    while let Some(first) = pieces.pop() {
        let mut chain = vec![first];
        while distance(chain[chain.len() - 1].end(), chain[0].start()) > LOOP_TOLERANCE_MM {
            let tail = chain[chain.len() - 1].end();
            let index = pieces
                .iter()
                .position(|segment| {
                    distance(segment.start(), tail) <= LOOP_TOLERANCE_MM
                        || distance(segment.end(), tail) <= LOOP_TOLERANCE_MM
                })
                .ok_or_else(refused)?;
            let next = pieces.swap_remove(index);
            chain.push(if distance(next.start(), tail) <= LOOP_TOLERANCE_MM {
                next
            } else {
                next.reversed()
            });
        }
        if chain.len() < 2 || (chain.len() == 2 && chain.iter().all(is_line)) {
            return Err(refused());
        }
        // Snap so consecutive segments share one exact point; an arc's ends
        // stay where its circle puts them and the line beside it moves.
        for index in 0..chain.len() {
            let next = (index + 1) % chain.len();
            let joint = match (&chain[index], &chain[next]) {
                (_, Segment::Arc { start, .. }) => *start,
                (Segment::Arc { end, .. }, _) | (Segment::Line { end, .. }, _) => *end,
            };
            if let Segment::Line { end, .. } = &mut chain[index] {
                *end = joint;
            }
            if let Segment::Line { start, .. } = &mut chain[next] {
                *start = joint;
            }
        }
        loops.push(chain);
    }
    Ok(loops)
}

const fn is_line(segment: &Segment) -> bool {
    matches!(segment, Segment::Line { .. })
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
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ],
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
    let walls = pocket_boundaries(region, clearance_mm)?;
    // Lanes stop short of the walls, whose own pass finishes them: a lane
    // ending on a wall's offset leaves its cutter touching a curved wall
    // along a line, which the exact simulation cannot cut reliably.
    let boundaries =
        match pocket_boundaries(region, clearance_mm + stepover_mm * LANE_WALL_GAP_STEPOVERS) {
            Ok(inner) => inner,
            Err(CamPlannerError::UnsupportedRegion) => walls.clone(),
            Err(other) => return Err(other),
        };
    let [minimum, maximum] = loop_bounds(&boundaries[..1]);
    let span = maximum[1] - minimum[1];
    // The fewest lanes whose stepover covers the span.
    let lanes = (1..=MAX_POCKET_LANES)
        .find(|&lanes| f64::from(lanes) * stepover_mm >= span)
        .ok_or_else(refused)?;
    let middle = (minimum[1] + maximum[1]) * 0.5;
    let mut rows: Vec<(f64, Vec<[f64; 2]>)> = Vec::new();
    for index in 0..=lanes {
        let nominal = minimum[1] + span * f64::from(index) / f64::from(lanes);
        let y = clear_lane(&boundaries, nominal, middle);
        let mut crossings: Vec<f64> = boundaries
            .iter()
            .flat_map(|segments| lane_crossings(segments, y))
            .collect();
        crossings.sort_by(f64::total_cmp);
        let mut spans: Vec<[f64; 2]> = Vec::new();
        for pair in crossings.chunks_exact(2) {
            match spans.last_mut() {
                // A lane that only grazes an island, never deeper than the loop
                // tolerance, is one span and not two cuts a sliver apart.
                Some(last) if grazes(&boundaries, y, last[1], pair[0]) => last[1] = pair[1],
                _ => spans.push([pair[0], pair[1]]),
            }
        }
        spans.retain(|span| span[1] - span[0] > LOOP_TOLERANCE_MM);
        rows.push((y, spans));
    }
    let mut paths = zigzag(&boundaries, &mut rows);
    paths.extend(walls.iter().map(|boundary| path_of(boundary)));
    Ok(paths)
}

/// Whether the stretch of lane `y` from `from_x` to `to_x` stays within the
/// loop tolerance of the boundaries all the way.
fn grazes(boundaries: &[Vec<Segment>], y: f64, from_x: f64, to_x: f64) -> bool {
    (0..=8).all(|step| {
        let x = from_x + (to_x - from_x) * f64::from(step) / 8.0;
        boundaries
            .iter()
            .flatten()
            .any(|segment| segment.distance_to([x, y]) <= LOOP_TOLERANCE_MM)
    })
}

fn zigzag(boundaries: &[Vec<Segment>], rows: &mut [(f64, Vec<[f64; 2]>)]) -> Vec<CamPath2d> {
    let mut paths = Vec::new();
    for first_row in 0..rows.len() {
        while let Some(span) = rows[first_row].1.pop() {
            let y = rows[first_row].0;
            let mut path = CamPath2d {
                start_mm: [span[0], y],
                segments: vec![CamPathSegment2d::Line {
                    to_mm: [span[1], y],
                }],
            };
            let mut at_right = true;
            let mut tail = [span[1], y];
            for (next_y, spans) in rows[first_row + 1..].iter_mut() {
                let side = usize::from(at_right);
                let found = spans
                    .iter()
                    .enumerate()
                    .filter(|(_, span)| move_stays_inside(boundaries, tail, [span[side], *next_y]))
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
            Ok(vec![path_of(&reversed(&offset_right(
                &outer,
                clearance_mm,
            )?))])
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
    if minimum
        .iter()
        .chain(&maximum)
        .any(|value| !value.is_finite())
    {
        return Err(refused());
    }
    Ok([minimum, maximum])
}
