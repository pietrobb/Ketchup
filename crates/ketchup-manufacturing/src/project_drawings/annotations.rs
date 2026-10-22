//! The drawing annotations of the plan: texts, dimensions writing the value the
//! document measured, leaders and drawn lines. They are projected like the solids
//! and drawn at the sheet scale; text heights and the marks of a dimension (ticks,
//! extension lines, the arrowhead of a leader) are millimetres on paper.

use super::ViewFrame;
use crate::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::annotation::{Annotation, AnnotationMark, DimensionLayout, dimension_text};
use ketchup_model::tolerance::{MIN_DIMENSION_MM, ROUNDING};
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;
use serde::Serialize;

/// On paper: the gap between a measured point and its extension line, how far the
/// extension line runs past the dimension line, the length of an oblique tick, the
/// gap between a dimension line and its value, and a leader's arrowhead.
const EXTENSION_GAP_MM: f64 = 1.0;
const EXTENSION_BEYOND_MM: f64 = 2.0;
const TICK_MM: f64 = 3.0;
const VALUE_GAP_MM: f64 = 1.0;
const ARROW_LENGTH_MM: f64 = 3.0;
const ARROW_WIDTH_MM: f64 = 1.0;
// not a tolerance: the pen width of an annotation on the sheet.
const ANNOTATION_STROKE_MM: f64 = 0.25;
const ANNOTATION_COLOR: [u8; 3] = [0, 0, 0];

/// The annotations a plan draws, by kind.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AnnotationCounts {
    pub texts: usize,
    pub dimensions: usize,
    pub leaders: usize,
    pub lines: usize,
    /// Dimensions measured straight up or down: they have no length in plan and
    /// are left out.
    pub outside_plan: usize,
}

/// One annotation in view millimetres.
enum Projected {
    Text {
        at: [f64; 2],
        text: String,
        height_mm: f64,
    },
    Dimension {
        measured: [[f64; 2]; 2],
        line: [[f64; 2]; 2],
        value: String,
        height_mm: f64,
    },
    Leader {
        point: [f64; 2],
        at: [f64; 2],
        text: String,
        height_mm: f64,
    },
    Line {
        points: Vec<[f64; 2]>,
        closed: bool,
    },
}

/// The annotations of a plan, projected into its view.
pub(super) struct PlanAnnotations {
    marks: Vec<Projected>,
    pub(super) counts: AnnotationCounts,
}

impl PlanAnnotations {
    pub(super) fn project(frame: &ViewFrame, annotations: &[Annotation]) -> Self {
        let mut counts = AnnotationCounts::default();
        let mut marks = Vec::with_capacity(annotations.len());
        for annotation in annotations {
            // The view places every point, so only a dimension without length in plan
            // is left out.
            let Some(mark) = projected(annotation, |point| Some(frame.project(point))) else {
                counts.outside_plan += 1;
                continue;
            };
            match mark {
                Projected::Text { .. } => counts.texts += 1,
                Projected::Dimension { .. } => counts.dimensions += 1,
                Projected::Leader { .. } => counts.leaders += 1,
                Projected::Line { .. } => counts.lines += 1,
            }
            marks.push(mark);
        }
        Self { marks, counts }
    }

    /// Every point the annotations are drawn through, in view millimetres.
    pub(super) fn points(&self) -> impl Iterator<Item = [f64; 2]> + '_ {
        self.marks.iter().flat_map(|mark| match mark {
            Projected::Text { at, .. } => vec![*at],
            Projected::Dimension { measured, line, .. } => {
                vec![measured[0], measured[1], line[0], line[1]]
            }
            Projected::Leader { point, at, .. } => vec![*point, *at],
            Projected::Line { points, .. } => points.clone(),
        })
    }

    /// Draws them on `page`; `at` takes a view point to the page.
    pub(super) fn draw(&self, page: &mut Page, at: impl Fn([f64; 2]) -> [f64; 2]) {
        for mark in &self.marks {
            mark.draw(page, &at);
        }
    }
}

/// The marks one annotation draws where `project` places its points in page
/// millimetres (y down): the plan of a sheet, or a window's view of the model. Text
/// heights and the marks of a dimension are millimetres of that page. `None` when
/// `project` has no place for a point, or a dimension has no length there.
pub fn annotation_marks(
    annotation: &Annotation,
    project: impl Fn([f64; 3]) -> Option<[f64; 2]>,
) -> Option<Vec<Mark>> {
    let mut page = Page::default();
    projected(annotation, project)?.draw(&mut page, &|point| point);
    Some(page.marks)
}

fn projected(
    annotation: &Annotation,
    project: impl Fn([f64; 3]) -> Option<[f64; 2]>,
) -> Option<Projected> {
    Some(match &annotation.mark {
        AnnotationMark::Text {
            at_mm,
            text,
            height_mm,
        } => Projected::Text {
            at: project(*at_mm)?,
            text: text.clone(),
            height_mm: *height_mm,
        },
        AnnotationMark::Dimension {
            from_mm,
            to_mm,
            offset_mm,
            height_mm,
        } => {
            let layout = DimensionLayout::new(*from_mm, *to_mm, *offset_mm);
            let line = [project(layout.line[0])?, project(layout.line[1])?];
            if distance(line[0], line[1]) < MIN_DIMENSION_MM {
                return None;
            }
            Projected::Dimension {
                measured: [project(*from_mm)?, project(*to_mm)?],
                line,
                value: dimension_text(layout.value_mm),
                height_mm: *height_mm,
            }
        }
        AnnotationMark::Leader {
            point_mm,
            at_mm,
            text,
            height_mm,
        } => Projected::Leader {
            point: project(*point_mm)?,
            at: project(*at_mm)?,
            text: text.clone(),
            height_mm: *height_mm,
        },
        AnnotationMark::Line { points_mm, closed } => Projected::Line {
            points: points_mm
                .iter()
                .map(|point| project(*point))
                .collect::<Option<_>>()?,
            closed: *closed,
        },
    })
}

