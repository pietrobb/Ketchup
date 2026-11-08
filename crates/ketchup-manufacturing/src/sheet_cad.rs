//! A sheet of the project drawings as a CAD drawing someone keeps working on in AutoCAD,
//! BricsCAD or DraftSight ([`ketchup_dwg::sheet`]): each view in model space at 1:1 in
//! millimetres, its marks on layers named by what they show ([`LAYERS`]), and the sheet as a
//! paper-space layout of its format with the frame, the title block and the view titles on
//! paper and a viewport at the set's scale on each view.
//!
//! A plan lies in model space where it lies in the model, turned back if the sheet turned it,
//! so a site plan or a survey drawn in the same coordinates lies under it; its viewport turns
//! it as the sheet shows it. A section or an elevation keeps its heights: its y is the
//! model's z. Views that would cover one another are moved apart along x. A sheet without
//! views of the set (shop drawings, cutting plans, the schedule) is drawn on paper alone.

use ketchup_dwg::sheet::{
    LinePattern, Pen, Sheet, SheetEntity, SheetLayer, SheetViewport, TextAlign, lineweight,
};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;

use crate::project_drawings::{MarkRole, PlacedView, SetSheet};

/// The layers of a sheet: name, AutoCAD colour index, lineweight in hundredths of a
/// millimetre and line pattern.
pub const LAYERS: [(&str, u8, i16, LinePattern); 12] = [
    (UNDERLAY, 8, 13, LinePattern::Continuous),
    (VISIBLE, 7, 18, LinePattern::Continuous),
    (CUT, 7, 50, LinePattern::Continuous),
    (CUT_FILL, 8, 0, LinePattern::Continuous),
    (ON_CUT, 7, 18, LinePattern::Continuous),
    (ANNOTATION, 7, 18, LinePattern::Continuous),
    (MARK, 7, 25, LinePattern::Continuous),
    (LEVEL, 7, 18, LinePattern::Continuous),
    (DIMENSION, 7, 18, LinePattern::Continuous),
    (VIEW_TITLE, 7, 25, LinePattern::Continuous),
    (SHEET, 7, 35, LinePattern::Continuous),
    (SHEET_TEXT, 7, 18, LinePattern::Continuous),
];
pub const UNDERLAY: &str = "UNDERLAY";
pub const VISIBLE: &str = "VISIBLE";
pub const CUT: &str = "CUT";
/// The fill of the parts the cutting plane passes through, in each part's colour.
pub const CUT_FILL: &str = "CUT-FILL";
pub const ON_CUT: &str = "ON-CUT";
pub const ANNOTATION: &str = "ANNOTATION";
pub const MARK: &str = "MARK";
pub const LEVEL: &str = "LEVEL";
pub const DIMENSION: &str = "DIMENSION";
pub const VIEW_TITLE: &str = "VIEW-TITLE";
/// The frame and the title block's lines.
pub const SHEET: &str = "SHEET";
pub const SHEET_TEXT: &str = "SHEET-TEXT";

/// Room left around a view's marks inside its viewport, in paper millimetres (not a
/// tolerance: a margin).
const VIEWPORT_MARGIN_MM: f64 = 3.0;
/// The gap between views moved apart in model space, in model millimetres per unit of scale
/// (not a tolerance: a spacing; 20 paper millimetres).
const VIEW_GAP_PAPER_MM: f64 = 20.0;
/// How wide a letter is likely to be for its height, to keep a text inside its view's
/// viewport (not a tolerance: a typeface's proportion, wide rather than narrow).
const TEXT_WIDTH_PER_HEIGHT: f64 = 0.8;
/// The most characters one TEXT holds, as AutoCAD reads it; a longer text goes on over the
/// next lines.
const TEXT_CHARACTERS: usize = 250;

