//! Project drawings of a model as a numbered set of sheets at one scale (see
//! [`drawing_set`]): a floor plan of each storey cut horizontally, a sheet of
//! the longitudinal and the cross section and a sheet of the four elevations,
//! and with windows or doors their schedule. The parts the cutting plane passes
//! through are filled and outlined heavily; each view carries its overall
//! dimensions.
//!
//! Each sheet is an ISO A format with a frame and a title block in the lower
//! right corner; the set is written as one vector PDF, and each sheet as a DXF
//! of its visible lines.
//!
//! The input is any set of closed triangulated solids in world millimetres (z up),
//! so the sheet follows whatever is visible: hiding a layer leaves it out.
//! Lines behind nearer faces are hidden by painting faces from far to near.

use crate::title_block::{
    SheetFormat, StoreyLevels, TITLE_BLOCK_MM, TitleValueDoesNotFit, drawing_area,
};
use ketchup_geometry::linalg::{Vec3, cross, cross2, dot, dot2, normalize_within, sub};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::tolerance::{HIDDEN_LINE_TOUCH_MM, NEGLIGIBLE, ROUNDING};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

mod annotations;
mod cutting;
mod levels;
mod openings;
mod plan_overlap;
mod set;
mod shop;
mod table;
mod threads;
mod visible;

use annotations::PlanAnnotations;
pub use annotations::{AnnotationCounts, annotation_marks};
pub use cutting::{CuttingPiece, CuttingSheet, CuttingStock, CuttingSummary};
use ketchup_model::annotation::Annotation;
pub use openings::{DrawingOpening, OpeningSchedule, ScheduleRow};
pub use set::{
    DrawingSet, DrawingSetOptions, PlanSheet, SetSheet, SheetKind, SheetTitles, drawing_set,
    drawing_set_until,
};
pub use shop::{ShopDrawings, ShopPosition, ShopSheet, ShopSummary};
pub use threads::DrawingThread;
use threads::ThreadMarks;

type Point = [f64; 3];
/// A closed outline in view millimetres.
type Polyline = Vec<[f64; 2]>;

/// One solid to draw: its triangles in world millimetres, the exact face each
/// triangle was meshed from, its colour and the threads on its cylinders.
#[derive(Clone, Debug)]
pub struct DrawingSolid {
    pub name: String,
    pub color: [u8; 3],
    pub triangles: Vec<[Point; 3]>,
    /// In step with `triangles`.
    pub faces: Vec<u32>,
    pub threads: Vec<DrawingThread>,
}

/// The views on the sheet, in layout order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectView {
    Plan,
    LongitudinalSection,
    CrossSection,
    Front,
    Back,
    Left,
    Right,
    /// A shop drawing: its parts seen square on, along their own z.
    Shop,
}

impl ProjectView {
    /// The views of the building, every one but [`Self::Shop`].
    pub const ALL: [Self; 7] = [
        Self::Plan,
        Self::LongitudinalSection,
        Self::CrossSection,
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
    ];

    #[must_use]
    pub const fn is_cut(self) -> bool {
        matches!(
            self,
            Self::Plan | Self::LongitudinalSection | Self::CrossSection
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewSummary {
    pub view: ProjectView,
    pub title: String,
    /// Solids the cutting plane passes through (zero for elevations).
    pub cut_solids: usize,
    /// Where the plane cuts, along the axis it is normal to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_at_mm: Option<f64>,
    /// Model extent of the view: width and height.
    pub extent_mm: [f64; 2],
    /// The level marks of a section or an elevation, lowest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
}

/// Why no sheet could be drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectDrawingError {
    /// There is nothing visible to draw.
    Empty,
    /// The views do not fit the format (the largest when none is chosen) at the
    /// chosen scale.
    DoesNotFit {
        scale: u32,
        format: Option<SheetFormat>,
    },
    /// A title block value would be cut short.
    TitleValueDoesNotFit(TitleValueDoesNotFit),
    /// The caller stopped the set before it was drawn.
    Cancelled,
}

impl From<TitleValueDoesNotFit> for ProjectDrawingError {
    fn from(error: TitleValueDoesNotFit) -> Self {
        Self::TitleValueDoesNotFit(error)
    }
}

impl std::fmt::Display for ProjectDrawingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => {
                formatter.write_str("there are no visible solids or annotations to draw")
            }
            Self::DoesNotFit { scale, format } => write!(
                formatter,
                "the views do not fit {} at 1:{scale}",
                format.map_or_else(
                    || "even an A0 sheet".to_owned(),
                    |format| format!("an {} sheet", format.name())
                )
            ),
            Self::TitleValueDoesNotFit(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("the drawing was cancelled"),
        }
    }
}

impl std::error::Error for ProjectDrawingError {}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Vec3::from(a).lerp(Vec3::from(b), t).into()
}

fn unit_normal([a, b, c]: &[Point; 3]) -> Option<Point> {
    normalize_within(cross(sub(*b, *a), sub(*c, *a)), NEGLIGIBLE)
}

/// A vertex position on a 0.01 mm grid, for matching shared edges.
fn grid_key(point: Point) -> [i64; 3] {
    #[allow(clippy::cast_possible_truncation)]
    point.map(|value| (value * 100.0).round() as i64)
}

/// The lines of one solid a drawing shows.
struct SolidEdges {
    /// Where two exact faces meet, however shallow the fold, and where the
    /// surface ends: drawn in every view.
    features: Vec<[Point; 2]>,
    /// Mesh lines inside one face, with the normals of the two facets they
    /// join: drawn only where the face turns away from the view, its outline.
    smooth: Vec<([Point; 2], [Point; 2])>,
}

/// The [`SolidEdges`] of `solid`. A mesh line inside one face is never drawn
/// as an edge, so a curved face does not show its facets whatever the mesh
/// step, but a round column still shows its sides (review 2026-10-19 MF-16).
fn solid_edges(solid: &DrawingSolid) -> SolidEdges {
    // Per mesh edge (keyed by its grid ends): its ends and the faces it
    // bounds, each with the normal of its facet.
    type FacetsOfEdge = (Point, Point, Vec<(u32, Point)>);
    let mut edges: BTreeMap<[[i64; 3]; 2], FacetsOfEdge> = BTreeMap::new();
    for (triangle, face) in solid.triangles.iter().zip(&solid.faces) {
        let Some(normal) = unit_normal(triangle) else {
            continue;
        };
        for index in 0..3 {
            let (a, b) = (triangle[index], triangle[(index + 1) % 3]);
            let (ka, kb) = (grid_key(a), grid_key(b));
            let key = if ka <= kb { [ka, kb] } else { [kb, ka] };
            edges
                .entry(key)
                .or_insert_with(|| (a, b, Vec::new()))
                .2
                .push((*face, normal));
        }
    }
    let mut lines = SolidEdges {
        features: Vec::new(),
        smooth: Vec::new(),
    };
    for (a, b, faces) in edges.into_values() {
        match faces.as_slice() {
            [(first, first_normal), (second, second_normal)] if first == second => {
                lines.smooth.push(([a, b], [*first_normal, *second_normal]));
            }
            _ => lines.features.push([a, b]),
        }
    }
    lines
}