impl Projected {
    fn draw(&self, page: &mut Page, at: &impl Fn([f64; 2]) -> [f64; 2]) {
        match self {
            Self::Text {
                at: point,
                text,
                height_mm,
            } => lines_of_text(page, at(*point), text, *height_mm),
            Self::Dimension {
                measured,
                line,
                value,
                height_mm,
            } => dimension(page, measured.map(at), line.map(at), value, *height_mm),
            Self::Leader {
                point,
                at: text_at,
                text,
                height_mm,
            } => {
                let (point, text_at) = (at(*point), at(*text_at));
                leader(page, point, text_at);
                lines_of_text(page, text_at, text, *height_mm);
            }
            Self::Line { points, closed } => page.marks.push(Mark::Line {
                points: points.iter().map(|point| at(*point)).collect(),
                closed: *closed,
                stroke: stroke(),
            }),
        }
    }
}

fn stroke() -> Stroke {
    Stroke {
        width_mm: ANNOTATION_STROKE_MM,
        color: ANNOTATION_COLOR,
        dash_mm: Vec::new(),
    }
}

/// A left-aligned text, the first line on the baseline at `at` (page mm, y down).
fn lines_of_text(page: &mut Page, at: [f64; 2], text: &str, height_mm: f64) {
    let mut baseline = at[1];
    for line in text.split('\n') {
        page.text([at[0], baseline], line, height_mm, Anchor::Start);
        baseline += height_mm * LINE_PITCH_PER_HEIGHT;
    }
}

/// An aligned dimension on the page: its line with oblique ticks, the extension
/// lines from the measured points, and the value above the line, reading from
/// left to right or from bottom to top.
fn dimension(
    page: &mut Page,
    measured: [[f64; 2]; 2],
    line: [[f64; 2]; 2],
    value: &str,
    height_mm: f64,
) {
    let length = distance(line[0], line[1]);
    if length <= 0.0 {
        return;
    }
    let along = [
        (line[1][0] - line[0][0]) / length,
        (line[1][1] - line[0][1]) / length,
    ];
    page.line(line.to_vec(), stroke());
    let tick = TICK_MM / 2.0 / std::f64::consts::SQRT_2;
    for end in line {
        let slant = [(along[0] + along[1]) * tick, (along[1] - along[0]) * tick];
        page.line(
            vec![
                [end[0] - slant[0], end[1] - slant[1]],
                [end[0] + slant[0], end[1] + slant[1]],
            ],
            stroke(),
        );
    }
    for (point, end) in measured.into_iter().zip(line) {
        let reach = distance(point, end);
        if reach <= EXTENSION_GAP_MM {
            continue;
        }
        let toward = [(end[0] - point[0]) / reach, (end[1] - point[1]) / reach];
        page.line(
            vec![
                [
                    point[0] + toward[0] * EXTENSION_GAP_MM,
                    point[1] + toward[1] * EXTENSION_GAP_MM,
                ],
                [
                    end[0] + toward[0] * EXTENSION_BEYOND_MM,
                    end[1] + toward[1] * EXTENSION_BEYOND_MM,
                ],
            ],
            stroke(),
        );
    }
    // The page runs y down: a line pointing left, or straight down the sheet, is
    // read the other way round.
    let reading = if along[0] < -ROUNDING || (along[0].abs() <= ROUNDING && along[1] > 0.0) {
        [-along[0], -along[1]]
    } else {
        along
    };
    // Up from the reading direction, as the sheet shows it.
    let up = [reading[1], -reading[0]];
    let middle = [
        f64::midpoint(line[0][0], line[1][0]),
        f64::midpoint(line[0][1], line[1][1]),
    ];
    page.marks.push(Mark::Text {
        at: [
            middle[0] + up[0] * VALUE_GAP_MM,
            middle[1] + up[1] * VALUE_GAP_MM,
        ],
        text: value.to_owned(),
        height_mm,
        anchor: Anchor::Middle,
        angle_deg: (-reading[1]).atan2(reading[0]).to_degrees(),
        bold: false,
    });
}

/// The line of a leader from `point` to `at` with a filled arrowhead at `point`.
fn leader(page: &mut Page, point: [f64; 2], at: [f64; 2]) {
    page.line(vec![point, at], stroke());
    let length = distance(point, at);
    if length <= 0.0 {
        return;
    }
    let toward = [(at[0] - point[0]) / length, (at[1] - point[1]) / length];
    let arrow = ARROW_LENGTH_MM.min(length);
    let half = ARROW_WIDTH_MM / 2.0;
    let base = [point[0] + toward[0] * arrow, point[1] + toward[1] * arrow];
    let side = [-toward[1] * half, toward[0] * half];
    page.marks.push(Mark::Fill {
        loops: vec![vec![
            point,
            [base[0] + side[0], base[1] + side[1]],
            [base[0] - side[0], base[1] - side[1]],
        ]],
        color: ANNOTATION_COLOR,
        outline: None,
    });
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}
