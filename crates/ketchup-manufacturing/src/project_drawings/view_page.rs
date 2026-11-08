//! One view drawn on its sheet, each run of marks it adds named by what it shows, so a CAD
//! drawing of the sheet can put the view in model space at 1:1 with its marks on layers of
//! their own ([`PlacedView`]).

use std::ops::Range;

use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};

use super::{
    CUT_STROKE_MM, DrawnView, EDGE_COLOR, EDGE_STROKE_MM, Item, ProjectView, UNDERLAY_COLOR,
    UNDERLAY_STROKE_MM, VIEW_TITLE_MM, levels, openings, overall_dimensions, visible,
};

/// What a run of a view's marks shows.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MarkRole {
    /// The lines of an underlay drawing beneath a plan.
    Underlay,
    /// Edges and faces of the solids seen beyond the cut, and their threads.
    Visible,
    /// The parts the cutting plane passes through: fill and outline.
    Cut,
    /// Thread lines drawn over a cut.
    OnCut,
    /// The program's texts, dimensions, leaders and lines.
    Annotation,
    /// The boxed marks of windows and doors, or the position numbers of a shop drawing.
    Mark,
    /// Level marks of a section or an elevation.
    Level,
    /// The overall dimensions of the view.
    Dimension,
    /// The view's title, which belongs on paper.
    Title,
}

/// A view as it lies on its sheet: `marks` of the sheet's page are its own, drawn at
/// 1:`scale` with its model corner `min` (view millimetres) at `origin` (page millimetres,
/// y down).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedView {
    pub view: ProjectView,
    pub origin: [f64; 2],
    pub min: [f64; 2],
    /// The horizontal world directions of a plan's right and up; `None` for a view whose
    /// up is the world's z.
    pub plan_axes: Option<[[f64; 2]; 2]>,
    pub marks: Vec<(MarkRole, Range<usize>)>,
}

impl PlacedView {
    /// The view millimetres of the page point `point`.
    #[must_use]
    pub fn view_point(&self, point: [f64; 2], scale: f64) -> [f64; 2] {
        [
            self.min[0] + (point[0] - self.origin[0]) * scale,
            self.min[1] + (self.origin[1] - point[1]) * scale,
        ]
    }
}

/// Records the marks `page` gains, run by run.
struct Runs<'a> {
    page: &'a mut Page,
    runs: Vec<(MarkRole, Range<usize>)>,
}

impl Runs<'_> {
    fn draw(&mut self, role: MarkRole, draw: impl FnOnce(&mut Page)) {
        let start = self.page.marks.len();
        draw(self.page);
        let end = self.page.marks.len();
        if end > start {
            self.runs.push((role, start..end));
        }
    }
}

/// Draws one view with its lower-left model corner at `origin` (page mm):
/// painted far to near, or with `visible` its visible lines alone (no face
/// fill), as a DXF carries it. Returns where the view lies and its runs of marks.
pub(super) fn draw_view_on_page(
    page: &mut Page,
    view: &DrawnView,
    title: &str,
    origin: [f64; 2],
    scale: f64,
    visible: Option<&[visible::VisibleLine]>,
) -> PlacedView {
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
    let mut runs = Runs {
        page,
        runs: Vec::new(),
    };
    // First, so the model is drawn over it.
    runs.draw(MarkRole::Underlay, |page| {
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
    });
    runs.draw(MarkRole::Visible, |page| {
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
    });
    runs.draw(MarkRole::Cut, |page| {
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
    });
    runs.draw(MarkRole::OnCut, |page| {
        for item in view.on_cut.iter().filter(|_| visible.is_none()) {
            if let Item::Thread { points, width_mm } = item {
                thread_line(page, points, *width_mm);
            }
        }
    });
    runs.draw(MarkRole::Annotation, |page| {
        if let Some(annotations) = &view.annotations {
            annotations.draw(page, at);
        }
    });
    runs.draw(MarkRole::Mark, |page| {
        for (point, mark) in &view.marks {
            openings::draw_mark(page, at(*point), mark, visible.is_none());
        }
    });
    runs.draw(MarkRole::Level, |page| {
        let levels = view
            .levels
            .iter()
            .map(|(height, text)| (at([view.min[0], *height])[1], text.clone()))
            .collect::<Vec<_>>();
        levels::draw_levels(page, at(view.max)[0], &levels);
    });
    let [outer_left, outer_bottom] = at(view.min);
    // Overall width below the view and overall height on its left, outside
    // whatever the view draws.
    let below = outer_bottom + 8.0;
    runs.draw(MarkRole::Dimension, |page| {
        if let Some((low, high)) = view.solids {
            overall_dimensions(
                page,
                [at(low), at(high)],
                [high[0] - low[0], high[1] - low[1]],
                [below, outer_left - 8.0],
            );
        }
    });
    runs.draw(MarkRole::Title, |page| {
        page.marks.push(Mark::Text {
            at: [outer_left, below + 8.0],
            text: title.to_owned(),
            height_mm: VIEW_TITLE_MM,
            anchor: Anchor::Start,
            angle_deg: 0.0,
            bold: true,
        });
    });
    let horizontal = |axis: [f64; 3]| [axis[0], axis[1]];
    PlacedView {
        view: view.view,
        origin,
        min: view.min,
        plan_axes: (view.view == ProjectView::Plan)
            .then(|| [horizontal(view.frame.right), horizontal(view.frame.up)]),
        marks: runs.runs,
    }
}
