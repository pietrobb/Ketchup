//! The site plan (Měchenice MECH-10 (c)): the model seen from above on the shown underlays
//! around it, at 1:250 or 1:500, with grid crosses every 100 mm of paper labelled with
//! the coordinates of the underlay's own grid (S-JTSK in the Czech Republic), where those
//! coordinates lie on the underlay.

use super::{
    DrawingSolid, DrawnView, Point, ProjectDrawingError, ProjectView, SheetViews, SolidEdges,
    ViewFrame, draw_view, format_at_scale,
};
use crate::title_block::SheetFormat;
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, SheetImage, Stroke};
use serde::Serialize;
use std::sync::Arc;

/// The scales of a site plan, finest first.
pub const SITUATION_SCALES: [u32; 2] = [250, 500];
/// Paper between two grid crosses, and around the model.
const GRID_PAPER_MM: f64 = 100.0;
/// Half the length of a cross arm on the paper.
const CROSS_ARM_MM: f64 = 5.0;
// not a tolerance: the pen width of a grid cross.
const CROSS_STROKE_MM: f64 = 0.25;
const CROSS_TEXT_MM: f64 = 2.0;

/// What the caller decides of the site plan sheet.
#[derive(Clone, Debug)]
pub struct SituationSheet {
    pub title: String,
    pub view_title: String,
    /// The grid's own coordinates in millimetres (as the underlay drawing has them) of a
    /// point `[x, y]` of the model's ground plane: row `i` is `[a, b, c]` of
    /// `a·x + b·y + c`.
    pub to_grid_mm: [[f64; 3]; 2],
}

/// An image underlay, an orthophoto: its name, its pixels and its upper-left,
/// upper-right and lower-left corners in the world. Without pixels, as for a DXF or
/// DWG set, which holds none, it is laid out but left out of the sheet.
#[derive(Clone, Debug)]
pub struct UnderlayImage {
    pub name: String,
    pub image: Option<Arc<SheetImage>>,
    pub corners: [Point; 3],
}

/// A grid cross of the site plan.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SituationCross {
    /// What its labels say, in metres: Y and X of S-JTSK (both positive, EPSG:5514
    /// negated) for a grid in the negative quadrant, else x and y of the grid.
    pub coordinates_m: [f64; 2],
    /// Its centre on the sheet, in page millimetres (y down).
    pub at_mm: [f64; 2],
}

/// What the site plan sheet shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SituationSummary {
    pub scale: u32,
    pub crosses: Vec<SituationCross>,
}

/// A grid cross in a view: its centre in view millimetres, the direction of the grid's
/// x axis, its labels and their values.
pub(super) struct GridCross {
    pub(super) at: [f64; 2],
    pub(super) angle_deg: f64,
    pub(super) labels: [String; 2],
    pub(super) coordinates_m: [f64; 2],
}

/// The view of the site plan, its format and its scale: the finest of
/// [`SITUATION_SCALES`] at which it fits the requested format, else the smallest
/// format that holds it; `None` when no solid is drawn.
///
/// # Errors
/// [`ProjectDrawingError::DoesNotFit`] when it fits no format at any.
pub(super) fn plan(
    site: &SituationSheet,
    solids: &[DrawingSolid],
    edges: &[SolidEdges],
    (underlay, images): (&[[Point; 2]], &[UnderlayImage]),
    requested: Option<SheetFormat>,
) -> Result<Option<(SheetViews, SheetFormat, u32)>, ProjectDrawingError> {
    let frame = ViewFrame {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        depth: [0.0, 0.0, -1.0],
        cut: None,
    };
    let Some(inverse) = invert(site.to_grid_mm) else {
        return Ok(None);
    };
    // The requested format first; the 100 mm of paper around the model never fit
    // some (A3 at any scale), so then the smallest that holds the site plan.
    for wanted in std::iter::once(requested).chain(requested.map(|_| None)) {
        for scale in SITUATION_SCALES {
            let mut view = draw_view(ProjectView::Situation, frame, solids, edges);
            if !view.min[0].is_finite() {
                return Ok(None);
            }
            let step_mm = GRID_PAPER_MM * f64::from(scale);
            let [[a, b, _], [c, d, _]] = site.to_grid_mm;
            let margin = step_mm / (a * d - b * c).abs().sqrt();
            for axis in 0..2 {
                view.min[axis] -= margin;
                view.max[axis] += margin;
            }
            view.lay_underlay(&frame, underlay);
            view.lay_images(&frame, images);
            view.grid = crosses(&view, site.to_grid_mm, inverse, step_mm);
            let sheet = SheetViews {
                views: vec![view],
                rows: vec![vec![0]],
            };
            if let Ok(format) = format_at_scale(&sheet, wanted, scale) {
                return Ok(Some((sheet, format, scale)));
            }
        }
    }
    Err(ProjectDrawingError::DoesNotFit {
        scale: SITUATION_SCALES[SITUATION_SCALES.len() - 1],
        format: requested,
    })
}

fn apply(map: [[f64; 3]; 2], [x, y]: [f64; 2]) -> [f64; 2] {
    map.map(|[a, b, c]| a * x + b * y + c)
}

