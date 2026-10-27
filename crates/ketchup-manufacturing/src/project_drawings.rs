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

use crate::sheet_pdf::{Anchor, Mark, Page, Stroke};
use crate::title_block::{SheetFormat, TITLE_BLOCK_MM, TitleValueDoesNotFit, drawing_area};
use ketchup_geometry::linalg::{Vec3, cross, cross2, dot, dot2, normalize_within, sub};
use ketchup_model::tolerance::{HIDDEN_LINE_TOUCH_MM, NEGLIGIBLE, ROUNDING};
use serde::Serialize;
use std::collections::BTreeMap;

mod annotations;
mod cutting;
mod openings;
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
/// mirroring placement), so their normals point inward.
fn winds_inward(solid: &DrawingSolid) -> bool {
    let volume: f64 = solid
        .triangles
        .iter()
        .map(|[a, b, c]| dot(*a, cross(*b, *c)))
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
    /// Boxed marks where they stand: the windows and doors a plan shows, the
    /// position numbers of a shop drawing.
    marks: Vec<([f64; 2], String)>,
    cut_at_mm: Option<f64>,
}

impl DrawnView {
    /// Draws `annotations` in this view and grows it to hold them.
    fn annotate(&mut self, frame: &ViewFrame, annotations: &[Annotation]) {
        let annotations = PlanAnnotations::project(frame, annotations);
        for point in annotations.points() {
            for (axis, value) in point.into_iter().enumerate() {
                self.min[axis] = self.min[axis].min(value);
                self.max[axis] = self.max[axis].max(value);
            }
        }
        self.annotations = Some(annotations);
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
        marks: Vec::new(),
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

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
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
/// Room around a view for its dimensions and title, in page millimetres.
const VIEW_PAD_MM: f64 = 22.0;
pub const SCALES: [u32; 10] = [10, 20, 25, 50, 75, 100, 200, 250, 500, 1000];

// not a tolerance: the pen width of a visible edge on the sheet.
const EDGE_STROKE_MM: f64 = 0.18;
const EDGE_COLOR: [u8; 3] = [34, 34, 34];
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

/// The floors: the upward-facing horizontal levels, lowest first, that carry
/// at least half of the area of the largest such level, so small tops below a
/// large deck do not count as a floor.
fn floor_levels(solids: &[DrawingSolid]) -> Vec<f64> {
    let mut levels = BTreeMap::<i64, f64>::new();
    for solid in solids {
        let outward = if winds_inward(solid) { -1.0 } else { 1.0 };
        for triangle in &solid.triangles {
            let [a, b, c] = triangle;
            let Some(normal) = unit_normal(triangle) else {
                continue;
            };
            if normal[2] * outward < 1.0 - ROUNDING {
                continue;
            }
            let area = cross(sub(*b, *a), sub(*c, *a))[2].abs() / 2.0;
            #[allow(clippy::cast_possible_truncation)]
            let level = ((a[2] + b[2] + c[2]) / 3.0).round() as i64;
            *levels.entry(level).or_default() += area;
        }
    }
    let largest = levels.values().copied().fold(0.0, f64::max);
    #[allow(clippy::cast_precision_loss)]
    levels
        .into_iter()
        .filter(|(_, area)| *area >= largest * FLOOR_SHARE_OF_LARGEST_LEVEL)
        .map(|(level, _)| level as f64)
        .collect()
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
mod tests {
    use super::*;

    /// The twelve triangles of an axis-aligned box.
    fn cuboid(name: &str, min: Point, max: Point) -> DrawingSolid {
        let corner = |index: usize| -> Point {
            std::array::from_fn(|axis| {
                if index & (1 << axis) == 0 {
                    min[axis]
                } else {
                    max[axis]
                }
            })
        };
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let triangles = quads
            .iter()
            .flat_map(|[a, b, c, d]| {
                [
                    [corner(*a), corner(*b), corner(*c)],
                    [corner(*a), corner(*c), corner(*d)],
                ]
            })
            .collect();
        DrawingSolid {
            name: name.to_owned(),
            color: [200, 160, 110],
            triangles,
            faces: (0..6).flat_map(|face| [face, face]).collect(),
            threads: Vec::new(),
        }
    }

    /// A closed cylinder about z, its side meshed in `segments` facets: the
    /// side is face 0, the bottom face 1, the top face 2.
    fn cylinder(radius: f64, height: f64, segments: u32) -> DrawingSolid {
        let rim = |index: u32, z: f64| -> Point {
            let angle = f64::from(index % segments) * std::f64::consts::TAU / f64::from(segments);
            [radius * angle.cos(), radius * angle.sin(), z]
        };
        let (mut triangles, mut faces) = (Vec::new(), Vec::new());
        for index in 0..segments {
            let (a, b) = (rim(index, 0.0), rim(index + 1, 0.0));
            let (c, d) = (rim(index + 1, height), rim(index, height));
            triangles.extend([[a, b, c], [a, c, d]]);
            faces.extend([0, 0]);
            triangles.push([[0.0, 0.0, 0.0], b, a]);
            faces.push(1);
            triangles.push([[0.0, 0.0, height], d, c]);
            faces.push(2);
        }
        DrawingSolid {
            name: "pin".to_owned(),
            color: [200, 160, 110],
            triangles,
            faces,
            threads: Vec::new(),
        }
    }

    /// P3-V: a cylinder meshed coarser than the old 20° fold limit (the
    /// display mesh steps up to 0.35 rad, just over 20°) drew a line at every
    /// facet of its side; only its two rims are edges.
    #[test]
    fn a_coarsely_meshed_cylinder_shows_only_its_rims() {
        let edges = solid_edges(&cylinder(50.0, 100.0, 17)).features;
        assert_eq!(edges.len(), 2 * 17, "{edges:?}");
        assert!(
            edges
                .iter()
                .all(|[a, b]| a[2] == b[2] && (a[2] == 0.0 || a[2] == 100.0)),
            "{edges:?}"
        );
    }

    /// MF-16: a round column seen from the side shows its two vertical
    /// outlines, not only its rims, and seen from above no line of its side.
    #[test]
    fn a_cylinder_seen_from_the_side_shows_its_outline() {
        let column = cylinder(50.0, 100.0, 17);
        let edges = vec![solid_edges(&column)];
        let frames = view_frames([-50.0, -50.0, 0.0], [50.0, 50.0, 100.0], 200.0, false);
        let (view, front) = frames[3];
        assert_eq!(view, ProjectView::Front);
        let drawn = draw_view(view, front, std::slice::from_ref(&column), &edges);
        let mut sides = drawn
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Edge([a, b]) if (a[0] - b[0]).abs() < 1e-9 => Some(a[0]),
                _ => None,
            })
            .collect::<Vec<_>>();
        sides.sort_by(f64::total_cmp);
        sides.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        assert_eq!(sides.len(), 2, "{sides:?}");
        assert!(
            sides[0] < -49.0 && (sides[1] - 50.0).abs() < 1e-9,
            "{sides:?}"
        );
        assert_eq!(outline_edges(&edges[0], frames[0].1.depth).count(), 0);
    }

    /// P3-V: two faces of a shallow roof meeting at 10° still show their ridge,
    /// because it is an edge of the model, however small the fold.
    #[test]
    fn a_shallow_fold_between_two_faces_is_drawn() {
        let pitch = 5_f64.to_radians().tan() * 1000.0;
        let (south_west, south_east) = ([0.0, 0.0, 0.0], [3000.0, 0.0, 0.0]);
        let (north_west, north_east) = ([0.0, 2000.0, 0.0], [3000.0, 2000.0, 0.0]);
        let (ridge_west, ridge_east) = ([0.0, 1000.0, pitch], [3000.0, 1000.0, pitch]);
        let roof = DrawingSolid {
            name: "roof".to_owned(),
            color: [200, 160, 110],
            triangles: vec![
                [south_west, south_east, ridge_east],
                [south_west, ridge_east, ridge_west],
                [ridge_west, ridge_east, north_east],
                [ridge_west, north_east, north_west],
            ],
            faces: vec![0, 0, 1, 1],
            threads: Vec::new(),
        };
        let edges = solid_edges(&roof).features;
        assert!(
            edges.contains(&[ridge_west, ridge_east]) || edges.contains(&[ridge_east, ridge_west]),
            "{edges:?}"
        );
        assert_eq!(
            edges.len(),
            7,
            "the ridge and the six outline edges: {edges:?}"
        );
    }

    /// A 6 × 3 m room: a floor slab and four 150 mm walls 2.5 m high.
    fn room() -> Vec<DrawingSolid> {
        vec![
            cuboid("floor", [0.0, 0.0, 0.0], [6000.0, 3000.0, 200.0]),
            cuboid("south", [0.0, 0.0, 200.0], [6000.0, 150.0, 2700.0]),
            cuboid("north", [0.0, 2850.0, 200.0], [6000.0, 3000.0, 2700.0]),
            cuboid("west", [0.0, 150.0, 200.0], [150.0, 2850.0, 2700.0]),
            cuboid("east", [5850.0, 150.0, 200.0], [6000.0, 2850.0, 2700.0]),
        ]
    }

    use crate::sheet_pdf::PdfInfo;
    use crate::title_block::{TitleBlock, TitleField};

    fn plans(storeys: Vec<Storey>) -> Vec<PlanSheet> {
        storeys
            .into_iter()
            .enumerate()
            .map(|(index, storey)| PlanSheet {
                storey,
                view_title: format!("Floor plan {}", index + 1),
                sheet_title: format!("Plan {}", index + 1),
            })
            .collect()
    }

    fn options(solids: &[DrawingSolid], cut_mm: Option<f64>) -> DrawingSetOptions {
        let storeys = match cut_mm {
            Some(cut_mm) => vec![Storey {
                floor_mm: None,
                cut_mm,
            }],
            None => storeys(solids, 1200.0),
        };
        DrawingSetOptions {
            plans: plans(storeys),
            sheet_titles: SheetTitles {
                sections: "Sections".to_owned(),
                elevations: "Elevations".to_owned(),
            },
            title_labels: BTreeMap::from([(TitleField::Project, "Stavba".to_owned())]),
            title_block: TitleBlock::from([(TitleField::Project, "Room <A>".to_owned())]),
            ..DrawingSetOptions::default()
        }
    }

    fn room_set(cut_mm: Option<f64>) -> DrawingSet {
        drawing_set(&room(), &options(&room(), cut_mm)).unwrap()
    }

    fn texts(page: &Page) -> Vec<&str> {
        page.texts().collect()
    }

    fn sheet_of(set: &DrawingSet, kind: SheetKind) -> &SetSheet {
        set.sheets.iter().find(|sheet| sheet.kind == kind).unwrap()
    }

    fn summary(set: &DrawingSet, view: ProjectView) -> &ViewSummary {
        set.sheets
            .iter()
            .flat_map(|sheet| &sheet.views)
            .find(|summary| summary.view == view)
            .unwrap()
    }

    /// MF-4: a half millimetre rounds up, as a drawing reads, not to the even neighbour.
    #[test]
    fn overall_dimensions_round_a_half_millimetre_up() {
        let block = [cuboid("block", [0.0, 0.0, 0.0], [2450.5, 1000.0, 2.5])];
        let set = drawing_set(&block, &options(&block, None)).unwrap();
        let texts = set
            .sheets
            .iter()
            .flat_map(|sheet| sheet.page.texts())
            .collect::<Vec<_>>();
        assert!(texts.contains(&"2451"), "{texts:?}");
        assert!(texts.contains(&"3"), "{texts:?}");
        assert!(!texts.contains(&"2450"), "{texts:?}");
    }

    #[test]
    fn a_plan_cut_below_the_origin_reports_its_height_with_its_sign() {
        // The room lowered by 1 m: its walls stand from -800 to 1700.
        let lowered = room()
            .into_iter()
            .map(|mut solid| {
                for corner in solid.triangles.iter_mut().flatten() {
                    corner[2] -= 1000.0;
                }
                solid
            })
            .collect::<Vec<_>>();
        let set = drawing_set(&lowered, &options(&lowered, Some(-300.0))).unwrap();
        assert_eq!(summary(&set, ProjectView::Plan).cut_at_mm, Some(-300.0));
        // Its slab top at -800: the storey is cut 1.2 m above it.
        assert_eq!(storeys(&lowered, 1200.0)[0].cut_mm, 400.0);
    }

    /// The room is a set of three sheets at 1:50: its plan, its sections and
    /// its elevations, numbered 1 / 3 to 3 / 3, each with the frame and the
    /// title block, saying what it shows.
    #[test]
    fn the_set_has_a_plan_a_sections_and_an_elevations_sheet_each_numbered_and_titled() {
        let set = room_set(Some(1200.0));
        assert_eq!(set.scale, 50, "a room is drawn at the building scale");
        assert_eq!(
            set.sheets
                .iter()
                .map(|sheet| (sheet.kind, sheet.number, sheet.title.as_str(), sheet.format))
                .collect::<Vec<_>>(),
            [
                (SheetKind::Plan { storey: 0 }, 1, "Plan 1", SheetFormat::A3),
                (SheetKind::Sections, 2, "Sections", SheetFormat::A3),
                (SheetKind::Elevations, 3, "Elevations", SheetFormat::A3),
            ]
        );
        for sheet in &set.sheets {
            let texts = texts(&sheet.page);
            let number = format!("{} / 3", sheet.number);
            for wanted in [
                sheet.title.as_str(),
                number.as_str(),
                "1:50",
                "A3",
                "Room <A>",
                "Stavba",
            ] {
                assert!(
                    texts.contains(&wanted),
                    "{wanted} on sheet {}: {texts:?}",
                    sheet.number
                );
            }
            // The frame and the title block in its lower right corner on every
            // sheet, in its painted and its line drawing alike.
            for page in [&sheet.page, &sheet.lines] {
                for (min, max, width_mm) in [
                    ([20.0, 10.0], [410.0, 287.0], 0.7),
                    ([230.0, 238.0], [410.0, 287.0], 0.5),
                ] {
                    assert!(
                        page.marks.contains(&Mark::Line {
                            points: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
                            closed: true,
                            stroke: Stroke::solid(width_mm),
                        }),
                        "sheet {}",
                        sheet.number
                    );
                }
            }
        }
        let pdf = set.pdf(&PdfInfo::default());
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(
            pdf.windows(b"/Type /Page /".len())
                .filter(|window| *window == b"/Type /Page /")
                .count(),
            3,
            "a page per sheet"
        );
    }

    #[test]
    fn plan_and_sections_fill_the_parts_the_plane_cuts() {
        let set = room_set(Some(1200.0));
        let plan = summary(&set, ProjectView::Plan);
        assert_eq!(plan.cut_solids, 4, "the four walls, not the floor");
        assert_eq!(plan.cut_at_mm, Some(1200.0));
        assert_eq!(plan.title, "Floor plan 1");
        assert_eq!(
            summary(&set, ProjectView::LongitudinalSection).cut_at_mm,
            Some(1500.0),
            "the room's middle across"
        );
        assert_eq!(plan.extent_mm, [6000.0, 3000.0]);
        // Along the 6 m length: floor plus the two end walls; across: floor and side walls.
        assert_eq!(
            summary(&set, ProjectView::LongitudinalSection).cut_solids,
            3
        );
        assert_eq!(summary(&set, ProjectView::CrossSection).cut_solids, 3);
        assert_eq!(
            summary(&set, ProjectView::CrossSection).extent_mm,
            [3000.0, 2700.0]
        );
        assert_eq!(summary(&set, ProjectView::Front).cut_solids, 0);
        assert_eq!(
            summary(&set, ProjectView::Front).extent_mm,
            [6000.0, 2700.0]
        );
        let cut_fills = |sheet: &SetSheet| {
            sheet
                .page
                .marks
                .iter()
                .filter(|mark| {
                    matches!(mark, Mark::Fill { outline: Some(stroke), .. } if stroke.width_mm == CUT_STROKE_MM)
                })
                .count()
        };
        assert_eq!(cut_fills(sheet_of(&set, SheetKind::Plan { storey: 0 })), 4);
        assert_eq!(cut_fills(sheet_of(&set, SheetKind::Sections)), 6);
        assert_eq!(cut_fills(sheet_of(&set, SheetKind::Elevations)), 0);
        let elevations = texts(&sheet_of(&set, SheetKind::Elevations).page);
        assert!(elevations.contains(&"6000"));
        assert!(elevations.contains(&"2700"));
    }

    #[test]
    fn the_default_plan_cut_is_measured_from_the_floor_not_from_the_footings() {
        // The room on four footings reaching 1312 mm below the slab.
        let mut house = room()
            .into_iter()
            .map(|mut solid| {
                for triangle in &mut solid.triangles {
                    for point in triangle {
                        point[2] += 1312.0;
                    }
                }
                solid
            })
            .collect::<Vec<_>>();
        for (x, y) in [(0.0, 0.0), (5400.0, 0.0), (0.0, 2400.0), (5400.0, 2400.0)] {
            house.push(cuboid(
                "footing",
                [x, y, 0.0],
                [x + 600.0, y + 600.0, 1312.0],
            ));
        }
        // Slab top at 1512: the cut is 1.2 m above it, through the walls only.
        assert_eq!(default_plan_cut_z(&house, 1200.0), Some(2712.0));
        // A height set in the sheet settings moves the cut with it.
        assert_eq!(default_plan_cut_z(&house, 900.0), Some(2412.0));
        // A model with no floor below head height is cut through its middle.
        let table = [cuboid("top", [0.0; 3], [1600.0, 800.0, 40.0])];
        assert_eq!(default_plan_cut_z(&table, 1200.0), Some(20.0));
    }

    /// The room with a second storey on its 200 mm ceiling slab and a flat
    /// roof slab: two storeys, the roof deck is none.
    fn two_storeys() -> Vec<DrawingSolid> {
        let mut house = room();
        for (name, z) in [("ceiling", 2700.0), ("roof", 5400.0)] {
            house.push(cuboid(name, [0.0, 0.0, z], [6000.0, 3000.0, z + 200.0]));
        }
        for wall in room().into_iter().skip(1) {
            let mut upper = wall;
            upper.name.push_str(" upper");
            for corner in upper.triangles.iter_mut().flatten() {
                corner[2] += 2700.0;
            }
            house.push(upper);
        }
        house
    }

    #[test]
    fn each_storey_has_a_plan_sheet_of_its_own_with_its_annotations() {
        use ketchup_model::annotation::AnnotationMark;
        let house = two_storeys();
        assert_eq!(
            storeys(&house, 1200.0),
            [
                Storey {
                    floor_mm: Some(200.0),
                    cut_mm: 1400.0
                },
                Storey {
                    floor_mm: Some(2900.0),
                    cut_mm: 4100.0
                },
            ]
        );
        let dimension = |name: &str, z: f64| Annotation {
            name: name.to_owned(),
            tags: std::collections::BTreeSet::new(),
            mark: AnnotationMark::Dimension {
                from_mm: [0.0, -500.0, z],
                to_mm: [6000.0, -500.0, z],
                offset_mm: -500.0,
                height_mm: 2.5,
            },
        };
        let mut options = options(&house, None);
        options.annotations = vec![dimension("ground", 0.0), dimension("upper", 2900.0)];
        options
            .title_block
            .insert(TitleField::Drawing, "Rodinný dom".to_owned());
        let set = drawing_set(&house, &options).unwrap();
        assert_eq!(
            set.sheets
                .iter()
                .map(|sheet| (sheet.kind, sheet.number))
                .collect::<Vec<_>>(),
            [
                (SheetKind::Plan { storey: 0 }, 1),
                (SheetKind::Plan { storey: 1 }, 2),
                (SheetKind::Sections, 3),
                (SheetKind::Elevations, 4),
            ]
        );
        let cuts = set.sheets[..2]
            .iter()
            .map(|sheet| sheet.views[0].cut_at_mm)
            .collect::<Vec<_>>();
        assert_eq!(cuts, [Some(1400.0), Some(4100.0)]);
        // Each plan draws the dimension of its own storey only.
        for sheet in &set.sheets[..2] {
            assert_eq!(sheet.annotations.dimensions, 1, "sheet {}", sheet.number);
        }
        assert_eq!(set.annotations().dimensions, 2);
        for sheet in &set.sheets {
            let texts = texts(&sheet.page);
            let title = format!("Rodinný dom – {}", sheet.title);
            let number = format!("{} / 4", sheet.number);
            assert!(texts.contains(&title.as_str()), "{texts:?}");
            assert!(texts.contains(&number.as_str()), "{texts:?}");
        }

        // Review 2026-10-24 MF-29: plans given by their cuts alone have no
        // floor, and each drew the annotations of every storey.
        options.plans = plans(
            [1400.0, 4100.0]
                .map(|cut_mm| Storey {
                    floor_mm: None,
                    cut_mm,
                })
                .to_vec(),
        );
        let set = drawing_set(&house, &options).unwrap();
        for sheet in &set.sheets[..2] {
            assert_eq!(sheet.annotations.dimensions, 1, "sheet {}", sheet.number);
        }
        assert_eq!(set.annotations().dimensions, 2);
    }

    #[test]
    fn every_sheet_is_a_dxf_of_its_visible_lines_with_its_title_block() {
        let set = room_set(Some(1200.0));
        let files = set.dxf();
        assert_eq!(files.len(), 3);
        for (sheet, dxf) in set.sheets.iter().zip(&files) {
            let dxf = String::from_utf8(dxf.clone()).unwrap();
            assert!(dxf.starts_with("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009"));
            assert!(dxf.contains(&format!("1\n{} / 3\n", sheet.number)), "{dxf}");
            assert!(dxf.contains(&format!("1\n{}\n", sheet.title)));
            assert!(dxf.contains("0\nLINE\n8\nPEN-0_18\n"), "the edges");
            // No face is painted: a fill is only the outline of a cut.
            assert!(
                sheet
                    .lines
                    .marks
                    .iter()
                    .all(|mark| !matches!(mark, Mark::Fill { outline: None, .. })),
                "sheet {}",
                sheet.number
            );
        }
        let plan = String::from_utf8(files[0].clone()).unwrap();
        assert_eq!(
            plan.matches("0\nLINE\n8\nCUT\n").count(),
            16,
            "the four walls the plan cuts, outlined"
        );
    }

    /// A box behind a wall is hidden in the front elevation's lines; the part
    /// of a taller box above the wall shows.
    #[test]
    fn the_lines_of_a_view_leave_out_what_nearer_faces_hide() {
        let wall = cuboid("wall", [0.0, 0.0, 0.0], [4000.0, 200.0, 2500.0]);
        let hidden = cuboid("hidden", [500.0, 1000.0, 0.0], [1500.0, 2000.0, 1000.0]);
        let taller = cuboid("taller", [2000.0, 1000.0, 0.0], [3000.0, 2000.0, 3000.0]);
        let solids = vec![wall, hidden, taller];
        let edges = solids.iter().map(solid_edges).collect::<Vec<_>>();
        let (min, max) = bounds(&solids).unwrap();
        let (view, front) = view_frames(min, max, 1200.0, false)[3];
        assert_eq!(view, ProjectView::Front);
        let drawn = draw_view(view, front, &solids, &edges);
        let painted_inside = drawn.items.iter().any(|item| {
            matches!(item, Item::Edge([a, b]) if a[0] > 500.0 - 1.0 && b[0] < 1500.0 + 1.0 && a[1] < 1000.0 + 1.0 && b[1] < 1000.0 + 1.0)
        });
        assert!(
            painted_inside,
            "the PDF paints the hidden box's edges, then the wall over them"
        );
        let lines = visible::visible_lines(&drawn);
        let on = |value: f64, wanted: f64| (value - wanted).abs() < 1e-6;
        for line in &lines {
            let [a, b] = [line.points[0], line.points[1]];
            let wall_outline = (on(a[0], 0.0) && on(b[0], 0.0))
                || (on(a[0], 4000.0) && on(b[0], 4000.0))
                || (on(a[1], 0.0) && on(b[1], 0.0))
                || (on(a[1], 2500.0) && on(b[1], 2500.0));
            let above_the_wall = a[1] >= 2500.0 - 1e-6 && b[1] >= 2500.0 - 1e-6;
            assert!(
                wall_outline || above_the_wall,
                "{line:?} is hidden behind the wall"
            );
        }
        // The taller box shows its sides above the wall and its top.
        for x in [2000.0, 3000.0] {
            assert!(
                lines.iter().any(|line| on(line.points[0][0], x)
                    && on(line.points[1][0], x)
                    && on(line.points[0][1].min(line.points[1][1]), 2500.0)
                    && on(line.points[0][1].max(line.points[1][1]), 3000.0)),
                "the side at x = {x} from 2500 to 3000: {lines:?}"
            );
        }
        assert!(
            lines
                .iter()
                .any(|line| on(line.points[0][1], 3000.0) && on(line.points[1][1], 3000.0))
        );
        // The bottom edge of the wall, split by depth slabs, is one line again.
        assert_eq!(
            lines
                .iter()
                .filter(|line| on(line.points[0][1], 0.0) && on(line.points[1][1], 0.0))
                .count(),
            1,
            "{lines:?}"
        );
    }

    #[test]
    fn every_sheet_is_drawn_at_the_scale_of_the_set_on_the_smallest_format_that_holds_it() {
        // A 24 m long hall: at 1:50 its plan needs an A2, its two sections side
        // by side an A1 and its elevations, two of 524 mm in a row, an A0.
        let hall = [
            cuboid("floor", [0.0, 0.0, 0.0], [24_000.0, 3000.0, 200.0]),
            cuboid("south", [0.0, 0.0, 200.0], [24_000.0, 150.0, 2700.0]),
        ];
        let set = drawing_set(&hall, &options(&hall, Some(1200.0))).unwrap();
        assert_eq!(set.scale, 50);
        let formats = set
            .sheets
            .iter()
            .map(|sheet| (sheet.kind, sheet.format))
            .collect::<Vec<_>>();
        assert_eq!(
            formats,
            [
                (SheetKind::Plan { storey: 0 }, SheetFormat::A2),
                (SheetKind::Sections, SheetFormat::A1),
                (SheetKind::Elevations, SheetFormat::A0),
            ]
        );
        // Allowed 1:100, the sections fit an A3 only at 1:100, so the whole
        // set is drawn at 1:100 and the elevations shrink to an A1.
        let mut coarser = options(&hall, Some(1200.0));
        coarser.coarsest_scale = Some(100);
        let coarser = drawing_set(&hall, &coarser).unwrap();
        assert_eq!(coarser.scale, 100);
        assert_eq!(
            coarser
                .sheets
                .iter()
                .map(|sheet| sheet.format)
                .collect::<Vec<_>>(),
            [SheetFormat::A3, SheetFormat::A3, SheetFormat::A1]
        );
    }

    #[test]
    fn a_plan_cut_above_everything_shows_the_roofless_room_from_above() {
        let set = room_set(Some(5000.0));
        let plan = summary(&set, ProjectView::Plan);
        assert_eq!(plan.cut_solids, 0);
        assert_eq!(plan.extent_mm, [6000.0, 3000.0]);
        let edges = set.sheets[0]
            .page
            .marks
            .iter()
            .filter(|mark| matches!(mark, Mark::Line { stroke, .. } if stroke.color == EDGE_COLOR))
            .count();
        assert!(edges > 0, "the walls are drawn from above");
    }

    #[test]
    fn cut_loops_close_around_each_wall() {
        let frame = ViewFrame {
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            depth: [0.0, 0.0, -1.0],
            cut: Some(-1200.0),
        };
        let loops = cut_loops(&frame, &room()[1]);
        assert_eq!(loops.len(), 1);
        let area = polygon_area(&loops[0][..loops[0].len() - 1]).abs();
        assert!((area - 6000.0 * 150.0).abs() < 1.0, "{area}");
    }

    /// The plan sheet of the room on A3 at 1:50, worked out by hand. The frame
    /// starts 20 mm from the left and 10 mm from the top and every view is
    /// padded by 22 mm, so the plan's 6000 mm run from x = 42 to 42 + 6000/50 =
    /// 162 and its 3000 mm from y = 21 to 21 + 60 = 81; the width line lies
    /// 8 mm below, the height line 8 mm to the left. On the sections sheet the
    /// cross section starts after the longitudinal one, 20 + 120 + 44 + 22 =
    /// 206, and is 3000/50 = 60 wide and 2700/50 = 54 high.
    #[test]
    fn dimension_lines_and_the_title_block_sit_where_the_scale_puts_them() {
        let set = room_set(Some(1200.0));
        let ticks = |page: &Page| {
            page.marks
                .iter()
                .filter_map(|mark| match mark {
                    Mark::Line { points, stroke, .. } if stroke.width_mm == 0.35 => Some([
                        f64::midpoint(points[0][0], points[1][0]),
                        f64::midpoint(points[0][1], points[1][1]),
                    ]),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let text_at = |page: &Page, text: &str, at: [f64; 2]| {
            page.marks.iter().any(|mark| {
                matches!(mark, Mark::Text { text: t, at: a, .. }
                    if t == text && (a[0] - at[0]).abs() < 1e-9 && (a[1] - at[1]).abs() < 1e-9)
            })
        };
        let plan = &set.sheets[0].page;
        let sections = &set.sheets[1].page;
        for (page, wanted) in [
            (
                plan,
                [[42.0, 89.0], [162.0, 89.0], [34.0, 81.0], [34.0, 21.0]],
            ),
            (
                sections,
                [[206.0, 83.0], [266.0, 83.0], [198.0, 75.0], [198.0, 21.0]],
            ),
        ] {
            let ticks = ticks(page);
            for tick in wanted {
                assert!(
                    ticks
                        .iter()
                        .any(|t| (t[0] - tick[0]).abs() < 1e-9 && (t[1] - tick[1]).abs() < 1e-9),
                    "no tick at {tick:?} in {ticks:?}"
                );
            }
        }
        // Each value sits over the middle of its line, 1 mm clear of it.
        assert!(text_at(plan, "6000", [102.0, 88.0]));
        assert!(text_at(plan, "3000", [33.0, 51.0]));
        assert!(text_at(sections, "3000", [236.0, 82.0]));
        assert!(text_at(sections, "2700", [197.0, 48.0]));
        // The scale and the sheet number in their cells of the 180 × 49 title
        // block in the lower right corner of the 420 × 297 sheet.
        assert!(plan.marks.contains(&Mark::Line {
            points: vec![
                [295.0, 278.0],
                [325.0, 278.0],
                [325.0, 287.0],
                [295.0, 287.0]
            ],
            closed: true,
            stroke: Stroke::solid(0.25),
        }));
        assert!(plan.marks.iter().any(|mark| matches!(mark,
            Mark::Text { text, at, .. } if text == "1:50" && (at[0] - 296.5).abs() < 1e-9)));
        assert!(plan.marks.iter().any(|mark| matches!(mark,
            Mark::Text { text, at, .. } if text == "1 / 3" && (at[0] - 346.5).abs() < 1e-9)));
    }

    #[test]
    fn nothing_visible_is_an_error() {
        assert_eq!(
            drawing_set(&[], &options(&[], Some(0.0))),
            Err(ProjectDrawingError::Empty)
        );
        assert_eq!(
            drawing_set(&room(), &DrawingSetOptions::default()),
            Err(ProjectDrawingError::Empty),
            "no plan asked for"
        );
    }

    /// Eight points 5 m apart on the south side of a 35 m square: a number at
    /// each, a chain of dimensions below them and the square's outline.
    fn anchor_plan() -> Vec<Annotation> {
        use ketchup_model::annotation::AnnotationMark;
        let point = |index: u32| [f64::from(index) * 5000.0, 0.0, 0.0];
        let annotation = |name: String, mark| Annotation {
            name,
            tags: std::collections::BTreeSet::new(),
            mark,
        };
        let mut plan = (0..8)
            .map(|index| {
                annotation(
                    format!("S-{:02}", index + 1),
                    AnnotationMark::Text {
                        at_mm: point(index),
                        text: format!("S-{:02}", index + 1),
                        height_mm: 3.5,
                    },
                )
            })
            .collect::<Vec<_>>();
        plan.extend((0..7).map(|index| {
            annotation(
                format!("d{index}"),
                AnnotationMark::Dimension {
                    from_mm: point(index),
                    to_mm: point(index + 1),
                    offset_mm: -1500.0,
                    height_mm: 2.5,
                },
            )
        }));
        plan.push(annotation(
            "outline".to_owned(),
            AnnotationMark::Line {
                points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [35_000.0, 0.0, 0.0],
                    [35_000.0, 35_000.0, 0.0],
                    [0.0, 35_000.0, 0.0],
                ],
                closed: true,
            },
        ));
        plan
    }

    fn text_at<'a>(page: &'a Page, wanted: &str) -> Vec<&'a [f64; 2]> {
        page.marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text { text, at, .. } if text == wanted => Some(at),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_plan_of_its_own_draws_its_annotations_at_the_scale_it_is_given() {
        let marker = cuboid("marker", [-100.0, -100.0, 5.0], [100.0, 100.0, 10.0]);
        let options = DrawingSetOptions {
            plan_only: true,
            scale: Some(250),
            annotations: anchor_plan(),
            ..options(std::slice::from_ref(&marker), None)
        };
        let set = drawing_set(std::slice::from_ref(&marker), &options).unwrap();
        assert_eq!(set.scale, 250);
        assert_eq!(set.sheets.len(), 1, "the plan alone");
        let sheet = &set.sheets[0];
        assert_eq!(
            sheet.views.iter().map(|view| view.view).collect::<Vec<_>>(),
            [ProjectView::Plan]
        );
        assert_eq!(
            sheet.annotations,
            AnnotationCounts {
                texts: 8,
                dimensions: 7,
                lines: 1,
                ..AnnotationCounts::default()
            }
        );
        // Every number at its point: 5 m apart is 20 mm at 1:250.
        let numbers = (1..=8)
            .map(|index| {
                let found = text_at(&sheet.page, &format!("S-{index:02}"));
                assert_eq!(found.len(), 1, "S-{index:02}");
                *found[0]
            })
            .collect::<Vec<_>>();
        for pair in numbers.windows(2) {
            assert!((pair[1][0] - pair[0][0] - 20.0).abs() < 1e-9, "{pair:?}");
            assert!((pair[1][1] - pair[0][1]).abs() < 1e-9, "{pair:?}");
        }
        // Seven measured values, each centred between its two points and below them.
        let values = text_at(&sheet.page, "5000");
        assert_eq!(values.len(), 7, "{values:?}");
        for (value, pair) in values.iter().zip(numbers.windows(2)) {
            assert!((value[0] - f64::midpoint(pair[0][0], pair[1][0])).abs() < 1e-9);
            assert!(value[1] > pair[0][1], "below the points on the sheet");
        }
        // The overall dimensions measure the marker, not the annotations.
        assert_eq!(text_at(&sheet.page, "200").len(), 2);
        assert!(text_at(&sheet.page, "35000").is_empty());
        // The DXF carries the same annotations.
        assert_eq!(text_at(&sheet.lines, "5000").len(), 7);

        let annotations_alone = DrawingSetOptions {
            plans: plans(vec![Storey {
                floor_mm: None,
                cut_mm: 0.0,
            }]),
            ..options.clone()
        };
        let alone = drawing_set(&[], &annotations_alone).unwrap();
        assert_eq!(alone.sheets[0].annotations, sheet.annotations);
        assert_eq!(text_at(&alone.sheets[0].page, "5000").len(), 7);

        let too_fine = DrawingSetOptions {
            scale: Some(10),
            ..options
        };
        assert_eq!(
            drawing_set(&[marker], &too_fine),
            Err(ProjectDrawingError::DoesNotFit {
                scale: 10,
                format: None
            })
        );
    }

    /// R9-C step 14: the plan marks every window and door it shows, and the
    /// last sheet lists each mark with its designation and how many openings
    /// carry it; each sheet's DXF carries the same marks and table.
    #[test]
    fn the_plan_marks_windows_and_doors_and_the_last_sheet_lists_them() {
        let mut house = room();
        // Two equal windows in the south wall, a door in the east wall.
        house.extend([
            cuboid("w1/glass", [1000.0, 40.0, 1100.0], [2200.0, 110.0, 2350.0]),
            cuboid("w2/glass", [3500.0, 40.0, 1100.0], [4700.0, 110.0, 2350.0]),
            cuboid(
                "entry/leaf",
                [5890.0, 1000.0, 200.0],
                [5960.0, 1900.0, 2300.0],
            ),
        ]);
        let opening = |mark: &str, designation: &str, part: &str| DrawingOpening {
            mark: mark.to_owned(),
            designation: designation.to_owned(),
            parts: vec![part.to_owned()],
        };
        let mut options = options(&house, Some(1200.0));
        options.openings = OpeningSchedule {
            openings: vec![
                opening("W1", "window 1200 x 1250", "w1/glass"),
                opening("D1", "door 900 x 2100", "entry/leaf"),
                opening("W1", "window 1200 x 1250", "w2/glass"),
                // Hidden: listed, but no plan shows it.
                opening("W2", "window 600 x 600", "attic/glass"),
            ],
            title: "Windows and doors".to_owned(),
            columns: ["Mark", "Designation", "Count"].map(str::to_owned),
        };
        let set = drawing_set(&house, &options).unwrap();
        assert_eq!(
            set.sheets
                .iter()
                .map(|sheet| (sheet.kind, sheet.number))
                .collect::<Vec<_>>(),
            [
                (SheetKind::Plan { storey: 0 }, 1),
                (SheetKind::Sections, 2),
                (SheetKind::Elevations, 3),
                (SheetKind::Openings, 4),
            ]
        );

        let plan = sheet_of(&set, SheetKind::Plan { storey: 0 });
        assert_eq!(plan.openings, ["W1", "D1", "W1"]);
        for page in [&plan.page, &plan.lines] {
            assert_eq!(text_at(page, "W1").len(), 2);
            assert_eq!(text_at(page, "D1").len(), 1);
            assert!(text_at(page, "W2").is_empty());
        }
        // The door's mark stands in the middle of its leaf, right of the windows'.
        let [door] = text_at(&plan.page, "D1")[..] else {
            panic!("one door mark")
        };
        assert!(text_at(&plan.page, "W1").iter().all(|w| w[0] < door[0]));

        assert_eq!(
            options.openings.rows(),
            [
                ScheduleRow {
                    mark: "W1".to_owned(),
                    designation: "window 1200 x 1250".to_owned(),
                    count: 2
                },
                ScheduleRow {
                    mark: "D1".to_owned(),
                    designation: "door 900 x 2100".to_owned(),
                    count: 1
                },
                ScheduleRow {
                    mark: "W2".to_owned(),
                    designation: "window 600 x 600".to_owned(),
                    count: 1
                },
            ]
        );
        let schedule = sheet_of(&set, SheetKind::Openings);
        let texts = texts(&schedule.page);
        for wanted in [
            "Mark",
            "Designation",
            "Count",
            "window 1200 x 1250",
            "door 900 x 2100",
            "window 600 x 600",
            "2",
            "4 / 4",
            "Windows and doors",
            "Room <A>",
        ] {
            assert!(texts.contains(&wanted), "{wanted}: {texts:?}");
        }
        assert_eq!(schedule.lines, schedule.page);
        let dxf = set.dxf();
        assert_eq!(dxf.len(), 4);
        assert!(String::from_utf8_lossy(&dxf[3]).contains("window 1200 x 1250"));

        // The plans alone have no schedule.
        options.plan_only = true;
        let plans = drawing_set(&house, &options).unwrap();
        assert_eq!(plans.sheets.len(), 1);
        assert_eq!(plans.sheets[0].openings, ["W1", "D1", "W1"]);
    }

    /// R9-C step 15: a wall panel 2000 x 2500 framed in 60 x 140 timber,
    /// a window 600 x 1200 between two of its studs, sheathed with OSB.
    #[test]
    fn a_shop_drawing_numbers_every_piece_and_dimensions_the_studs_and_the_opening() {
        let stud =
            |name: &str, x: f64, top: f64| cuboid(name, [x, 0.0, 60.0], [x + 60.0, 140.0, top]);
        let lying =
            |name: &str, x: [f64; 2], z: f64| cuboid(name, [x[0], 0.0, z], [x[1], 140.0, z + 60.0]);
        let panel = vec![
            lying("p/bottom plate", [0.0, 2000.0], 0.0),
            lying("p/top plate", [0.0, 2000.0], 2440.0),
            stud("p/stud 1", 0.0, 2440.0),
            stud("p/stud 2", 600.0, 2440.0),
            stud("p/stud 3", 1940.0, 2440.0),
            stud("p/king 1", 840.0, 2000.0),
            stud("p/king 2", 1500.0, 2000.0),
            lying("p/header", [840.0, 1560.0], 2000.0),
            lying("p/sill", [900.0, 1500.0], 740.0),
            cuboid("p/osb 1", [0.0, -15.0, 0.0], [900.0, 0.0, 2500.0]),
            cuboid("p/osb 2", [1500.0, -15.0, 0.0], [2000.0, 0.0, 2500.0]),
        ];
        let position = |material: &str, dimensions_mm: [f64; 3], solids: &[&str]| ShopPosition {
            material: material.to_owned(),
            dimensions_mm,
            solids: solids.iter().map(|name| format!("p/{name}")).collect(),
            count: solids.len(),
        };
        let mut options = options(&panel, Some(1200.0));
        options.shop = ShopDrawings {
            sheets: vec![ShopSheet {
                title: "Panel p".to_owned(),
                right: [1.0, 0.0, 0.0],
                up: [0.0, 0.0, 1.0],
                positions: vec![
                    position("C24", [2000.0, 140.0, 60.0], &["bottom plate", "top plate"]),
                    position(
                        "C24",
                        [2380.0, 140.0, 60.0],
                        &["stud 1", "stud 2", "stud 3"],
                    ),
                    position("C24", [1940.0, 140.0, 60.0], &["king 1", "king 2"]),
                    position("C24", [720.0, 140.0, 60.0], &["header"]),
                    position("C24", [600.0, 140.0, 60.0], &["sill"]),
                    position("OSB", [2500.0, 900.0, 15.0], &["osb 1"]),
                    position("OSB", [2500.0, 500.0, 15.0], &["osb 2"]),
                ],
            }],
            columns: ["Pos.", "Material", "Size", "Count"].map(str::to_owned),
        };
        let set = drawing_set(&panel, &options).unwrap();
        assert_eq!(
            set.sheets
                .iter()
                .map(|sheet| sheet.kind)
                .collect::<Vec<_>>(),
            [
                SheetKind::Plan { storey: 0 },
                SheetKind::Sections,
                SheetKind::Elevations,
                SheetKind::Shop { drawing: 0 },
            ]
        );
        let sheet = sheet_of(&set, SheetKind::Shop { drawing: 0 });
        let shop = sheet.shop.as_ref().unwrap();
        // The studs' edges, and between the kings the window's 600.
        assert_eq!(
            shop.across,
            [
                0.0, 60.0, 600.0, 660.0, 840.0, 900.0, 1500.0, 1560.0, 1940.0, 2000.0
            ]
        );
        // The plates', and the window's sill top at 800 and header at 2000.
        assert_eq!(
            shop.up,
            [0.0, 60.0, 740.0, 800.0, 2000.0, 2060.0, 2440.0, 2500.0]
        );
        assert_eq!(
            shop.marks,
            ["1", "1", "2", "2", "2", "3", "3", "4", "5", "6", "7"]
        );
        for page in [&sheet.page, &sheet.lines] {
            // Its mark on every piece, and its row in the table.
            for (number, pieces) in [("1", 2), ("2", 3), ("3", 2), ("4", 1), ("6", 1)] {
                assert!(text_at(page, number).len() > pieces, "{number}");
            }
            let texts = texts(page);
            for wanted in [
                "Pos.",
                "2380 x 140 x 60",
                "2500 x 900 x 15",
                "OSB",
                "4 / 4",
                "Panel p",
                "1560",
            ] {
                assert!(texts.contains(&wanted), "{wanted}: {texts:?}");
            }
        }
        // Sheet and pieces at a shop scale, not the set's.
        assert!(set.scale > 25 && sheet.page.texts().any(|text| text == "1:25"));
        assert!(String::from_utf8_lossy(&set.dxf()[3]).contains("2500 x 500 x 15"));

        // The OSB lies in front of the framing, so it gets a view of its own
        // below: the framing view shows every stud, its edges drawn from plate
        // to plate (as the DXF carries them) and its number on it.
        assert_eq!(
            sheet
                .views
                .iter()
                .map(|view| view.title.as_str())
                .collect::<Vec<_>>(),
            ["Panel p", "Panel p - OSB"]
        );
        let shop = shop::plan_shop(&options.shop.sheets[0], &panel, &options.shop.columns, None)
            .unwrap()
            .unwrap();
        let views = shop.visible_lines();
        let drawn = |view: usize, x: f64, from: f64, to: f64| {
            views[view]
                .iter()
                .flat_map(|line| line.points.windows(2))
                .any(|pair| {
                    (pair[0][0] - x).abs() < 0.01
                        && (pair[1][0] - x).abs() < 0.01
                        && pair[0][1].min(pair[1][1]) <= from + 0.01
                        && pair[0][1].max(pair[1][1]) >= to - 0.01
                })
        };
        for x in [0.0, 60.0, 600.0, 660.0, 1940.0, 2000.0] {
            assert!(drawn(0, x, 60.0, 2440.0), "stud edge {x}");
        }
        for x in [840.0, 900.0, 1500.0, 1560.0] {
            assert!(drawn(0, x, 60.0, 2000.0), "king edge {x}");
        }
        for x in [0.0, 900.0, 1500.0, 2000.0] {
            assert!(drawn(1, x, 0.0, 2500.0), "osb edge {x}");
        }
        let numbers = |view: &DrawnView| {
            view.marks
                .iter()
                .map(|(_, number)| number.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            numbers(&shop.view),
            ["1", "1", "2", "2", "2", "3", "3", "4", "5"]
        );
        assert_eq!(numbers(&shop.layers[0].1), ["6", "7"]);

        options.plan_only = true;
        assert_eq!(drawing_set(&panel, &options).unwrap().sheets.len(), 1);
    }
}
