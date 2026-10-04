//! Project drawings of a model: a floor plan cut horizontally, a longitudinal and
//! a cross section and the four elevations, laid out on one sheet at a common
//! scale. The parts the cutting plane passes through are filled and outlined
//! heavily; each view carries its overall dimensions.
//!
//! The sheet is an ISO A format with a frame and a title block in the lower
//! right corner, written as a vector PDF.
//!
//! The input is any set of closed triangulated solids in world millimetres (z up),
//! so the sheet follows whatever is visible: hiding a layer leaves it out.
//! Lines behind nearer faces are hidden by painting faces from far to near.

use crate::sheet_pdf::{Anchor, Mark, Page, PdfInfo, Stroke, page_pdf};
use crate::title_block::{
    SheetFormat, TITLE_BLOCK_MM, TitleBlock, TitleField, draw_frame_and_title_block, drawing_area,
};
use ketchup_geometry::linalg::{Vec3, cross, dot, normalize_within, sub};
use serde::Serialize;
use std::collections::BTreeMap;

type Point = [f64; 3];
/// A closed outline in view millimetres.
type Polyline = Vec<[f64; 2]>;

/// One solid to draw: its triangles in world millimetres and its colour.
#[derive(Clone, Debug)]
pub struct DrawingSolid {
    pub name: String,
    pub color: [u8; 3],
    pub triangles: Vec<[Point; 3]>,
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
}

impl ProjectView {
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

/// What the caller decides: view titles, the plan cut, the sheet format and
/// the title block.
#[derive(Clone, Debug, Default)]
pub struct ProjectSheetOptions {
    pub view_titles: BTreeMap<ProjectView, String>,
    /// Height of the horizontal plan cut; [`default_plan_cut_z`] when absent.
    pub plan_cut_z_mm: Option<f64>,
    /// The sheet format; when absent the smallest one that holds the views at
    /// 1:50 or finer.
    pub format: Option<SheetFormat>,
    /// Captions of the title block cells.
    pub title_labels: BTreeMap<TitleField, String>,
    /// The title block values; scale and format are filled in by the sheet.
    pub title_block: TitleBlock,
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

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectSheet {
    pub page: Page,
    /// The sheet scale as 1:`scale`.
    pub scale: u32,
    pub format: SheetFormat,
    pub views: Vec<ViewSummary>,
}

impl ProjectSheet {
    /// The sheet as a vector PDF with its typeface embedded.
    #[must_use]
    pub fn pdf(&self, info: &PdfInfo) -> Vec<u8> {
        page_pdf(&self.page, info)
    }
}

/// Why no sheet could be drawn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectDrawingError {
    /// There is nothing visible to draw.
    Empty,
}

impl std::fmt::Display for ProjectDrawingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "there are no visible solids to draw",
        })
    }
}

impl std::error::Error for ProjectDrawingError {}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Vec3::from(a).lerp(Vec3::from(b), t).into()
}

fn unit_normal([a, b, c]: &[Point; 3]) -> Option<Point> {
    normalize_within(cross(sub(*b, *a), sub(*c, *a)), 1e-12)
}

/// A vertex position on a 0.01 mm grid, for matching shared edges.
fn grid_key(point: Point) -> [i64; 3] {
    #[allow(clippy::cast_possible_truncation)]
    point.map(|value| (value * 100.0).round() as i64)
}

/// Edges where the surface folds (or ends): the lines a drawing shows.
fn feature_edges(solid: &DrawingSolid) -> Vec<[Point; 2]> {
    // Per shared edge (keyed by its grid ends): its ends and the normals of its faces.
    let mut edges: BTreeMap<[[i64; 3]; 2], (Point, Point, Vec<Point>)> = BTreeMap::new();
    for triangle in &solid.triangles {
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
                .push(normal);
        }
    }
    // Faces meeting at more than about 20° show their edge.
    let fold = 20_f64.to_radians().cos();
    edges
        .into_values()
        .filter(|(_, _, normals)| match normals.as_slice() {
            [first, second] => dot(*first, *second) < fold,
            _ => true,
        })
        .map(|(a, b, _)| [a, b])
        .collect()
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
    if (db - da).abs() > 1e-9 {
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
            loops.push(polyline);
        }
    }
    loops
}