/// The smooth mesh lines of `edges` on the outline of their face seen along
/// `depth`: one facet they join faces the viewer and the other does not.
fn outline_edges(edges: &SolidEdges, depth: Point) -> impl Iterator<Item = [Point; 2]> + '_ {
    edges
        .smooth
        .iter()
        .filter(move |(_, [first, second])| {
            (dot(*first, depth) < 0.0) != (dot(*second, depth) < 0.0)
        })
        .map(|(ends, _)| *ends)
}

#[derive(Clone, Copy)]
struct ViewFrame {
    right: Point,
    up: Point,
    /// Into the drawing, away from the viewer.
    depth: Point,
    /// Keeps only what lies at or beyond this depth.
    cut: Option<f64>,
}

impl ViewFrame {
    fn project(&self, point: Point) -> [f64; 2] {
        [dot(point, self.right), dot(point, self.up)]
    }
}

/// The part of a polygon where `dot(point, direction) >= offset`.
fn keep_beyond(points: &[Point], direction: Point, offset: f64) -> Vec<Point> {
    let distance = |point: Point| dot(point, direction) - offset;
    let mut kept = Vec::with_capacity(points.len() + 1);
    for index in 0..points.len() {
        let (from, to) = (points[index], points[(index + 1) % points.len()]);
        let (df, dt) = (distance(from), distance(to));
        if df >= 0.0 {
            kept.push(from);
        }
        if (df >= 0.0) != (dt >= 0.0) {
            kept.push(lerp(from, to, df / (df - dt)));
        }
    }
    kept
}

/// The part of a polygon at or beyond the cut.
fn clip_polygon(frame: &ViewFrame, points: &[Point]) -> Vec<Point> {
    match frame.cut {
        Some(cut) => keep_beyond(points, frame.depth, cut),
        None => points.to_vec(),
    }
}

/// Depth slab size for sorting. Painting far to near orders whole polygons, so a
/// sloped face is cut into thin slabs: otherwise a long roof layer, whose middle
/// lies deeper than that of a shorter layer under it, would be painted first and
/// the layer under it would show through.
const DEPTH_SLAB_MM: f64 = 40.0;

fn slab_bounds(low: f64, high: f64) -> impl Iterator<Item = f64> {
    #[allow(clippy::cast_possible_truncation)]
    let (first, last) = (
        (low / DEPTH_SLAB_MM).floor() as i64 + 1,
        (high / DEPTH_SLAB_MM).ceil() as i64 - 1,
    );
    #[allow(clippy::cast_precision_loss)]
    (first..=last).map(|index| index as f64 * DEPTH_SLAB_MM)
}

/// A polygon cut into depth slabs, nearest last.
fn depth_slabs(frame: &ViewFrame, points: Vec<Point>) -> Vec<Vec<Point>> {
    let depths = points.iter().map(|point| dot(*point, frame.depth));
    let low = depths.clone().fold(f64::INFINITY, f64::min);
    let high = depths.fold(f64::NEG_INFINITY, f64::max);
    let toward = frame.depth.map(|value| -value);
    let mut slabs = Vec::new();
    let mut rest = points;
    for bound in slab_bounds(low, high).collect::<Vec<_>>().into_iter().rev() {
        let far = keep_beyond(&rest, frame.depth, bound);
        rest = keep_beyond(&rest, toward, -bound);
        if far.len() >= 3 {
            slabs.push(far);
        }
    }
    if rest.len() >= 3 {
        slabs.push(rest);
    }
    slabs
}

/// A segment cut at the same depth slabs.
fn segment_slabs(frame: &ViewFrame, [a, b]: [Point; 2]) -> Vec<[Point; 2]> {
    let (da, db) = (dot(a, frame.depth), dot(b, frame.depth));
    let mut cuts = vec![0.0];
    if (db - da).abs() > ROUNDING {
        cuts.extend(slab_bounds(da.min(db), da.max(db)).map(|bound| (bound - da) / (db - da)));
    }
    cuts.push(1.0);
    cuts.sort_by(f64::total_cmp);
    cuts.windows(2)
        .map(|pair| [lerp(a, b, pair[0]), lerp(a, b, pair[1])])
        .collect()
}

/// Whether the solid's triangles wind clockwise seen from outside (as after a
/// mirroring placement), so their normals point inward. The volume is summed
/// from a corner of the solid, not the world origin: a model in national
/// coordinates (1e9 mm away) would sum terms of 1e27 and lose its sign
/// (review 2026-11-07 r26 MF-240).
fn winds_inward(solid: &DrawingSolid) -> bool {
    let Some(&[origin, _, _]) = solid.triangles.first() else {
        return false;
    };
    let volume: f64 = solid
        .triangles
        .iter()
        .map(|[a, b, c]| dot(sub(*a, origin), cross(sub(*b, origin), sub(*c, origin))))
        .sum();
    volume < 0.0
}

fn clip_segment(frame: &ViewFrame, [a, b]: [Point; 2]) -> Option<[Point; 2]> {
    let Some(cut) = frame.cut else {
        return Some([a, b]);
    };
    let (da, db) = (dot(a, frame.depth) - cut, dot(b, frame.depth) - cut);
    match (da >= 0.0, db >= 0.0) {
        (true, true) => Some([a, b]),
        (false, false) => None,
        (true, false) => Some([a, lerp(a, b, da / (da - db))]),
        (false, true) => Some([lerp(a, b, da / (da - db)), b]),
    }
}

