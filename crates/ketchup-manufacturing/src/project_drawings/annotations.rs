//! The drawing annotations of the plan: texts, dimensions writing the value the
//! document measured, leaders and drawn lines. They are projected like the solids
//! and drawn at the sheet scale; text heights and the marks of a dimension (ticks,
//! extension lines, the arrowhead of a leader) are millimetres on paper.

use super::ViewFrame;
use ketchup_geometry::linalg::dot;
use ketchup_model::annotation::{
    Annotation, AnnotationMark, DimensionLayout, dimension_text, paper,
};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::tolerance::{MIN_DIMENSION_MM, ROUNDING};
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;
use serde::Serialize;

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

impl AnnotationCounts {
    /// These and `other` together: the annotations of a sheet with several views.
    pub(super) fn add(&mut self, other: &Self) {
        self.texts += other.texts;
        self.dimensions += other.dimensions;
        self.leaders += other.leaders;
        self.lines += other.lines;
        self.outside_plan += other.outside_plan;
    }
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

    /// The dimensions a section or an elevation draws (Měchenice MECH-6): those
    /// with a height that lie parallel to the view, which shows their true length,
    /// and in a section only those at or beyond the cut. A plan leaves a vertical
    /// one out. Its offset, along −x as [`DimensionLayout`] sets it, has no length
    /// in a view along x: the line is set off to the left in the view there.
    pub(super) fn project_upright(frame: &ViewFrame, annotations: &[Annotation]) -> Self {
        let mut marks = Vec::new();
        for annotation in annotations {
            let AnnotationMark::Dimension {
                from_mm,
                to_mm,
                offset_mm,
                height_mm,
            } = &annotation.mark
            else {
                continue;
            };
            let layout = DimensionLayout::new(*from_mm, *to_mm, *offset_mm);
            if layout.along[2].abs() <= ROUNDING
                || dot(layout.along, frame.depth).abs() > ROUNDING
                || frame
                    .cut
                    .is_some_and(|cut| dot(*from_mm, frame.depth) < cut - ROUNDING)
            {
                continue;
            }
            let measured = [frame.project(*from_mm), frame.project(*to_mm)];
            let across = frame.project(layout.across);
            let across = if across[0].hypot(across[1]) > ROUNDING {
                across
            } else {
                [-1.0, 0.0]
            };
            let line = measured.map(|point| {
                [
                    point[0] + across[0] * offset_mm,
                    point[1] + across[1] * offset_mm,
                ]
            });
            marks.push(Projected::Dimension {
                measured,
                line,
                value: dimension_text(layout.value_mm),
                height_mm: *height_mm,
            });
        }
        let counts = AnnotationCounts {
            dimensions: marks.len(),
            ..AnnotationCounts::default()
        };
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

/// The page runs y down and [`paper`] y up: a point of the one is the other's
/// with y negated, and a turn keeps its sense as the sheet shows it.
const fn flipped([x, y]: [f64; 2]) -> [f64; 2] {
    [x, -y]
}

/// An aligned dimension on the page as [`paper::dimension_marks`] draws it.
fn dimension(
    page: &mut Page,
    measured: [[f64; 2]; 2],
    line: [[f64; 2]; 2],
    value: &str,
    height_mm: f64,
) {
    let Some(marks) = paper::dimension_marks(measured.map(flipped), line.map(flipped), 1.0) else {
        return;
    };
    for drawn in marks.lines {
        page.line(drawn.map(flipped).to_vec(), stroke());
    }
    page.marks.push(Mark::Text {
        at: flipped(marks.value_at),
        text: value.to_owned(),
        height_mm,
        anchor: Anchor::Middle,
        angle_deg: marks.value_angle_degrees,
        bold: false,
    });
}

/// The line of a leader from `point` to `at` with a filled arrowhead at `point`.
fn leader(page: &mut Page, point: [f64; 2], at: [f64; 2]) {
    page.line(vec![point, at], stroke());
    if let Some(corners) = paper::leader_arrowhead(point, at, 1.0) {
        page.marks.push(Mark::Fill {
            loops: vec![corners.to_vec()],
            color: ANNOTATION_COLOR,
            outline: None,
        });
    }
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view along x has no length for the offset along −x: the line of a
    /// vertical dimension still stands off the edge it measures.
    #[test]
    fn a_side_view_sets_a_vertical_dimension_off_to_the_left() {
        let side = ViewFrame {
            right: [0.0, -1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            depth: [1.0, 0.0, 0.0],
            cut: None,
        };
        let vertical = Annotation {
            name: "height".to_owned(),
            tags: std::collections::BTreeSet::new(),
            mark: AnnotationMark::Dimension {
                from_mm: [0.0, 0.0, 0.0],
                to_mm: [0.0, 0.0, 2000.0],
                offset_mm: 400.0,
                height_mm: 2.5,
            },
        };
        let drawn = PlanAnnotations::project_upright(&side, &[vertical]);
        let Some(Projected::Dimension { measured, line, .. }) = drawn.marks.first() else {
            panic!("the vertical dimension is drawn");
        };
        assert_eq!(*measured, [[0.0, 0.0], [0.0, 2000.0]]);
        assert_eq!(*line, [[-400.0, 0.0], [-400.0, 2000.0]]);
    }
}
