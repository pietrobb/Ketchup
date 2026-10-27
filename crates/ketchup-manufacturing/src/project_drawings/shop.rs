//! Shop drawings on the drawing set: a sheet per group of parts the caller
//! names (a framed wall or floor build-up), seen square on. Each part carries its
//! position number, identical parts one number; running dimensions above the
//! view give the edges of the upright members (and between them the
//! openings) from the left edge, and on its right the edges of the lying
//! members (plates, sills, headers) from the bottom; a table beside the view
//! lists the positions.
//!
//! A member is a part no wider in the view than it is deep, so the sheets
//! and the infill between members are drawn and numbered but not dimensioned.

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
use ketchup_model::tolerance::{NEGLIGIBLE, ROUNDING};
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

/// A shop drawing laid out: its view and table, format and scale.
pub(super) struct PlannedShop {
    pub(super) title: String,
    pub(super) view: DrawnView,
    pub(super) format: SheetFormat,
    pub(super) scale: u32,
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

/// The view extent of `solid` in `frame` and how deep it is.
fn extents(frame: &ViewFrame, solid: &DrawingSolid) -> ([f64; 2], [f64; 2], f64) {
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
    (min, max, depth[1] - depth[0])
}

/// The page size the sheet takes at 1:`scale`: view, margins, gap and table.
fn page_size(view: &DrawnView, table: [f64; 2], scale: u32) -> [f64; 2] {
    let [width, height] = extent(view).map(|value| value / f64::from(scale));
    let [left, top, right, bottom] = MARGIN_MM;
    [
        left + width + right + GAP_MM + table[0],
        (top + height + bottom).max(table[1]),
    ]
}

fn fits(size: [f64; 2], format: SheetFormat) -> bool {
    let [[left, top], [right, bottom]] = drawing_area(format);
    size[0] <= right - left && size[1] <= bottom - top - TITLE_BLOCK_MM[1]
}

/// The requested format at the finest scale that holds the sheet, else the
/// smallest format holding it at 1:[`COARSEST_SCALE`] or finer, else A0.
fn layout(
    view: &DrawnView,
    table: [f64; 2],
    requested: Option<SheetFormat>,
) -> Result<(SheetFormat, u32), ProjectDrawingError> {
    let finest = |format: SheetFormat, coarsest: u32| {
        SCALES
            .into_iter()
            .filter(|scale| *scale <= coarsest)
            .find(|scale| fits(page_size(view, table, *scale), format))
            .map(|scale| (format, scale))
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
    let mut drawn = Vec::new();
    let mut numbers = Vec::new();
    for solid in solids {
        if let Some(index) = sheet
            .positions
            .iter()
            .position(|position| position.solids.contains(&solid.name))
        {
            drawn.push(solid.clone());
            numbers.push((index + 1).to_string());
        }
    }
    if drawn.is_empty() {
        return Ok(None);
    }
    let edges = drawn.iter().map(solid_edges).collect::<Vec<_>>();
    let mut view = draw_view(ProjectView::Shop, frame, &drawn, &edges);
    let Some((low, high)) = view.solids else {
        return Ok(None);
    };
    let mut chains = [vec![low[0], high[0]], vec![low[1], high[1]]];
    for (solid, number) in drawn.iter().zip(&numbers) {
        let (min, max, depth) = extents(&frame, solid);
        let size = [max[0] - min[0], max[1] - min[1]];
        if size[0].min(size[1]) <= depth * (1.0 + ROUNDING) {
            let axis = usize::from(size[1] <= size[0]);
            chains[axis].extend([min[axis], max[axis]]);
        }
        view.marks.push((
            [f64::midpoint(min[0], max[0]), f64::midpoint(min[1], max[1])],
            number.clone(),
        ));
    }
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
    let (format, scale) = layout(&view, table_size(columns, &rows), requested)?;
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
        format,
        scale,
        chains,
        rows,
    }))
}

impl PlannedShop {
    /// Draws the view with its dimensions and the table; with `visible`, the
    /// view as visible lines alone.
    pub(super) fn draw(
        &self,
        page: &mut Page,
        columns: &[String; 4],
        visible: Option<&[VisibleLine]>,
    ) {
        let scale = f64::from(self.scale);
        let [[area_left, area_top], _] = drawing_area(self.format);
        let [left, top, right, _] = MARGIN_MM;
        let [width, height] = extent(&self.view).map(|value| value / scale);
        let origin = [area_left + left, area_top + top + height];
        draw_view_on_page(page, &self.view, &self.title, origin, scale, visible);
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

        let table_left = area_left + left + width + right + GAP_MM;
        draw_table(page, [table_left, area_top], columns, &self.rows);
    }
}