/// Where the cutting plane crosses a solid: closed loops in view coordinates.
fn cut_loops(frame: &ViewFrame, solid: &DrawingSolid) -> Vec<Vec<[f64; 2]>> {
    let Some(cut) = frame.cut else {
        return Vec::new();
    };
    let mut segments = Vec::new();
    for triangle in &solid.triangles {
        let distances = triangle.map(|point| dot(point, frame.depth) - cut);
        let mut crossing = Vec::with_capacity(2);
        for index in 0..3 {
            let next = (index + 1) % 3;
            let (da, db) = (distances[index], distances[next]);
            if (da >= 0.0) != (db >= 0.0) {
                crossing.push(frame.project(lerp(triangle[index], triangle[next], da / (da - db))));
            }
        }
        if let [a, b] = crossing.as_slice() {
            segments.push([*a, *b]);
        }
    }
    chain_loops(segments)
}

fn key_2d(point: [f64; 2]) -> [i64; 2] {
    #[allow(clippy::cast_possible_truncation)]
    point.map(|value| (value * 100.0).round() as i64)
}

/// Joins segments that share endpoints into polylines, closed where they meet.
fn chain_loops(segments: Vec<[[f64; 2]; 2]>) -> Vec<Vec<[f64; 2]>> {
    let mut by_start: BTreeMap<[i64; 2], Vec<usize>> = BTreeMap::new();
    for (index, segment) in segments.iter().enumerate() {
        by_start.entry(key_2d(segment[0])).or_default().push(index);
        by_start.entry(key_2d(segment[1])).or_default().push(index);
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    for first in 0..segments.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut polyline = vec![segments[first][0], segments[first][1]];
        loop {
            let end = key_2d(*polyline.last().expect("a polyline has points"));
            let next = by_start.get(&end).and_then(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .find(|candidate| !used[*candidate])
            });
            let Some(next) = next else {
                break;
            };
            used[next] = true;
            let [a, b] = segments[next];
            polyline.push(if key_2d(a) == end { b } else { a });
        }
        if polyline.len() >= 3 {
            loops.push(without_straight_corners(polyline));
        }
    }
    loops
}

/// `polyline` without the points it runs straight through (a cut crosses a
/// flat face in one piece per triangle); a closed one, ending where it
/// starts, stays closed and may start at another corner.
fn without_straight_corners(mut polyline: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let closed = polyline.len() > 3 && key_2d(polyline[0]) == key_2d(polyline[polyline.len() - 1]);
    if closed {
        polyline.pop();
    }
    let straight = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let (ab, bc) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        let length = ab[0].hypot(ab[1]) + bc[0].hypot(bc[1]);
        length > 0.0 && cross2(ab, bc).abs() <= HIDDEN_LINE_TOUCH_MM * length && dot2(ab, bc) >= 0.0
    };
    let mut changed = true;
    while changed && polyline.len() > 3 {
        changed = false;
        let count = polyline.len();
        let ends = if closed { 0..count } else { 1..count - 1 };
        for index in ends {
            let (before, after) = ((index + count - 1) % count, (index + 1) % count);
            if straight(polyline[before], polyline[index], polyline[after]) {
                polyline.remove(index);
                changed = true;
                break;
            }
        }
    }
    if closed {
        polyline.push(polyline[0]);
    }
    polyline
}

enum Item {
    Face {
        points: Vec<[f64; 2]>,
        color: [u8; 3],
    },
    Edge([[f64; 2]; 2]),
    /// A thread line by ISO 6410, with its own stroke width.
    Thread {
        points: Vec<[f64; 2]>,
        width_mm: f64,
    },
}

struct DrawnView {
    view: ProjectView,
    /// Far-to-near faces and edges.
    items: Vec<Item>,
    /// Cut fill per solid: loops and colour.
    cuts: Vec<(Vec<Polyline>, [u8; 3])>,
    /// Thread lines on the cut, drawn over its fill.
    on_cut: Vec<Item>,
    /// What the view draws, annotations included.
    min: [f64; 2],
    max: [f64; 2],
    /// The extent of the solids alone, which the overall dimensions measure.
    solids: Option<([f64; 2], [f64; 2])>,
    annotations: Option<PlanAnnotations>,
    /// The underlay lines a plan draws beneath the model, cut at its extent.
    underlay: Vec<[[f64; 2]; 2]>,
    /// Boxed marks where they stand: the windows and doors a plan shows, the
    /// position numbers of a shop drawing.
    marks: Vec<([f64; 2], String)>,
    /// The level marks right of a section or an elevation: view height and text.
    levels: Vec<(f64, String)>,
    cut_at_mm: Option<f64>,
}

impl DrawnView {
    /// Draws `annotations` in this view and grows it to hold them.
    fn annotate(&mut self, annotations: PlanAnnotations) {
        for point in annotations.points() {
            for (axis, value) in point.into_iter().enumerate() {
                self.min[axis] = self.min[axis].min(value);
                self.max[axis] = self.max[axis].max(value);
            }
        }
        self.annotations = Some(annotations);
    }

    /// Lays the underlay `lines` beneath this view, cut at what it draws: an
    /// underlay as large as a cadastral map never grows the view.
    fn lay_underlay(&mut self, frame: &ViewFrame, lines: &[[Point; 2]]) {
        if !self.min[0].is_finite() {
            return;
        }
        self.underlay = lines
            .iter()
            .filter_map(|[a, b]| {
                clip_to_box(frame.project(*a), frame.project(*b), self.min, self.max)
            })
            .collect();
    }

    /// Marks the windows and doors standing at `points` in this view.
    fn mark_openings(&mut self, frame: &ViewFrame, points: &[(String, Point)]) {
        self.marks = points
            .iter()
            .map(|(mark, point)| (frame.project(*point), mark.clone()))
            .collect();
    }
}

fn shade(color: [u8; 3], facing: f64) -> [u8; 3] {
    // A light, drawing-like tone: the part colour washed toward white,
    // darker for faces seen at a glancing angle.
    let light = 0.55 + 0.25 * facing.abs();
    color.map(|channel| {
        let value = f64::from(channel) * (1.0 - light) + 255.0 * light;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            value.round().clamp(0.0, 255.0) as u8
        }
    })
}

