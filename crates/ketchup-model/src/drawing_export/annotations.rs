//! The plan's annotations on a drawing sheet: on a sheet that draws its parts
//! where they lie, every top view draws the shown annotations of the document
//! where they lie, at the scale of that view and inside its cell.
//! Text heights and the marks of a dimension (ticks, extension lines, the
//! arrowhead of a leader) are millimetres on paper. The page runs y up. In the
//! DXF each annotation lies on the layer of its tag, and a dimension is a
//! DIMENSION entity whose block draws it and which carries the measured value.

use super::layers::R12Layers;
use super::{
    LINE_SPACING, LineStyle, PageFill, PageLine, PageText, bounds_contains, page_point, view_scale,
};
use crate::annotation::{Annotation, AnnotationMark, DimensionLayout, dimension_text, paper};
use crate::document::Snapshot;
use crate::drawing::{DrawingSheet, DrawingSource, OrthographicDrawing, OrthographicViewKind};
use crate::tolerance::MIN_DIMENSION_MM;

/// The layer of an annotation without a tag.
const UNTAGGED_LAYER: &str = "Annotations";
/// The layers the sheet draws itself, never given to a tag; dimension blocks
/// draw on layer 0.
pub(super) const SHEET_LAYERS: [&str; 6] = [
    "0",
    "BORDER",
    "VISIBLE",
    "HIDDEN",
    "DIMENSION",
    "ANNOTATION",
];

/// The annotations a sheet drew, by kind, and those it left out with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SheetAnnotations {
    pub texts: usize,
    pub dimensions: usize,
    pub leaders: usize,
    pub lines: usize,
    /// Shown annotations of a sheet without a top view, or of one part drawn
    /// in its own frame: it has no plan to draw them in.
    pub without_plan_view: usize,
    /// Dimensions measured straight up or down: they have no length in plan.
    pub outside_plan: usize,
    /// Annotations that would reach past the top view's cell of the sheet,
    /// into another view or off the sheet; left out whole.
    pub off_sheet: Vec<String>,
}

/// What one annotation draws on the page, on the layer of its tag.
pub(super) struct SheetMark {
    pub(super) layer: String,
    pub(super) lines: Vec<PageLine>,
    /// A leader's arrowhead.
    pub(super) fills: Vec<PageFill>,
    pub(super) texts: Vec<PageText>,
    /// For a dimension: what its DIMENSION entity records.
    pub(super) dimension: Option<SheetDimension>,
}

/// A dimension on the page: the measured points, its line, where its value
/// sits, and the value Kečup measured in the model.
pub(super) struct SheetDimension {
    pub(super) measured: [[f64; 2]; 2],
    pub(super) line: [[f64; 2]; 2],
    pub(super) value_at: [f64; 2],
    pub(super) value: String,
}

/// The shown annotations drawn into every top view of `drawing`, when its sheet
/// draws its parts where they lie in the plan.
pub(super) fn draw_sheet_annotations(
    snapshot: &Snapshot,
    sheet: &DrawingSheet,
    drawing: &OrthographicDrawing,
) -> (SheetAnnotations, Vec<SheetMark>) {
    let shown: Vec<&Annotation> = snapshot
        .annotations()
        .iter()
        .filter(|annotation| snapshot.tags_visible(&annotation.tags))
        .collect();
    let mut counts = SheetAnnotations::default();
    let mut drawn = Vec::new();
    let plans: Vec<_> = drawing
        .views
        .iter()
        .zip(&drawing.layout.view_placements)
        .filter(|(view, _)| view.kind == OrthographicViewKind::Top)
        .collect();
    // A sheet of one part draws it in the part's own frame, not the plan's.
    if plans.is_empty() || matches!(sheet.source(), DrawingSource::Definition(_)) {
        counts.without_plan_view = shown.len();
        return (counts, drawn);
    }
    let mut layers = R12Layers::reserving(&SHEET_LAYERS);
    for (view, placement) in plans {
        let scale = view_scale(view, placement);
        let at = |point: [f64; 3]| page_point([point[0], point[1]], placement.origin_mm, scale);
        for annotation in &shown {
            let source = annotation
                .tags
                .iter()
                .filter_map(|id| snapshot.tag(*id))
                .map(|tag| tag.name())
                .min()
                .unwrap_or(UNTAGGED_LAYER);
            let mut marks = Marks {
                name: &annotation.name,
                lines: Vec::new(),
                fills: Vec::new(),
                texts: Vec::new(),
                dimension: None,
            };
            if !marks.draw(annotation, &at) {
                counts.outside_plan += 1;
                continue;
            }
            if !marks.inside(placement.cell_bounds_mm) {
                if !counts.off_sheet.contains(&annotation.name) {
                    counts.off_sheet.push(annotation.name.clone());
                }
                continue;
            }
            match annotation.mark {
                AnnotationMark::Text { .. } => counts.texts += 1,
                AnnotationMark::Dimension { .. } => counts.dimensions += 1,
                AnnotationMark::Leader { .. } => counts.leaders += 1,
                AnnotationMark::Line { .. } => counts.lines += 1,
            }
            drawn.push(SheetMark {
                layer: layers.layer(source),
                lines: marks.lines,
                fills: marks.fills,
                texts: marks.texts,
                dimension: marks.dimension,
            });
        }
    }
    (counts, drawn)
}

