//! Shop drawings on the drawing set: a sheet per group of parts the caller
//! names (a framed wall or floor build-up), seen square on. Each part carries its
//! position number, identical parts one number; running dimensions above the
//! view give the edges of the upright members (and between them the
//! openings) from the left edge, and on its right the edges of the lying
//! members (plates, sills, headers) from the bottom; a table beside the view
//! lists the positions.
//!
//! A member is a part no wider in the view than it is deep, so the sheets
//! and the infill between members are drawn and numbered but not dimensioned;
//! nor is a strip of a sheet, a narrow part of the same material at the same
//! depths as a board of it.
//! The view shows the members and what lies at their depth (infill); a part
//! wholly in front of or behind them (a covering sheet) would hide them, so
//! the parts at each other depth get a view of their own below it.

use super::table::{draw_table, table_size};
use super::visible::VisibleLine;
use super::{
    DIMENSION_TEXT_MM, DrawingSolid, DrawnView, EDGE_STROKE_MM, Point, ProjectDrawingError,
    ProjectView, SCALES, ViewFrame, draw_view, draw_view_on_page, extent, solid_edges, tick,
    whole_mm,
};
use crate::sheet_pdf::{Anchor, Mark, Page, Stroke};
use crate::title_block::{SheetFormat, TITLE_BLOCK_MM, drawing_area};
use ketchup_geometry::linalg::{cross, dot, normalize_within};
use ketchup_model::tolerance::{HIDDEN_LINE_TOUCH_MM, NEGLIGIBLE, ROUNDING};
use serde::Serialize;

/// Room around the view on the page, in mm: left for its overall height,
/// above and right for the running dimensions, below for its overall width
/// and title.
const MARGIN_MM: [f64; 4] = [20.0, 22.0, 25.0, 24.0];
/// Between the view and the table.
const GAP_MM: f64 = 10.0;
/// Of the running dimension lines from the view.
const CHAIN_OFFSET_MM: f64 = 8.0;
/// The coarsest scale a shop drawing takes on an automatic format.
const COARSEST_SCALE: u32 = 50;

/// The identical parts of one position: drawn as `solids`, `count` of them
/// made (some may be hidden).
#[derive(Clone, Debug, PartialEq)]
pub struct ShopPosition {
    pub material: String,
    /// The blank, longest first.
    pub dimensions_mm: [f64; 3],
    pub solids: Vec<String>,
    pub count: usize,
}

/// One shop drawing: its title, the world directions its view shows to the
/// right and up, and its positions, numbered from 1.
#[derive(Clone, Debug, PartialEq)]
pub struct ShopSheet {
    pub title: String,
    pub right: Point,
    pub up: Point,
    pub positions: Vec<ShopPosition>,
}

/// The shop drawings and the captions of their tables' position, material,
/// size and count columns.
#[derive(Clone, Debug, Default)]
pub struct ShopDrawings {
    pub sheets: Vec<ShopSheet>,
    pub columns: [String; 4],
}

/// What a shop drawing shows: its running dimensions, in mm from the left and
/// from the bottom edge of its parts, and the position number of each part it
/// draws.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ShopSummary {
    pub across: Vec<f64>,
    pub up: Vec<f64>,
    pub marks: Vec<String>,
}

/// A shop drawing laid out: its view, the views of its covering layers (each
/// with its title), its table, format and scale.
pub(super) struct PlannedShop {
    pub(super) title: String,
    pub(super) view: DrawnView,
    pub(super) layers: Vec<(String, DrawnView)>,
    pub(super) format: SheetFormat,
    pub(super) scale: u32,
    /// The views side by side, else one below the other.
    beside: bool,
    pub(super) summary: ShopSummary,
    /// The running dimensions in view coordinates.
    chains: [Vec<f64>; 2],
    rows: Vec<Vec<String>>,
}

/// `values` sorted, those printing as the same whole millimetre once.
fn distinct(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64::total_cmp);
    values.dedup_by(|value, kept| whole_mm(*value) == whole_mm(*kept));
    values
}

/// The view extent of `solid` in `frame` and the depths it spans.
fn extents(frame: &ViewFrame, solid: &DrawingSolid) -> ([f64; 2], [f64; 2], [f64; 2]) {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut depth = [f64::INFINITY, f64::NEG_INFINITY];
    for point in solid.triangles.iter().flatten() {
        let projected = frame.project(*point);
        for axis in 0..2 {
            min[axis] = min[axis].min(projected[axis]);
            max[axis] = max[axis].max(projected[axis]);
        }
        let along = dot(*point, frame.depth);
        depth = [depth[0].min(along), depth[1].max(along)];
    }
    (min, max, depth)
}