/// `sheet`, whose views are drawn at 1:`scale`, as a CAD drawing named `layout`.
#[must_use]
pub fn sheet_cad(sheet: &SetSheet, scale: u32, layout: &str) -> Sheet {
    let scale = f64::from(scale);
    let page = &sheet.lines;
    let height = page.size_mm[1];
    let paper = |point: [f64; 2]| [point[0], height - point[1]];
    let mut on_paper = vec![true; page.marks.len()];
    let mut model = Vec::new();
    let mut paper_entities = Vec::new();
    let mut viewports = Vec::new();
    let mut taken: Vec<([f64; 2], [f64; 2])> = Vec::new();
    for placed in &sheet.placed {
        let Some(placing) = Placing::of(placed, page, scale, &taken) else {
            continue;
        };
        for (role, range) in &placed.marks {
            if *role == MarkRole::Title {
                continue;
            }
            for index in range.clone() {
                on_paper[index] = false;
                let to_model = |point: [f64; 2]| placing.model(point);
                entities(
                    &page.marks[index],
                    layers_of(*role),
                    &to_model,
                    scale,
                    placing.turn_deg,
                    &mut model,
                );
            }
        }
        taken.push(placing.extent);
        let [low, high] = placing.paper;
        viewports.push(SheetViewport {
            paper_min: paper([low[0], high[1]]),
            paper_max: paper([high[0], low[1]]),
            view_center: placing.display_center,
            twist_deg: -placing.turn_deg,
            scale,
        });
    }
    let titles = sheet
        .placed
        .iter()
        .flat_map(|placed| &placed.marks)
        .filter(|(role, _)| *role == MarkRole::Title)
        .flat_map(|(_, range)| range.clone())
        .collect::<Vec<_>>();
    for (index, mark) in page.marks.iter().enumerate() {
        if !on_paper[index] {
            continue;
        }
        let layers = if titles.contains(&index) {
            (VIEW_TITLE, VIEW_TITLE, VIEW_TITLE)
        } else {
            (SHEET, SHEET, SHEET_TEXT)
        };
        entities(mark, layers, &paper, 1.0, 0.0, &mut paper_entities);
    }
    Sheet {
        layout: layout.to_owned(),
        paper_mm: page.size_mm,
        layers: LAYERS
            .iter()
            .map(|(name, color, lineweight, pattern)| SheetLayer {
                name: (*name).to_owned(),
                color: *color,
                lineweight: *lineweight,
                pattern: *pattern,
            })
            .collect(),
        model,
        paper: paper_entities,
        viewports,
    }
}

/// Where a placed view goes in model space.
struct Placing {
    /// The view's runs of marks on the page, without its title: lower-left and upper-right
    /// corners in page millimetres (y down), with the viewport's margin.
    paper: [[f64; 2]; 2],
    /// The view in model space: lower-left and upper-right corners.
    extent: ([f64; 2], [f64; 2]),
    /// The model point of a view point is `offset` + the view point turned by `turn_deg`.
    turn_deg: f64,
    offset: [f64; 2],
    /// The middle of the viewport in display coordinates: the view point there, moved by the
    /// offset turned back.
    display_center: [f64; 2],
    placed_scale: f64,
    origin: [f64; 2],
    min: [f64; 2],
}