/// The lines and texts one annotation draws on the page.
struct Marks<'a> {
    name: &'a str,
    lines: Vec<PageLine>,
    fills: Vec<PageFill>,
    texts: Vec<PageText>,
    dimension: Option<SheetDimension>,
}

impl Marks<'_> {
    /// `false` for a dimension that has no length in plan.
    fn draw(&mut self, annotation: &Annotation, at: &impl Fn([f64; 3]) -> [f64; 2]) -> bool {
        match &annotation.mark {
            AnnotationMark::Text {
                at_mm,
                text,
                height_mm,
            } => self.text(at(*at_mm), text, *height_mm),
            AnnotationMark::Dimension {
                from_mm,
                to_mm,
                offset_mm,
                height_mm,
            } => {
                let layout = DimensionLayout::new(*from_mm, *to_mm, *offset_mm);
                let [start, end] = layout.line;
                if (end[0] - start[0]).hypot(end[1] - start[1]) < MIN_DIMENSION_MM {
                    return false;
                }
                self.dimension(
                    [at(*from_mm), at(*to_mm)],
                    [at(start), at(end)],
                    &dimension_text(layout.value_mm),
                    *height_mm,
                );
            }
            AnnotationMark::Leader {
                point_mm,
                at_mm,
                text,
                height_mm,
            } => {
                let text_at = at(*at_mm);
                self.leader(at(*point_mm), text_at);
                self.text(text_at, text, *height_mm);
            }
            AnnotationMark::Line { points_mm, closed } => {
                let points: Vec<_> = points_mm.iter().map(|point| at(*point)).collect();
                for pair in points.windows(2) {
                    self.line(pair[0], pair[1]);
                }
                if *closed && points.len() > 2 {
                    self.line(points[points.len() - 1], points[0]);
                }
            }
        }
        true
    }

    fn line(&mut self, start: [f64; 2], end: [f64; 2]) {
        self.lines.push(PageLine {
            id: format!("annotation-{}-{}", self.name, self.lines.len()),
            start,
            end,
            style: LineStyle::Dimension,
        });
    }

    /// A left-aligned text, the first line on the baseline at `at`.
    fn text(&mut self, at: [f64; 2], text: &str, height_mm: f64) {
        self.texts.push(PageText {
            id: format!("annotation-{}", self.name),
            position: at,
            lines: text.split('\n').map(str::to_owned).collect(),
            height_mm,
            angle_deg: 0.0,
            middle: false,
        });
    }

    /// An aligned dimension as [`paper::dimension_marks`] draws it, its value
    /// centred above the line.
    fn dimension(
        &mut self,
        measured: [[f64; 2]; 2],
        line: [[f64; 2]; 2],
        value: &str,
        height_mm: f64,
    ) {
        let Some(marks) = paper::dimension_marks(measured, line, 1.0) else {
            return;
        };
        for [start, end] in marks.lines {
            self.line(start, end);
        }
        self.dimension = Some(SheetDimension {
            measured,
            line,
            value_at: marks.value_at,
            value: value.to_owned(),
        });
        self.texts.push(PageText {
            id: format!("annotation-{}", self.name),
            position: marks.value_at,
            lines: vec![value.to_owned()],
            height_mm,
            angle_deg: marks.value_angle_degrees,
            middle: true,
        });
    }

    /// The line of a leader from `point` to `at`, with a filled arrowhead at
    /// `point`.
    fn leader(&mut self, point: [f64; 2], at: [f64; 2]) {
        self.line(point, at);
        if let Some(corners) = paper::leader_arrowhead(point, at, 1.0) {
            self.fills.push(PageFill {
                id: format!("annotation-{}-arrow", self.name),
                corners,
            });
        }
    }

    /// Whether every line, fill and text baseline lies inside `bounds`.
    fn inside(&self, bounds: [[f64; 2]; 2]) -> bool {
        self.lines
            .iter()
            .all(|line| bounds_contains(bounds, line.start) && bounds_contains(bounds, line.end))
            && self.fills.iter().all(|fill| {
                fill.corners
                    .iter()
                    .all(|corner| bounds_contains(bounds, *corner))
            })
            && self.texts.iter().all(|text| {
                let last = text
                    .lines
                    .iter()
                    .skip(1)
                    .fold(text.position[1], |y, _| y - LINE_SPACING * text.height_mm);
                bounds_contains(bounds, text.position)
                    && bounds_contains(bounds, [text.position[0], last])
            })
    }
}