enum Item {
    Face {
        points: Vec<[f64; 2]>,
        color: [u8; 3],
    },
    Edge([[f64; 2]; 2]),
}

struct DrawnView {
    view: ProjectView,
    /// Far-to-near faces and edges.
    items: Vec<Item>,
    /// Cut fill per solid: loops and colour.
    cuts: Vec<(Vec<Polyline>, [u8; 3])>,
    min: [f64; 2],
    max: [f64; 2],
    cut_at_mm: Option<f64>,
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
    edges: &[Vec<[Point; 2]>],
) -> DrawnView {
    let mut sorted: Vec<(f64, Item)> = Vec::new();
    let mut cuts = Vec::new();
    for (solid, solid_edges) in solids.iter().zip(edges) {
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
            if frame.cut.is_none() && dot(normal, frame.depth) > 1e-9 {
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
        for edge in solid_edges {
            let Some(edge) = clip_segment(&frame, *edge) else {
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
        }
    }
    DrawnView {
        view,
        items: sorted.into_iter().map(|(_, item)| item).collect(),
        cuts,
        min,
        max,
        cut_at_mm: frame.cut.map(f64::abs),
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
/// horizontal side is the length: the longitudinal section runs along it.
fn view_frames(min: Point, max: Point, plan_cut_z: f64) -> [(ProjectView, ViewFrame); 7] {
    let centre = lerp(min, max, 0.5);
    let along_x = max[0] - min[0] >= max[1] - min[1];
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

const PLAN_CUT_ABOVE_LOWEST_MM: f64 = 1200.0;
/// Room around a view for its dimensions and title, in page millimetres.
const VIEW_PAD_MM: f64 = 22.0;
const SCALES: [u32; 10] = [10, 20, 25, 50, 75, 100, 200, 250, 500, 1000];
/// The coarsest scale a format is chosen for: building drawings are read at 1:50.
const PREFERRED_SCALE: u32 = 50;

const EDGE_STROKE_MM: f64 = 0.18;
const EDGE_COLOR: [u8; 3] = [34, 34, 34];
const CUT_STROKE_MM: f64 = 0.5;
const DIMENSION_TEXT_MM: f64 = 2.5;
const VIEW_TITLE_MM: f64 = 4.0;

/// The view rows: cut views, then elevations.
fn rows(views: &[DrawnView]) -> [Vec<usize>; 2] {
    let (cut, elevation): (Vec<usize>, Vec<usize>) =
        (0..views.len()).partition(|index| views[*index].view.is_cut());
    [cut, elevation]
}

fn extent(view: &DrawnView) -> [f64; 2] {
    [view.max[0] - view.min[0], view.max[1] - view.min[1]]
}

/// The largest standard scale at which both rows fit the drawing area of
/// `format` above the title block, if any does.
fn choose_scale(views: &[DrawnView], format: SheetFormat) -> Option<u32> {
    let rows = rows(views);
    let [[left, top], [right, bottom]] = drawing_area(format);
    let usable_width = right - left;
    let usable_height = bottom - top - TITLE_BLOCK_MM[1];
    SCALES.into_iter().find(|scale| {
        let scale = f64::from(*scale);
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
    })
}

/// The requested format at its best scale, or the smallest format that holds
/// the views at 1:50 or finer (A0 at whatever fits when none does).
fn choose_layout(views: &[DrawnView], requested: Option<SheetFormat>) -> (SheetFormat, u32) {
    let coarsest = SCALES[SCALES.len() - 1];
    if let Some(format) = requested {
        return (format, choose_scale(views, format).unwrap_or(coarsest));
    }
    SheetFormat::ALL
        .into_iter()
        .find_map(|format| {
            choose_scale(views, format)
                .filter(|scale| *scale <= PREFERRED_SCALE)
                .map(|scale| (format, scale))
        })
        .unwrap_or_else(|| {
            let format = SheetFormat::A0;
            (format, choose_scale(views, format).unwrap_or(coarsest))
        })
}

/// A short oblique tick across a dimension line end, as architects draw it.
fn tick(page: &mut Page, [x, y]: [f64; 2]) {
    page.line(
        vec![[x - 1.2, y + 1.2], [x + 1.2, y - 1.2]],
        Stroke::solid(0.35),
    );
}

/// Draws one view with its lower-left model corner at `origin` (page mm).
fn draw_view_on_page(page: &mut Page, view: &DrawnView, title: &str, origin: [f64; 2], scale: f64) {
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
    for item in &view.items {
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
    let [width, height] = extent(view);
    let [left, bottom] = at(view.min);
    let [right, top] = at(view.max);
    let thin = Stroke::solid(EDGE_STROKE_MM);
    // Overall width below the view and overall height on its left.
    let below = bottom + 8.0;
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
        &format!("{width:.0}"),
        DIMENSION_TEXT_MM,
        Anchor::Middle,
    );
    let aside = left - 8.0;
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
        text: format!("{height:.0}"),
        height_mm: DIMENSION_TEXT_MM,
        anchor: Anchor::Middle,
        angle_deg: 90.0,
        bold: false,
    });
    page.marks.push(Mark::Text {
        at: [left, below + 8.0],
        text: title.to_owned(),
        height_mm: VIEW_TITLE_MM,
        anchor: Anchor::Start,
        angle_deg: 0.0,
        bold: true,
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

/// The usual plan cut: 1.2 m above the lowest point (about a metre above a
/// floor on its slab), or the middle of a model lower than 2.4 m.
#[must_use]
pub fn default_plan_cut_z(solids: &[DrawingSolid]) -> Option<f64> {
    bounds(solids)
        .map(|(min, max)| (min[2] + PLAN_CUT_ABOVE_LOWEST_MM).min(f64::midpoint(min[2], max[2])))
}

/// Draws the project sheet of `solids`.
///
/// # Errors
/// [`ProjectDrawingError::Empty`] when there is no triangle to draw.
pub fn project_sheet(
    solids: &[DrawingSolid],
    options: &ProjectSheetOptions,
) -> Result<ProjectSheet, ProjectDrawingError> {
    let (min, max) = bounds(solids).ok_or(ProjectDrawingError::Empty)?;
    let plan_cut_z = options
        .plan_cut_z_mm
        .or_else(|| default_plan_cut_z(solids))
        .ok_or(ProjectDrawingError::Empty)?;
    let edges = solids.iter().map(feature_edges).collect::<Vec<_>>();
    let views = view_frames(min, max, plan_cut_z)
        .into_iter()
        .map(|(view, frame)| draw_view(view, frame, solids, &edges))
        .filter(|view| view.min[0].is_finite())
        .collect::<Vec<_>>();
    let (format, scale) = choose_layout(&views, options.format);
    let title_of = |view: ProjectView| {
        options
            .view_titles
            .get(&view)
            .cloned()
            .unwrap_or_else(|| format!("{view:?}"))
    };

    let mut page = Page {
        size_mm: format.size_mm(),
        marks: Vec::new(),
    };
    let scale_f = f64::from(scale);
    let [[area_left, mut top], _] = drawing_area(format);
    for row in rows(&views) {
        let row_height = row
            .iter()
            .map(|index| extent(&views[*index])[1] / scale_f + 2.0 * VIEW_PAD_MM)
            .fold(0.0, f64::max);
        let mut left = area_left;
        for index in row {
            let view = &views[index];
            let [width, height] = extent(view);
            let origin = [
                left + VIEW_PAD_MM,
                top + VIEW_PAD_MM / 2.0 + height / scale_f,
            ];
            draw_view_on_page(&mut page, view, &title_of(view.view), origin, scale_f);
            left += width / scale_f + 2.0 * VIEW_PAD_MM;
        }
        top += row_height;
    }
    let mut values = options.title_block.clone();
    values.insert(TitleField::Scale, format!("1:{scale}"));
    values.insert(TitleField::Format, format.name().to_owned());
    draw_frame_and_title_block(&mut page, format, &options.title_labels, &values);

    let summaries = views
        .iter()
        .map(|view| ViewSummary {
            view: view.view,
            title: title_of(view.view),
            cut_solids: view.cuts.len(),
            cut_at_mm: view.cut_at_mm,
            extent_mm: extent(view).map(|value| (value * 10.0).round() / 10.0),
        })
        .collect();
    Ok(ProjectSheet {
        page,
        scale,
        format,
        views: summaries,
    })
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
        }
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

    fn sheet(plan_cut_z_mm: Option<f64>) -> ProjectSheet {
        project_sheet(
            &room(),
            &ProjectSheetOptions {
                view_titles: BTreeMap::from([(ProjectView::Plan, "Floor plan".to_owned())]),
                plan_cut_z_mm,
                title_labels: BTreeMap::from([(TitleField::Project, "Stavba".to_owned())]),
                title_block: TitleBlock::from([(TitleField::Project, "Room <A>".to_owned())]),
                ..ProjectSheetOptions::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn plan_and_sections_fill_the_parts_the_plane_cuts() {
        let sheet = sheet(Some(1200.0));
        let cut = |view| {
            sheet
                .views
                .iter()
                .find(|summary| summary.view == view)
                .unwrap()
        };
        let plan = cut(ProjectView::Plan);
        assert_eq!(plan.cut_solids, 4, "the four walls, not the floor");
        assert_eq!(plan.cut_at_mm, Some(1200.0));
        assert_eq!(plan.extent_mm, [6000.0, 3000.0]);
        // Along the 6 m length: floor plus the two end walls; across: floor and side walls.
        assert_eq!(cut(ProjectView::LongitudinalSection).cut_solids, 3);
        assert_eq!(cut(ProjectView::CrossSection).cut_solids, 3);
        assert_eq!(cut(ProjectView::CrossSection).extent_mm, [3000.0, 2700.0]);
        assert_eq!(cut(ProjectView::Front).cut_solids, 0);
        assert_eq!(cut(ProjectView::Front).extent_mm, [6000.0, 2700.0]);
        assert_eq!(sheet.views.len(), 7);
        let cut_fills = sheet
            .page
            .marks
            .iter()
            .filter(|mark| {
                matches!(mark, Mark::Fill { outline: Some(stroke), .. } if stroke.width_mm == CUT_STROKE_MM)
            })
            .count();
        assert_eq!(cut_fills, 10);
        let texts = sheet.page.texts().collect::<Vec<_>>();
        assert!(texts.contains(&"6000"));
        assert!(texts.contains(&"2700"));
        assert!(texts.contains(&"Floor plan"));
        assert!(texts.contains(&"Room <A>"));
        assert!(texts.contains(&"Stavba"));
        assert!(texts.contains(&format!("1:{}", sheet.scale).as_str()));
        assert!(texts.contains(&sheet.format.name()));
        assert_eq!(sheet.scale, 50, "a room is drawn at the building scale");
        assert_eq!(
            sheet.format,
            SheetFormat::A2,
            "the smallest sheet that holds it at 1:50"
        );
        assert_eq!(sheet.page.size_mm, SheetFormat::A2.size_mm());
        let pdf = sheet.pdf(&PdfInfo::default());
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn a_plan_cut_above_everything_shows_the_roofless_room_from_above() {
        let sheet = sheet(Some(5000.0));
        assert_eq!(sheet.views[0].cut_solids, 0);
        assert_eq!(sheet.views[0].extent_mm, [6000.0, 3000.0]);
        let edges = sheet
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

    #[test]
    fn nothing_visible_is_an_error() {
        assert_eq!(
            project_sheet(&[], &ProjectSheetOptions::default()),
            Err(ProjectDrawingError::Empty)
        );
    }
}