/// Whether two depth spans share more than a touching face.
fn overlap(a: [f64; 2], b: [f64; 2]) -> bool {
    a[1].min(b[1]) - a[0].max(b[0]) > HIDDEN_LINE_TOUCH_MM
}

/// The view of the members first, then those of the covering layers.
fn views<'a>(
    view: &'a DrawnView,
    layers: &'a [(String, DrawnView)],
) -> impl Iterator<Item = &'a DrawnView> {
    std::iter::once(view).chain(layers.iter().map(|(_, layer)| layer))
}

/// The views at 1:`scale` side by side (`beside`) or one below the other:
/// each view's lower-left corner from the top left of the drawing area, and
/// the size they take with their margins.
fn arrange<'a>(
    views: impl Iterator<Item = &'a DrawnView>,
    scale: u32,
    beside: bool,
) -> (Vec<[f64; 2]>, [f64; 2]) {
    let [left, top, right, bottom] = MARGIN_MM;
    let mut origins = Vec::new();
    let mut size = [0.0_f64; 2];
    for view in views {
        let [width, height] = extent(view).map(|value| value / f64::from(scale));
        let first = origins.is_empty();
        let origin = if beside {
            [
                if first { 0.0 } else { size[0] + GAP_MM } + left,
                top + height,
            ]
        } else {
            [left, if first { top } else { size[1] + GAP_MM } + height]
        };
        origins.push(origin);
        size = [
            size[0].max(origin[0] + width + right),
            size[1].max(origin[1] + bottom),
        ];
    }
    (origins, size)
}

/// The page size the sheet takes: the views as arranged, the gap and the
/// table.
fn page_size(views: [f64; 2], table: [f64; 2]) -> [f64; 2] {
    [views[0] + GAP_MM + table[0], views[1].max(table[1])]
}

fn fits(size: [f64; 2], format: SheetFormat) -> bool {
    let [[left, top], [right, bottom]] = drawing_area(format);
    size[0] <= right - left && size[1] <= bottom - top - TITLE_BLOCK_MM[1]
}

/// The requested format at the finest scale that holds the sheet, else the
/// smallest format holding it at 1:[`COARSEST_SCALE`] or finer, else A0; and
/// whether its views stand side by side there (else one below the other).
fn layout(
    view: &DrawnView,
    layers: &[(String, DrawnView)],
    table: [f64; 2],
    requested: Option<SheetFormat>,
) -> Result<(SheetFormat, u32, bool), ProjectDrawingError> {
    let finest = |format: SheetFormat, coarsest: u32| {
        SCALES
            .into_iter()
            .filter(|scale| *scale <= coarsest)
            .find_map(|scale| {
                [false, true]
                    .into_iter()
                    .find(|beside| {
                        let (_, size) = arrange(views(view, layers), scale, *beside);
                        fits(page_size(size, table), format)
                    })
                    .map(|beside| (format, scale, beside))
            })
    };
    let any = SCALES[SCALES.len() - 1];
    match requested {
        Some(format) => finest(format, any),
        None => SheetFormat::ALL
            .into_iter()
            .find_map(|format| finest(format, COARSEST_SCALE))
            .or_else(|| finest(SheetFormat::A0, any)),
    }
    .ok_or(ProjectDrawingError::DoesNotFit {
        scale: any,
        format: requested,
    })
}

/// A part the shop drawing draws: its position number and material, its view
/// extent and the depths it spans.
struct Piece<'a> {
    solid: &'a DrawingSolid,
    number: String,
    material: &'a str,
    min: [f64; 2],
    max: [f64; 2],
    depth: [f64; 2],
}

impl Piece<'_> {
    fn size(&self) -> [f64; 2] {
        [self.max[0] - self.min[0], self.max[1] - self.min[1]]
    }

    fn thickness(&self) -> f64 {
        self.depth[1] - self.depth[0]
    }

    /// No wider in the view than it is deep: an upright or lying member, or a
    /// strip of a sheet.
    fn narrow(&self) -> bool {
        let size = self.size();
        size[0].min(size[1]) <= self.thickness() * (1.0 + ROUNDING)
    }

    /// At least [`BOARD_WIDTH_PER_THICKNESS`] times as wide in the view as it
    /// is thick: a sheet, not a deep header.
    fn board(&self) -> bool {
        let size = self.size();
        size[0].min(size[1]) >= self.thickness() * BOARD_WIDTH_PER_THICKNESS
    }

    /// Of this material at these depths: a piece of the same layer.
    fn same_layer(&self, other: &Piece) -> bool {
        self.material == other.material
            && (0..2).all(|end| (self.depth[end] - other.depth[end]).abs() <= HIDDEN_LINE_TOUCH_MM)
    }
}