/// The inverse of an affine map of the plane, unless it flattens it.
fn invert([[a, b, c], [d, e, f]]: [[f64; 3]; 2]) -> Option<[[f64; 3]; 2]> {
    let det = a * e - b * d;
    if det.abs() < f64::EPSILON || !det.is_finite() {
        return None;
    }
    let (ia, ib, id, ie) = (e / det, -b / det, -d / det, a / det);
    Some([[ia, ib, -(ia * c + ib * f)], [id, ie, -(id * c + ie * f)]])
}

/// The crosses at every multiple of `step_mm` of the grid inside `view`.
fn crosses(
    view: &DrawnView,
    to_grid: [[f64; 3]; 2],
    from_grid: [[f64; 3]; 2],
    step_mm: f64,
) -> Vec<GridCross> {
    let corners = [
        view.min,
        [view.max[0], view.min[1]],
        [view.min[0], view.max[1]],
        view.max,
    ]
    .map(|corner| apply(to_grid, corner));
    let range = |axis: usize| {
        let low = corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f64::INFINITY, f64::min);
        let high = corners
            .iter()
            .map(|corner| corner[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        ((low / step_mm).ceil(), (high / step_mm).floor())
    };
    let origin = apply(from_grid, [0.0, 0.0]);
    let along = apply(from_grid, [1.0, 0.0]);
    let angle_deg = (along[1] - origin[1])
        .atan2(along[0] - origin[0])
        .to_degrees();
    let inside = |point: [f64; 2]| {
        (0..2).all(|axis| view.min[axis] <= point[axis] && point[axis] <= view.max[axis])
    };
    let ((first_x, last_x), (first_y, last_y)) = (range(0), range(1));
    let mut crosses = Vec::new();
    let mut gx = first_x;
    while gx <= last_x {
        let mut gy = first_y;
        while gy <= last_y {
            let grid = [gx * step_mm, gy * step_mm];
            let at = apply(from_grid, grid);
            if inside(at) {
                let (coordinates_m, labels) = labelled(grid.map(|value| value / 1000.0));
                crosses.push(GridCross {
                    at,
                    angle_deg,
                    labels,
                    coordinates_m,
                });
            }
            gy += 1.0;
        }
        gx += 1.0;
    }
    crosses
}

/// The values and labels of a cross at `grid` metres: S-JTSK drawings keep EPSG:5514's
/// negative x and y, while the map writes Y = −x and X = −y.
fn labelled([x, y]: [f64; 2]) -> ([f64; 2], [String; 2]) {
    let (values, names) = if x < 0.0 && y < 0.0 {
        ([-x, -y], ["Y", "X"])
    } else {
        ([x, y], ["x", "y"])
    };
    (
        values,
        [0, 1].map(|axis| format!("{} = {}", names[axis], grouped(values[axis]))),
    )
}

/// Whole metres with a space between thousands: "1 045 200".
fn grouped(value: f64) -> String {
    let digits = format!("{:.0}", value.abs());
    let mut text = String::from(if value < -0.5 { "-" } else { "" });
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            text.push(' ');
        }
        text.push(digit);
    }
    text
}

/// Draws `cross` at `at` on the page: two arms along the grid's axes and its two
/// labels beside them, along its x axis.
pub(super) fn draw_cross(page: &mut Page, cross: &GridCross, at: [f64; 2]) {
    let (sin, cos) = cross.angle_deg.to_radians().sin_cos();
    // The page's y runs down: the grid's x axis is (cos, −sin) on it, its y (−sin, −cos).
    let offset = |along: f64, up: f64| {
        [
            at[0] + along * cos - up * sin,
            at[1] - along * sin - up * cos,
        ]
    };
    for [along, up] in [[1.0, 0.0], [0.0, 1.0]] {
        page.line(
            vec![
                offset(-along * CROSS_ARM_MM, -up * CROSS_ARM_MM),
                offset(along * CROSS_ARM_MM, up * CROSS_ARM_MM),
            ],
            Stroke::solid(CROSS_STROKE_MM),
        );
    }
    for (label, up) in cross.labels.iter().zip([1.0, -1.0 - CROSS_TEXT_MM]) {
        page.marks.push(Mark::Text {
            at: offset(1.0, up),
            text: label.clone(),
            height_mm: CROSS_TEXT_MM,
            anchor: Anchor::Start,
            angle_deg: cross.angle_deg,
            bold: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s_jtsk_crosses_are_labelled_y_and_x_positive_with_grouped_thousands() {
        let (values, labels) = labelled([-742_300.0, -1_045_200.0]);
        assert_eq!(values, [742_300.0, 1_045_200.0]);
        assert_eq!(labels, ["Y = 742 300", "X = 1 045 200"]);
        assert_eq!(labelled([500.0, -25.0]).1, ["x = 500", "y = -25"]);
    }

    #[test]
    fn a_turned_scaled_map_inverts() {
        let (sin, cos) = 0.3_f64.sin_cos();
        let map = [[2.0 * cos, -2.0 * sin, 7.0], [2.0 * sin, 2.0 * cos, -3.0]];
        let inverse = invert(map).unwrap();
        let back = apply(inverse, apply(map, [11.0, -5.0]));
        assert!((back[0] - 11.0).abs() < 1e-12 && (back[1] + 5.0).abs() < 1e-12);
        assert!(invert([[1.0, 2.0, 0.0], [2.0, 4.0, 0.0]]).is_none());
    }
}