fn draw_view(
    view: ProjectView,
    frame: ViewFrame,
    solids: &[DrawingSolid],
    edges: &[SolidEdges],
) -> DrawnView {
    let mut sorted: Vec<(f64, Item)> = Vec::new();
    let mut cuts = Vec::new();
    let mut threads = ThreadMarks::default();
    for (solid, solid_edges) in solids.iter().zip(edges) {
        for thread in &solid.threads {
            threads.add(&frame, thread);
        }
        let inward = winds_inward(solid);
        for triangle in &solid.triangles {
            let Some(normal) = unit_normal(triangle) else {
                continue;
            };
            let normal = if inward {
                normal.map(|value| -value)
            } else {
                normal
            };
            // A face turned away is hidden by the solid's own near faces, unless
            // the cut has opened the solid; the cut face is filled over it then.
            if frame.cut.is_none() && dot(normal, frame.depth) > ROUNDING {
                continue;
            }
            let kept = clip_polygon(&frame, triangle);
            if kept.len() < 3 {
                continue;
            }
            let color = shade(solid.color, dot(normal, frame.depth));
            for slab in depth_slabs(&frame, kept) {
                let projected = slab
                    .iter()
                    .map(|point| frame.project(*point))
                    .collect::<Vec<_>>();
                // not a tolerance: a sliver under 0.001 mm² of model area draws nothing visible.
                if polygon_area(&projected).abs() < 1e-3 {
                    continue;
                }
                #[allow(clippy::cast_precision_loss)]
                let depth = slab
                    .iter()
                    .map(|point| dot(*point, frame.depth))
                    .sum::<f64>()
                    / slab.len() as f64;
                sorted.push((
                    depth,
                    Item::Face {
                        points: projected,
                        color,
                    },
                ));
            }
        }
        let outline = outline_edges(solid_edges, frame.depth);
        for edge in solid_edges.features.iter().copied().chain(outline) {
            let Some(edge) = clip_segment(&frame, edge) else {
                continue;
            };
            let (pa, pb) = (frame.project(edge[0]), frame.project(edge[1]));
            if (pa[0] - pb[0]).hypot(pa[1] - pb[1]) < 0.5 {
                continue;
            }
            for [a, b] in segment_slabs(&frame, edge) {
                // Just in front of the faces it bounds, so they do not paint over it.
                let depth = (dot(a, frame.depth) + dot(b, frame.depth)) / 2.0 - 1.0;
                sorted.push((depth, Item::Edge([frame.project(a), frame.project(b)])));
            }
        }

        let loops = cut_loops(&frame, solid);
        if !loops.is_empty() {
            cuts.push((loops, solid.color));
        }
    }
    sorted.extend(threads.sorted);
    sorted.sort_by(|left, right| right.0.total_cmp(&left.0));
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut grow = |point: &[f64; 2]| {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    };
    for (_, item) in &sorted {
        match item {
            Item::Face { points, .. } => points.iter().for_each(&mut grow),
            Item::Edge(points) => points.iter().for_each(&mut grow),
            Item::Thread { points, .. } => points.iter().for_each(&mut grow),
        }
    }
    DrawnView {
        view,
        items: sorted.into_iter().map(|(_, item)| item).collect(),
        cuts,
        on_cut: threads.on_cut,
        min,
        max,
        solids: min[0].is_finite().then_some((min, max)),
        annotations: None,
        underlay: Vec::new(),
        marks: Vec::new(),
        levels: Vec::new(),
        // `cut` is measured along the depth direction, which is ±1 on the
        // world axis the plane is normal to: divide to get that coordinate.
        cut_at_mm: frame.cut.map(|cut| {
            let axis = (0..3)
                .max_by(|&a, &b| frame.depth[a].abs().total_cmp(&frame.depth[b].abs()))
                .unwrap_or(2);
            cut / frame.depth[axis]
        }),
    }
}

/// The piece of the line from `a` to `b` inside the box `min`..`max`, if any
/// (Liang–Barsky).
fn clip_to_box(a: [f64; 2], b: [f64; 2], min: [f64; 2], max: [f64; 2]) -> Option<[[f64; 2]; 2]> {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let (mut enter, mut leave) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        for (toward, room) in [
            (-delta[axis], a[axis] - min[axis]),
            (delta[axis], max[axis] - a[axis]),
        ] {
            if toward.abs() > 0.0 {
                let at = room / toward;
                if toward < 0.0 {
                    enter = enter.max(at);
                } else {
                    leave = leave.min(at);
                }
            } else if room < 0.0 {
                return None;
            }
        }
    }
    let point = |t: f64| [a[0] + delta[0] * t, a[1] + delta[1] * t];
    (enter < leave).then(|| [point(enter), point(leave)])
}