impl Placing {
    fn of(
        placed: &PlacedView,
        page: &Page,
        scale: f64,
        taken: &[([f64; 2], [f64; 2])],
    ) -> Option<Self> {
        let mut low = [f64::INFINITY; 2];
        let mut high = [f64::NEG_INFINITY; 2];
        for (role, range) in &placed.marks {
            if *role == MarkRole::Title {
                continue;
            }
            for point in range
                .clone()
                .flat_map(|index| mark_points(&page.marks[index]))
            {
                for axis in 0..2 {
                    low[axis] = low[axis].min(point[axis]);
                    high[axis] = high[axis].max(point[axis]);
                }
            }
        }
        if !low[0].is_finite() {
            return None;
        }
        let low = low.map(|value| value - VIEWPORT_MARGIN_MM);
        let high = high.map(|value| value + VIEWPORT_MARGIN_MM);
        // A plan's right is the world direction at `turn_deg`, its up a quarter turn on.
        let turn_deg = placed
            .plan_axes
            .map_or(0.0, |[right, _]| right[1].atan2(right[0]).to_degrees());
        let mut placing = Self {
            paper: [low, high],
            extent: ([0.0; 2], [0.0; 2]),
            turn_deg,
            offset: [0.0; 2],
            display_center: [0.0; 2],
            placed_scale: scale,
            origin: placed.origin,
            min: placed.min,
        };
        let corners = [low, [high[0], low[1]], high, [low[0], high[1]]].map(|p| placing.model(p));
        let extent_of = |offset: [f64; 2]| {
            let mut min = [f64::INFINITY; 2];
            let mut max = [f64::NEG_INFINITY; 2];
            for corner in corners {
                for axis in 0..2 {
                    min[axis] = min[axis].min(corner[axis] + offset[axis]);
                    max[axis] = max[axis].max(corner[axis] + offset[axis]);
                }
            }
            (min, max)
        };
        let mut extent = extent_of([0.0; 2]);
        let overlaps = |(min, max): ([f64; 2], [f64; 2])| {
            taken.iter().any(|(other_min, other_max)| {
                (0..2).all(|axis| min[axis] < other_max[axis] && other_min[axis] < max[axis])
            })
        };
        if overlaps(extent) {
            let right_of_all = taken
                .iter()
                .map(|(_, max)| max[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let shift = right_of_all + VIEW_GAP_PAPER_MM * scale - extent.0[0];
            placing.offset = [shift, 0.0];
            extent = extent_of(placing.offset);
        }
        placing.extent = extent;
        let centre = placing.model([
            f64::midpoint(low[0], high[0]),
            f64::midpoint(low[1], high[1]),
        ]);
        placing.display_center = turned(centre, -turn_deg);
        Some(placing)
    }

    /// The model point of the page point `point`.
    fn model(&self, point: [f64; 2]) -> [f64; 2] {
        let view = [
            self.min[0] + (point[0] - self.origin[0]) * self.placed_scale,
            self.min[1] + (self.origin[1] - point[1]) * self.placed_scale,
        ];
        let [x, y] = turned(view, self.turn_deg);
        [x + self.offset[0], y + self.offset[1]]
    }
}

/// `point` turned counter-clockwise about the origin by `degrees`.
fn turned([x, y]: [f64; 2], degrees: f64) -> [f64; 2] {
    if degrees == 0.0 {
        return [x, y];
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    [x * cos - y * sin, x * sin + y * cos]
}

/// The layers of a role's lines, fill and texts.
fn layers_of(role: MarkRole) -> (&'static str, &'static str, &'static str) {
    match role {
        MarkRole::Underlay => (UNDERLAY, UNDERLAY, UNDERLAY),
        MarkRole::Visible => (VISIBLE, VISIBLE, VISIBLE),
        MarkRole::Cut => (CUT, CUT_FILL, CUT),
        MarkRole::OnCut => (ON_CUT, ON_CUT, ON_CUT),
        MarkRole::Annotation => (ANNOTATION, ANNOTATION, ANNOTATION),
        MarkRole::Mark => (MARK, MARK, MARK),
        MarkRole::Level => (LEVEL, LEVEL, LEVEL),
        MarkRole::Dimension => (DIMENSION, DIMENSION, DIMENSION),
        MarkRole::Title => (VIEW_TITLE, VIEW_TITLE, VIEW_TITLE),
    }
}

/// The points `mark` reaches on the page: a text's as wide as its letters are likely to be.
fn mark_points(mark: &Mark) -> Vec<[f64; 2]> {
    match mark {
        Mark::Fill { loops, .. } => loops.iter().flatten().copied().collect(),
        Mark::Line { points, .. } => points.clone(),
        Mark::Text {
            at,
            text,
            height_mm,
            anchor,
            angle_deg,
            ..
        } => {
            let letters = text.chars().map(|_| 1.0).sum::<f64>();
            let width = TEXT_WIDTH_PER_HEIGHT * height_mm * letters;
            let (before, after) = match anchor {
                Anchor::Start => (0.0, width),
                Anchor::Middle => (width / 2.0, width / 2.0),
                Anchor::End => (width, 0.0),
            };
            // Along the text and up it, on a page that runs y down.
            let (sin, cos) = angle_deg.to_radians().sin_cos();
            let along = |distance: f64| [at[0] + cos * distance, at[1] - sin * distance];
            let [start, end] = [along(-before), along(after)];
            let up = [-sin * height_mm, -cos * height_mm];
            vec![
                start,
                end,
                [start[0] + up[0], start[1] + up[1]],
                [end[0] + up[0], end[1] + up[1]],
            ]
        }
    }
}

/// What a stroke draws with beyond its layer: a colour other than black, its width as the
/// nearest lineweight, and dashes.
fn pen(stroke: &Stroke) -> Pen {
    let black = stroke.color.iter().all(|channel| *channel < 48);
    Pen {
        color: (!black).then_some(stroke.color),
        lineweight: Some(lineweight(stroke.width_mm)),
        pattern: (!stroke.dash_mm.is_empty()).then_some(LinePattern::Dashed),
    }
}

/// The entities of `mark` on the layers `(lines, fill, texts)`: points mapped by `to`, text
/// heights times `scale` and turned on by `turn_deg`.
fn entities(
    mark: &Mark,
    (lines, fill, texts): (&str, &str, &str),
    to: &dyn Fn([f64; 2]) -> [f64; 2],
    scale: f64,
    turn_deg: f64,
    out: &mut Vec<SheetEntity>,
) {
    match mark {
        Mark::Line {
            points,
            closed,
            stroke,
        } => out.push(SheetEntity::Polyline {
            layer: lines.to_owned(),
            points: points.iter().map(|point| to(*point)).collect(),
            closed: *closed,
            pen: pen(stroke),
        }),
        Mark::Fill {
            loops,
            color,
            outline,
        } => {
            let loops = loops
                .iter()
                .map(|points| points.iter().map(|point| to(*point)).collect::<Vec<_>>())
                .filter(|points| points.len() > 2)
                .collect::<Vec<_>>();
            if loops.is_empty() {
                return;
            }
            // The fill first, so its outline is drawn over it.
            out.push(SheetEntity::Fill {
                layer: fill.to_owned(),
                loops: loops.clone(),
                color: Some(*color),
            });
            if let Some(stroke) = outline {
                for points in loops {
                    out.push(SheetEntity::Polyline {
                        layer: lines.to_owned(),
                        points,
                        closed: true,
                        pen: pen(stroke),
                    });
                }
            }
        }
        Mark::Text {
            at,
            text,
            height_mm,
            anchor,
            angle_deg,
            ..
        } => {
            let height = height_mm * scale;
            // Down the text, on a page that runs y down.
            let (sin, cos) = angle_deg.to_radians().sin_cos();
            let step = LINE_PITCH_PER_HEIGHT * height_mm;
            let mut down = 0.0;
            for value in text_lines(text) {
                out.push(SheetEntity::Text {
                    layer: texts.to_owned(),
                    at: to([at[0] + sin * down, at[1] + cos * down]),
                    value,
                    height,
                    angle_deg: angle_deg + turn_deg,
                    align: match anchor {
                        Anchor::Start => TextAlign::Start,
                        Anchor::Middle => TextAlign::Middle,
                        Anchor::End => TextAlign::End,
                    },
                });
                down += step;
            }
        }
    }
}

/// `text` in lines of at most [`TEXT_CHARACTERS`]: after the last space that fits, or
/// between characters when none does; a text of only spaces is none.
fn text_lines(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut rest = text.chars().collect::<Vec<_>>();
    while rest.len() > TEXT_CHARACTERS {
        let cut = rest[..=TEXT_CHARACTERS]
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|at| *at > 0)
            .unwrap_or(TEXT_CHARACTERS);
        lines.push(rest[..cut].iter().collect());
        rest.drain(..cut);
        while rest.first() == Some(&' ') {
            rest.remove(0);
        }
    }
    lines.push(rest.into_iter().collect());
    lines
}

#[cfg(test)]
mod tests;