/// How many times a board is at least as wide as it is thick. A header of
/// two 240 mm planks in a 140 mm frame (1.7) is no board; the narrowest OSB
/// or plywood sheet a layout keeps (PG-60) is well past it.
const BOARD_WIDTH_PER_THICKNESS: f64 = 4.0;

/// Whether each piece is a member: narrow, and not a strip of a sheet, which
/// shares its material and depths with a board.
fn members(pieces: &[Piece]) -> Vec<bool> {
    pieces
        .iter()
        .map(|piece| {
            piece.narrow()
                && !pieces
                    .iter()
                    .any(|other| other.board() && other.same_layer(piece))
        })
        .collect()
}

/// The pieces at the members' depth (all of them when there are no members),
/// and the others grouped by the depths they share, near to far.
fn by_depth(pieces: &[Piece], members: &[bool]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let members = pieces
        .iter()
        .zip(members)
        .filter(|(_, member)| **member)
        .map(|(piece, _)| piece.depth)
        .collect::<Vec<_>>();
    let mut order = (0..pieces.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| pieces[*a].depth[0].total_cmp(&pieces[*b].depth[0]));
    let mut framed = Vec::new();
    let mut groups: Vec<([f64; 2], Vec<usize>)> = Vec::new();
    for index in order {
        let depth = pieces[index].depth;
        if members.is_empty() || members.iter().any(|member| overlap(*member, depth)) {
            framed.push(index);
        } else if let Some((span, group)) =
            groups.last_mut().filter(|(span, _)| overlap(*span, depth))
        {
            span[1] = span[1].max(depth[1]);
            group.push(index);
        } else {
            groups.push((depth, vec![index]));
        }
    }
    framed.sort_unstable();
    for (_, group) in &mut groups {
        group.sort_unstable();
    }
    (framed, groups.into_iter().map(|(_, group)| group).collect())
}

/// The view of `pieces[indices]`, each with its position number.
fn piece_view(frame: ViewFrame, pieces: &[Piece], indices: &[usize]) -> DrawnView {
    let solids = indices
        .iter()
        .map(|index| pieces[*index].solid.clone())
        .collect::<Vec<_>>();
    let edges = solids.iter().map(solid_edges).collect::<Vec<_>>();
    let mut view = draw_view(ProjectView::Shop, frame, &solids, &edges);
    view.marks = indices
        .iter()
        .map(|index| {
            let piece = &pieces[*index];
            (
                [
                    f64::midpoint(piece.min[0], piece.max[0]),
                    f64::midpoint(piece.min[1], piece.max[1]),
                ],
                piece.number.clone(),
            )
        })
        .collect();
    view
}