/// The signed area, measured from the polygon's first corner so that it keeps
/// its precision far from the origin (review 2026-11-07 r26 MF-240).
fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let Some(&origin) = points.first() else {
        return 0.0;
    };
    let local = |point: [f64; 2]| [point[0] - origin[0], point[1] - origin[1]];
    (0..points.len())
        .map(|index| {
            let (a, b) = (
                local(points[index]),
                local(points[(index + 1) % points.len()]),
            );
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// The frames of the seven views around the model's bounds. The longer
/// horizontal side is the length: the longitudinal section runs along it;
/// `x_right` keeps x to the right in every case, as a plan of its own shows it.
fn view_frames(
    min: Point,
    max: Point,
    plan_cut_z: f64,
    x_right: bool,
) -> [(ProjectView, ViewFrame); 7] {
    let centre = lerp(min, max, 0.5);
    let along_x = x_right || max[0] - min[0] >= max[1] - min[1];
    // `length` runs along the model, `width` across it, seen from the front.
    let (length, width) = if along_x {
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
    } else {
        ([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0])
    };
    let up = [0.0, 0.0, 1.0];
    let neg = |v: Point| v.map(|value| -value);
    let frame = |right, up, depth, cut| ViewFrame {
        right,
        up,
        depth,
        cut,
    };
    [
        (
            ProjectView::Plan,
            frame(length, width, neg(up), Some(-plan_cut_z)),
        ),
        (
            ProjectView::LongitudinalSection,
            frame(length, up, width, Some(dot(centre, width))),
        ),
        (
            ProjectView::CrossSection,
            frame(neg(width), up, length, Some(dot(centre, length))),
        ),
        (ProjectView::Front, frame(length, up, width, None)),
        (ProjectView::Back, frame(neg(length), up, neg(width), None)),
        (ProjectView::Left, frame(neg(width), up, length, None)),
        (ProjectView::Right, frame(width, up, neg(length), None)),
    ]
}

/// Share of the largest horizontal level a level needs to be the floor (not a
/// tolerance: a ratio of areas).
const FLOOR_SHARE_OF_LARGEST_LEVEL: f64 = 0.5;
/// The narrowest a level a person stands on is, a gallery or a landing, as
/// narrow as a side flight of steps may be; the top of a wall or a
/// parapet is narrower (not a tolerance: a clear width, review 2026-11-04 r19
/// P3: a landing 900 mm wide went unmarked).
const STANDING_WIDTH_MM: f64 = 750.0;
/// How high a raised level is at least over the level below to be one a
/// person stands on, a landing halfway up a storey or a gallery; a bed, a
/// table, a kitchen island or a bar counter is lower (not a tolerance: a
/// height, review 2026-11-04 r19 MF-161).
const RAISED_LEVEL_MM: f64 = 1200.0;
/// Share of a top a level under it covers in plan to be the level it stands
/// over, as a gallery under a table past its edge or a bed over its opening is
/// (not a tolerance: a ratio of areas, review 2026-11-07 P3).
const UNDER_SHARE_OF_TOP: f64 = 0.25;
/// How far below z = 0 a ground floor modelled under it lies at most; the
/// floor of a basement is a storey below (not a tolerance: a depth, review
/// 2026-11-06 P3).
const SUNKEN_GROUND_FLOOR_MM: f64 = 1000.0;
/// The widest joint between the strips of one floor, a deck of
/// planks laid apart (not a tolerance: a gap, review 2026-11-06 P3).
const FACE_JOINT_MM: f64 = 20.0;
/// Room around a view for its dimensions and title, in page millimetres.
const VIEW_PAD_MM: f64 = 22.0;
pub const SCALES: [u32; 10] = [10, 20, 25, 50, 75, 100, 200, 250, 500, 1000];

// not a tolerance: the pen width of a visible edge on the sheet.
const EDGE_STROKE_MM: f64 = 0.18;
const EDGE_COLOR: [u8; 3] = [34, 34, 34];
// not a tolerance: the pen width of an underlay line, thinner than an edge.
const UNDERLAY_STROKE_MM: f64 = 0.13;
const UNDERLAY_COLOR: [u8; 3] = [160, 160, 160];
// not a tolerance: the pen width of a cut edge on the sheet.
const CUT_STROKE_MM: f64 = 0.5;
const DIMENSION_TEXT_MM: f64 = 2.5;
const VIEW_TITLE_MM: f64 = 4.0;

/// The views of one sheet and the rows they are laid out in, top to bottom,
/// each left to right (indices into `views`).
struct SheetViews {
    views: Vec<DrawnView>,
    rows: Vec<Vec<usize>>,
}

fn extent(view: &DrawnView) -> [f64; 2] {
    [view.max[0] - view.min[0], view.max[1] - view.min[1]]
}

/// The finest standard scale at which every row fits the drawing area of
/// `format` above the title block, if any does.
fn choose_scale(sheet: &SheetViews, format: SheetFormat) -> Option<u32> {
    SCALES.into_iter().find(|scale| fits(sheet, format, *scale))
}

/// Whether every row fits the drawing area of `format` at 1:`scale`.
fn fits(SheetViews { views, rows }: &SheetViews, format: SheetFormat, scale: u32) -> bool {
    let [[left, top], [right, bottom]] = drawing_area(format);
    let usable_width = right - left;
    let usable_height = bottom - top - TITLE_BLOCK_MM[1];
    let scale = f64::from(scale);
    let widths_fit = rows.iter().all(|row| {
        row.iter()
            .map(|index| extent(&views[*index])[0] / scale + 2.0 * VIEW_PAD_MM)
            .sum::<f64>()
            <= usable_width
    });
    let height = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|index| extent(&views[*index])[1] / scale + 2.0 * VIEW_PAD_MM)
                .fold(0.0, f64::max)
        })
        .sum::<f64>();
    widths_fit && height <= usable_height
}

/// The scale of the requested format, or of the smallest format that holds the
/// views at `preferred` or finer (A0 at whatever fits when none does).
fn choose_layout_scale(sheet: &SheetViews, requested: Option<SheetFormat>, preferred: u32) -> u32 {
    let coarsest = SCALES[SCALES.len() - 1];
    if let Some(format) = requested {
        return choose_scale(sheet, format).unwrap_or(coarsest);
    }
    SheetFormat::ALL
        .into_iter()
        .find_map(|format| choose_scale(sheet, format).filter(|scale| *scale <= preferred))
        .unwrap_or_else(|| choose_scale(sheet, SheetFormat::A0).unwrap_or(coarsest))
}

/// The requested format, or the smallest that holds the views at the fixed
/// `scale`; never a sheet the views run off.
fn format_at_scale(
    sheet: &SheetViews,
    requested: Option<SheetFormat>,
    scale: u32,
) -> Result<SheetFormat, ProjectDrawingError> {
    requested
        .map_or_else(|| SheetFormat::ALL.to_vec(), |format| vec![format])
        .into_iter()
        .find(|format| fits(sheet, *format, scale))
        .ok_or(ProjectDrawingError::DoesNotFit {
            scale,
            format: requested,
        })
}

/// A short oblique tick across a dimension line end, as architects draw it.
fn tick(page: &mut Page, [x, y]: [f64; 2]) {
    page.line(
        vec![[x - 1.2, y + 1.2], [x + 1.2, y - 1.2]],
        Stroke::solid(0.35),
    );
}

/// Draws one view with its lower-left model corner at `origin` (page mm):
/// painted far to near, or with `visible` its visible lines alone (no face
/// fill), as a DXF carries it.
fn draw_view_on_page(
    page: &mut Page,
    view: &DrawnView,
    title: &str,
    origin: [f64; 2],
    scale: f64,
    visible: Option<&[visible::VisibleLine]>,
) {
    let at = |point: [f64; 2]| {
        [
            origin[0] + (point[0] - view.min[0]) / scale,
            origin[1] - (point[1] - view.min[1]) / scale,
        ]
    };
    let edge = Stroke {
        width_mm: EDGE_STROKE_MM,
        color: EDGE_COLOR,
        dash_mm: Vec::new(),
    };
    // First, so the model is drawn over it.
    for [a, b] in &view.underlay {
        page.line(
            vec![at(*a), at(*b)],
            Stroke {
                width_mm: UNDERLAY_STROKE_MM,
                color: UNDERLAY_COLOR,
                dash_mm: Vec::new(),
            },
        );
    }
    let thread_line = |page: &mut Page, points: &[[f64; 2]], width_mm: f64| {
        page.line(
            points.iter().map(|point| at(*point)).collect(),
            Stroke {
                width_mm,
                color: EDGE_COLOR,
                dash_mm: Vec::new(),
            },
        );
    };
    if let Some(lines) = visible {
        for line in lines {
            thread_line(page, &line.points, line.width_mm);
        }
    }
    for item in view.items.iter().filter(|_| visible.is_none()) {
        match item {
            Item::Face { points, color } => page.marks.push(Mark::Fill {
                loops: vec![points.iter().map(|point| at(*point)).collect()],
                color: *color,
                // A hairline of the face colour closes the seams between pieces.
                outline: Some(Stroke {
                    width_mm: 0.05,
                    color: *color,
                    dash_mm: Vec::new(),
                }),
            }),
            Item::Edge([a, b]) => page.line(vec![at(*a), at(*b)], edge.clone()),
            Item::Thread { points, width_mm } => thread_line(page, points, *width_mm),
        }
    }
    for (loops, color) in &view.cuts {
        page.marks.push(Mark::Fill {
            loops: loops
                .iter()
                .map(|polyline| polyline.iter().map(|point| at(*point)).collect())
                .collect(),
            color: color.map(|channel| channel / 2),
            outline: Some(Stroke::solid(CUT_STROKE_MM)),
        });
    }
    for item in view.on_cut.iter().filter(|_| visible.is_none()) {
        if let Item::Thread { points, width_mm } = item {
            thread_line(page, points, *width_mm);
        }
    }
    if let Some(annotations) = &view.annotations {
        annotations.draw(page, at);
    }
    for (point, mark) in &view.marks {
        openings::draw_mark(page, at(*point), mark, visible.is_none());
    }
    let levels = view
        .levels
        .iter()
        .map(|(height, text)| (at([view.min[0], *height])[1], text.clone()))
        .collect::<Vec<_>>();
    levels::draw_levels(page, at(view.max)[0], &levels);
    let [outer_left, outer_bottom] = at(view.min);
    // Overall width below the view and overall height on its left, outside
    // whatever the view draws.
    let below = outer_bottom + 8.0;
    if let Some((low, high)) = view.solids {
        overall_dimensions(
            page,
            [at(low), at(high)],
            [high[0] - low[0], high[1] - low[1]],
            [below, outer_left - 8.0],
        );
    }
    page.marks.push(Mark::Text {
        at: [outer_left, below + 8.0],
        text: title.to_owned(),
        height_mm: VIEW_TITLE_MM,
        anchor: Anchor::Start,
        angle_deg: 0.0,
        bold: true,
    });
}

/// The overall width of the solids at `below` and their height at `aside`, from
/// their lower-left and upper-right corners on the page; `size` is in model
/// millimetres.
fn overall_dimensions(
    page: &mut Page,
    [[left, bottom], [right, top]]: [[f64; 2]; 2],
    [width, height]: [f64; 2],
    [below, aside]: [f64; 2],
) {
    let thin = Stroke::solid(EDGE_STROKE_MM);
    page.line(
        vec![[left - 2.0, below], [right + 2.0, below]],
        thin.clone(),
    );
    for x in [left, right] {
        page.line(vec![[x, bottom + 2.0], [x, below + 2.0]], thin.clone());
        tick(page, [x, below]);
    }
    page.text(
        [f64::midpoint(left, right), below - 1.0],
        &whole_mm(width),
        DIMENSION_TEXT_MM,
        Anchor::Middle,
    );
    page.line(
        vec![[aside, bottom + 2.0], [aside, top - 2.0]],
        thin.clone(),
    );
    for y in [bottom, top] {
        page.line(vec![[aside - 2.0, y], [left - 2.0, y]], thin.clone());
        tick(page, [aside, y]);
    }
    page.marks.push(Mark::Text {
        at: [aside - 1.0, f64::midpoint(bottom, top)],
        text: whole_mm(height),
        height_mm: DIMENSION_TEXT_MM,
        anchor: Anchor::Middle,
        angle_deg: 90.0,
        bold: false,
    });
}

fn bounds(solids: &[DrawingSolid]) -> Option<(Point, Point)> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in solids
        .iter()
        .flat_map(|solid| solid.triangles.iter().flatten())
    {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    min.iter()
        .chain(&max)
        .all(|value| value.is_finite())
        .then_some((min, max))
}

/// An upward-facing horizontal level of the model.
#[derive(Default)]
struct UpwardLevel {
    /// Of all the solids' faces at the level.
    area: f64,
    /// Of the widest group of faces there that touch along an edge: the
    /// short side of the rectangle with their area and the length of their
    /// outline, their clear width.
    widest_mm: f64,
    /// Where in plan: the triangles of its faces, counter-clockwise, not the
    /// boxes around them, so an L-shaped gallery is not over the landing in
    /// its free corner (review 2026-11-06 r23 MF-200).
    triangles: Vec<[[f64; 2]; 3]>,
}

impl UpwardLevel {
    /// How much of the level lies over `other` in plan; None once `stop` is set.
    fn area_over(&self, other: &Self, stop: &AtomicBool) -> Option<f64> {
        plan_overlap::area_over(&self.triangles, &other.triangles, stop)
    }
}

/// One solid's upward faces at one level: their area, the edges of their
/// outline and their triangles in plan.
struct LevelPiece {
    area: f64,
    outline: Vec<[[f64; 2]; 2]>,
    triangles: Vec<[[f64; 2]; 3]>,
}

/// One solid's upward faces at one level as they are gathered: their area,
/// each mesh edge with how many facets share it and the triangles in plan.
type Faces = (
    f64,
    BTreeMap<[[i64; 3]; 2], (usize, [[f64; 2]; 2])>,
    Vec<[[f64; 2]; 3]>,
);