/// Lays out the shop drawing of `sheet` from the `solids` it names; `None`
/// when none of them is drawn.
pub(super) fn plan_shop(
    sheet: &ShopSheet,
    solids: &[DrawingSolid],
    columns: &[String; 4],
    requested: Option<SheetFormat>,
) -> Result<Option<PlannedShop>, ProjectDrawingError> {
    let (Some(right), Some(up)) = (
        normalize_within(sheet.right, NEGLIGIBLE),
        normalize_within(sheet.up, NEGLIGIBLE),
    ) else {
        return Ok(None);
    };
    let frame = ViewFrame {
        right,
        up,
        depth: cross(up, right),
        cut: None,
    };
    let mut pieces = Vec::new();
    for solid in solids {
        if let Some(index) = sheet
            .positions
            .iter()
            .position(|position| position.solids.contains(&solid.name))
        {
            let (min, max, depth) = extents(&frame, solid);
            pieces.push(Piece {
                solid,
                number: (index + 1).to_string(),
                material: &sheet.positions[index].material,
                min,
                max,
                depth,
            });
        }
    }
    if pieces.is_empty() {
        return Ok(None);
    }
    let members = members(&pieces);
    let (framed, groups) = by_depth(&pieces, &members);
    let view = piece_view(frame, &pieces, &framed);
    let Some((low, high)) = view.solids else {
        return Ok(None);
    };
    let layers = groups
        .iter()
        .map(|group| {
            let mut materials: Vec<&str> = Vec::new();
            for index in group {
                if !materials.contains(&pieces[*index].material) {
                    materials.push(pieces[*index].material);
                }
            }
            (
                format!("{} - {}", sheet.title, materials.join(", ")),
                piece_view(frame, &pieces, group),
            )
        })
        .collect::<Vec<_>>();
    let mut chains = [vec![low[0], high[0]], vec![low[1], high[1]]];
    for (piece, _) in pieces.iter().zip(&members).filter(|(_, member)| **member) {
        let size = piece.size();
        let axis = usize::from(size[1] <= size[0]);
        chains[axis].extend([piece.min[axis], piece.max[axis]]);
    }
    let numbers = pieces
        .iter()
        .map(|piece| piece.number.clone())
        .collect::<Vec<_>>();
    let chains = chains.map(distinct);
    let rows = sheet
        .positions
        .iter()
        .enumerate()
        .map(|(index, position)| {
            vec![
                (index + 1).to_string(),
                position.material.clone(),
                position.dimensions_mm.map(whole_mm).join(" x "),
                position.count.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    let (format, scale, beside) = layout(&view, &layers, table_size(columns, &rows), requested)?;
    let from = |values: &[f64], origin: f64| {
        values
            .iter()
            .map(|value| (value - origin).round())
            .collect::<Vec<_>>()
    };
    Ok(Some(PlannedShop {
        title: sheet.title.clone(),
        summary: ShopSummary {
            across: from(&chains[0], low[0]),
            up: from(&chains[1], low[1]),
            marks: numbers,
        },
        view,
        layers,
        format,
        scale,
        beside,
        chains,
        rows,
    }))
}

impl PlannedShop {
    /// The visible lines of each view, the members' first.
    pub(super) fn visible_lines(&self) -> Vec<Vec<VisibleLine>> {
        views(&self.view, &self.layers)
            .map(super::visible::visible_lines)
            .collect()
    }

    /// Draws the views, the first with its dimensions, and the table; with
    /// `visible` (per view, as [`Self::visible_lines`] gives them), the views
    /// as visible lines alone.
    pub(super) fn draw(
        &self,
        page: &mut Page,
        columns: &[String; 4],
        visible: Option<&[Vec<VisibleLine>]>,
    ) {
        let scale = f64::from(self.scale);
        let [[area_left, area_top], _] = drawing_area(self.format);
        let (origins, size) = arrange(views(&self.view, &self.layers), self.scale, self.beside);
        let origins = origins
            .into_iter()
            .map(|[x, y]| [area_left + x, area_top + y])
            .collect::<Vec<_>>();
        let titles = std::iter::once(&self.title).chain(self.layers.iter().map(|(title, _)| title));
        for (index, (view, title)) in views(&self.view, &self.layers).zip(titles).enumerate() {
            let lines = visible.map(|views| views[index].as_slice());
            draw_view_on_page(page, view, title, origins[index], scale, lines);
        }
        let origin = origins[0];
        let min = self.view.min;
        let at = |point: [f64; 2]| {
            [
                origin[0] + (point[0] - min[0]) / scale,
                origin[1] - (point[1] - min[1]) / scale,
            ]
        };
        let Some((low, high)) = self.view.solids else {
            return;
        };
        let thin = Stroke::solid(EDGE_STROKE_MM);
        let [view_right, view_top] = at(high);

        // Across, above the view: each edge from the left one, read upward.
        let line = view_top - CHAIN_OFFSET_MM;
        let xs = self.chains[0]
            .iter()
            .map(|x| at([*x, low[1]])[0])
            .collect::<Vec<_>>();
        if let (Some(first), Some(last)) = (xs.first(), xs.last()) {
            page.line(vec![[first - 2.0, line], [last + 2.0, line]], thin.clone());
        }
        for (x, value) in xs.iter().zip(&self.chains[0]) {
            page.line(vec![[*x, view_top - 2.0], [*x, line - 2.0]], thin.clone());
            tick(page, [*x, line]);
            page.marks.push(Mark::Text {
                at: [x + DIMENSION_TEXT_MM / 2.0, line - 1.5],
                text: whole_mm(value - low[0]),
                height_mm: DIMENSION_TEXT_MM,
                anchor: Anchor::Start,
                angle_deg: 90.0,
                bold: false,
            });
        }

        // Up, right of the view: each edge from the bottom one.
        let line = view_right + CHAIN_OFFSET_MM;
        let ys = self.chains[1]
            .iter()
            .map(|y| at([low[0], *y])[1])
            .collect::<Vec<_>>();
        if let (Some(first), Some(last)) = (ys.first(), ys.last()) {
            page.line(vec![[line, first + 2.0], [line, last - 2.0]], thin.clone());
        }
        for (y, value) in ys.iter().zip(&self.chains[1]) {
            page.line(vec![[view_right + 2.0, *y], [line + 2.0, *y]], thin.clone());
            tick(page, [line, *y]);
            page.text(
                [line + 2.0, y + DIMENSION_TEXT_MM / 2.0],
                &whole_mm(value - low[1]),
                DIMENSION_TEXT_MM,
                Anchor::Start,
            );
        }

        let table_left = area_left + size[0] + GAP_MM;
        draw_table(page, [table_left, area_top], columns, &self.rows);
    }
}