/// The upward-facing horizontal levels of `solids` by their height, lowest
/// first.
fn upward_levels<'a>(
    solids: impl IntoIterator<Item = &'a DrawingSolid>,
) -> Vec<(f64, UpwardLevel)> {
    let mut pieces = BTreeMap::<i64, Vec<LevelPiece>>::new();
    for solid in solids {
        let outward = if winds_inward(solid) { -1.0 } else { 1.0 };
        // Per level: the area, each mesh edge with how many facets share it
        // and the triangles in plan.
        let mut faces = BTreeMap::<i64, Faces>::new();
        for triangle in &solid.triangles {
            let [a, b, c] = triangle;
            let Some(normal) = unit_normal(triangle) else {
                continue;
            };
            if normal[2] * outward < 1.0 - ROUNDING {
                continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let level = ((a[2] + b[2] + c[2]) / 3.0).round() as i64;
            let (area, edges, triangles) = faces.entry(level).or_default();
            let twice = cross(sub(*b, *a), sub(*c, *a))[2];
            *area += twice.abs() / 2.0;
            let mut plan = [[a[0], a[1]], [b[0], b[1]], [c[0], c[1]]];
            if twice < 0.0 {
                plan.swap(1, 2);
            }
            triangles.push(plan);
            for [p, q] in [[a, b], [b, c], [c, a]] {
                let mut key = [grid_key(*p), grid_key(*q)];
                key.sort_unstable();
                let ends = [[p[0], p[1]], [q[0], q[1]]];
                edges.entry(key).or_insert((0, ends)).0 += 1;
            }
        }
        for (level, (area, edges, triangles)) in faces {
            let outline = edges
                .into_values()
                .filter(|(facets, _)| *facets == 1)
                .map(|(_, ends)| ends)
                .collect();
            pieces.entry(level).or_default().push(LevelPiece {
                area,
                outline,
                triangles,
            });
        }
    }
    pieces
        .into_iter()
        .map(|(level, pieces)| {
            let level_of = UpwardLevel {
                area: pieces.iter().map(|piece| piece.area).sum(),
                widest_mm: widest_touching(&pieces),
                triangles: pieces
                    .iter()
                    .flat_map(|piece| piece.triangles.iter().copied())
                    .collect(),
            };
            #[allow(clippy::cast_precision_loss)]
            let height = level as f64;
            (height, level_of)
        })
        .collect()
}

/// The group of `index` in a union-find forest, its paths halved on the way.
fn root(group: &mut [usize], mut index: usize) -> usize {
    while group[index] != index {
        group[index] = group[group[index]];
        index = group[index];
    }
    index
}

fn length_2d([p, q]: [[f64; 2]; 2]) -> f64 {
    (q[0] - p[0]).hypot(q[1] - p[1])
}

/// The clear width of the widest group of `pieces` that touch along an edge,
/// or lie at most [`FACE_JOINT_MM`] apart along it, measured as one face:
/// the outline they share is inside it, so a gallery of planks, laid apart
/// or not, or of OSB strips is as wide as all of them (review 2026-11-04 r19
/// MF-161, 2026-11-06 P3).
fn widest_touching(pieces: &[LevelPiece]) -> f64 {
    // Each outline edge by its direction (on the grid of `grid_key`) with its
    // offset across it, its piece and its extent along it.
    let mut lines = BTreeMap::<[i64; 3], Vec<(f64, usize, f64, f64)>>::new();
    for (index, piece) in pieces.iter().enumerate() {
        for [p, q] in &piece.outline {
            let length = length_2d([*p, *q]);
            if length <= NEGLIGIBLE {
                continue;
            }
            let mut direction = [(q[0] - p[0]) / length, (q[1] - p[1]) / length];
            if direction[0] < -ROUNDING || (direction[0] <= ROUNDING && direction[1] < 0.0) {
                direction = [-direction[0], -direction[1]];
            }
            let across = cross2(direction, *p);
            let [from, to] = [p, q].map(|end| dot2(direction, *end));
            // The direction to a millionth.
            let key = grid_key([direction[0] * 1e4, direction[1] * 1e4, 0.0]);
            lines
                .entry(key)
                .or_default()
                .push((across, index, from.min(to), from.max(to)));
        }
    }
    let mut group: Vec<usize> = (0..pieces.len()).collect();
    let mut shared = Vec::new();
    for edges in lines.values_mut() {
        edges.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (position, (across, first, from, to)) in edges.iter().enumerate() {
            let near = edges[position + 1..]
                .iter()
                .take_while(|other| other.0 - across <= FACE_JOINT_MM + ROUNDING);
            for (_, second, other_from, other_to) in near {
                let overlap = to.min(*other_to) - from.max(*other_from);
                if first != second && overlap > ROUNDING {
                    let (a, b) = (root(&mut group, *first), root(&mut group, *second));
                    group[a] = b;
                    shared.push((*first, overlap));
                }
            }
        }
    }
    // Per group: the area, and the outline less twice what its pieces share.
    let mut measures = BTreeMap::<usize, (f64, f64)>::new();
    for (index, piece) in pieces.iter().enumerate() {
        let measure = measures.entry(root(&mut group, index)).or_default();
        measure.0 += piece.area;
        measure.1 += piece.outline.iter().copied().map(length_2d).sum::<f64>();
    }
    for (index, overlap) in shared {
        measures.entry(root(&mut group, index)).or_default().1 -= 2.0 * overlap;
    }
    measures
        .into_values()
        .filter(|(_, outline)| *outline > 0.0)
        .map(|(area, outline)| clear_width(area, outline))
        .fold(0.0, f64::max)
}

/// The short side of the rectangle with `area` and perimeter `outline`: a
/// w × l rectangle has outline² − 16·area = 4(l − w)², so w is exact for a
/// rectangle; a rounder shape, with no such rectangle, counts as the square
/// of its outline.
fn clear_width(area: f64, outline: f64) -> f64 {
    (outline - (outline * outline - 16.0 * area).max(0.0).sqrt()) / 4.0
}

/// The floors of the plans: the upward-facing horizontal levels, lowest
/// first, that carry at least half of the area of the largest such level, so
/// small tops below a large deck do not count as a storey.
fn floor_levels<'a>(solids: impl IntoIterator<Item = &'a DrawingSolid>) -> Vec<f64> {
    let levels = upward_levels(solids);
    let largest = levels
        .iter()
        .map(|(_, level)| level.area)
        .fold(0.0, f64::max);
    levels
        .into_iter()
        .filter(|(_, level)| level.area >= largest * FLOOR_SHARE_OF_LARGEST_LEVEL)
        .map(|(height, _)| height)
        .collect()
}

/// The floors the sections and elevations mark, lowest first: those of the
/// plans and every level a person stands on, however small, such as a
/// gallery or a mezzanine; not the top of a wall, a knee wall or a parapet
/// (review 2026-11-04 MF-141), nor a top to sit or work at, less than
/// [`RAISED_LEVEL_MM`] over the highest level marked under
/// [`UNDER_SHARE_OF_TOP`] of it in plan, or with none under it, over the
/// highest floor below it (r19 MF-161; review 2026-11-06 MF-180: a landing on
/// the way up, the last level marked, unmarked a gallery across the hall; r23
/// MF-200: so did a landing in the free corner of an L-shaped gallery or under
/// its edge; r23 P3: tables beside the floor were marked; 2026-11-07 P3: a
/// table past the gallery's edge and a bed over an opening in it were measured
/// against the floor under the most of them, and a platform beside the hall
/// against a landing elsewhere). None once `stop` is set.
fn marked_floor_levels(solids: &[DrawingSolid], stop: &AtomicBool) -> Option<Vec<f64>> {
    let levels = upward_levels(solids);
    let largest = levels
        .iter()
        .map(|(_, level)| level.area)
        .fold(0.0, f64::max);
    // Each with whether it is a floor of the plans.
    let mut marked: Vec<(f64, UpwardLevel, bool)> = Vec::new();
    for (height, level) in levels {
        let floor = level.area >= largest * FLOOR_SHARE_OF_LARGEST_LEVEL;
        let mut under = None;
        if level.widest_mm >= STANDING_WIDTH_MM {
            for (below, beneath, _) in &marked {
                if level.area_over(beneath, stop)? >= level.area * UNDER_SHARE_OF_TOP {
                    under = Some(under.map_or(*below, |under: f64| under.max(*below)));
                }
            }
            under = under.or_else(|| {
                marked
                    .iter()
                    .filter(|(_, _, floor)| *floor)
                    .map(|(below, ..)| *below)
                    .reduce(f64::max)
            });
        }
        let raised = level.widest_mm >= STANDING_WIDTH_MM
            && under.is_none_or(|below| height - below >= RAISED_LEVEL_MM);
        if floor || raised {
            marked.push((height, level, floor));
        }
    }
    Some(marked.into_iter().map(|(height, ..)| height).collect())
}

/// Where ±0,000 is by default, as Czech and Slovak drawings put it: the floor
/// of the ground storey, the floor nearest z = 0 (one a storey below is a
/// basement); the highest floor of a model all below that, and z = 0 for a
/// model with no floor (review 2026-11-04 MF-141).
#[must_use]
pub fn ground_floor<'a>(solids: impl IntoIterator<Item = &'a DrawingSolid>) -> f64 {
    ground_floor_of(&floor_levels(solids))
}

/// [`ground_floor`] among `floors`, lowest first: the one nearest z = 0 of
/// those less than [`SUNKEN_GROUND_FLOOR_MM`] below it, so a ground floor
/// modelled 1 mm or 150 mm under z = 0 is still ±0,000, not the floor above
/// (review 2026-11-06 P3). A floor at or above z = 0 by less than that comes
/// first, so with the model's origin on the ground a terrace 150 mm below it
/// is not ±0,000 under a floor at +450 (r23 MF-203).
fn ground_floor_of(floors: &[f64]) -> f64 {
    floors
        .iter()
        .copied()
        .find(|floor| (0.0..SUNKEN_GROUND_FLOOR_MM).contains(floor))
        .or_else(|| {
            floors
                .iter()
                .copied()
                .filter(|floor| *floor > -SUNKEN_GROUND_FLOOR_MM)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()).then(b.total_cmp(a)))
        })
        .or_else(|| floors.last().copied())
        .unwrap_or(0.0)
}

/// A storey of the plans: its floor level and the height of its plan cut.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Storey {
    /// None for a model with no floor below its top, cut through its middle.
    pub floor_mm: Option<f64>,
    pub cut_mm: f64,
}

/// The storeys of `solids`, lowest first, each cut `above_floor_mm` above its
/// floor: a floor counts as a storey of its own only above the plan cut of the
/// storey below it and that far below the model's top, so a landing or a
/// top deck is no storey. A model with no such floor is one storey cut
/// through its middle; no solid, no storey.
#[must_use]
pub fn storeys(solids: &[DrawingSolid], above_floor_mm: f64) -> Vec<Storey> {
    let Some((min, max)) = bounds(solids) else {
        return Vec::new();
    };
    let mut storeys: Vec<Storey> = Vec::new();
    for floor in floor_levels(solids) {
        let cut = floor + above_floor_mm;
        if cut < max[2] && storeys.last().is_none_or(|below| floor > below.cut_mm) {
            storeys.push(Storey {
                floor_mm: Some(floor),
                cut_mm: cut,
            });
        }
    }
    if storeys.is_empty() {
        storeys.push(Storey {
            floor_mm: None,
            cut_mm: f64::midpoint(min[2], max[2]),
        });
    }
    storeys
}

/// The finished floors of a table of storeys, lowest first.
fn finished_floors(table: &[StoreyLevels]) -> Vec<f64> {
    let mut floors = table
        .iter()
        .map(|storey| f64::from(storey.finished_floor_mm))
        .collect::<Vec<_>>();
    floors.sort_by(f64::total_cmp);
    floors
}

/// ±0,000 of a table of storeys: the finished floor of its ground storey,
/// chosen among its floors as [`ground_floor`] chooses among those found.
#[must_use]
pub fn table_ground_floor(table: &[StoreyLevels]) -> f64 {
    ground_floor_of(&finished_floors(table))
}

/// The storeys of a table of storeys, lowest first, each cut `above_floor_mm`
/// above its finished floor (review 2026-11-08 MF-241).
#[must_use]
pub fn table_storeys(table: &[StoreyLevels], above_floor_mm: f64) -> Vec<Storey> {
    let mut storeys = table
        .iter()
        .map(|storey| {
            let floor = f64::from(storey.finished_floor_mm);
            Storey {
                floor_mm: Some(floor),
                cut_mm: floor + above_floor_mm,
            }
        })
        .collect::<Vec<_>>();
    storeys.sort_by(|a, b| a.cut_mm.total_cmp(&b.cut_mm));
    storeys
}

/// The plan cut of the lowest storey: `above_floor_mm` above the floor, or the
/// middle of a model with no floor that far below its top.
#[must_use]
pub fn default_plan_cut_z(solids: &[DrawingSolid], above_floor_mm: f64) -> Option<f64> {
    storeys(solids, above_floor_mm)
        .first()
        .map(|storey| storey.cut_mm)
}

fn annotation_bounds(annotations: &[Annotation]) -> Option<(Point, Point)> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in annotations.iter().flat_map(Annotation::points) {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    min[0].is_finite().then_some((min, max))
}

/// A dimension in whole millimetres, a half rounded up as a drawing reads:
/// `{:.0}` alone rounds 2450.5 to the even 2450.
fn whole_mm(value: f64) -> String {
    format!("{:.0}", value.round())
}

#[cfg(test)]
mod tests;
