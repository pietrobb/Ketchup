//! The drawing annotations of a plan DXF: a text as TEXT, a dimension as an aligned
//! DIMENSION with the block that draws it, a leader as a line, an arrowhead and a
//! TEXT, and a drawn line as a POLYLINE. Each annotation lies on the layer of its tag.
//! Text heights are millimetres on paper; at a scale of 1:N the file, which is in
//! model millimetres, writes them N times taller.

use crate::dxf_export::{R12Layers, number};
use ketchup_model::annotation::{AnnotationMark, DimensionLayout, dimension_text, paper};
use ketchup_model::document::Snapshot;
use ketchup_model::r12_text;
use ketchup_model::tolerance::MIN_DIMENSION_MM;
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The scale of a plan DXF whose export names none: 1:100.
pub const DEFAULT_PLAN_SCALE: f64 = 100.0;
/// The largest scale denominator a plan takes, 1:100 000.
pub const MAX_PLAN_SCALE: f64 = 100_000.0;
/// The layer of an annotation without a tag.
const UNTAGGED_LAYER: &str = "Annotations";

/// What the annotations add to a plan DXF.
#[derive(Default)]
pub(crate) struct PlanAnnotations {
    /// Records of the ENTITIES section.
    pub(crate) entities: String,
    /// BLOCK records drawing the dimensions, for the BLOCKS section.
    pub(crate) blocks: String,
    pub(crate) layers: BTreeSet<String>,
    pub(crate) texts: usize,
    pub(crate) dimensions: usize,
    pub(crate) leaders: usize,
    pub(crate) lines: usize,
    /// Every TEXT written, those inside dimension blocks included.
    pub(crate) drawn_texts: usize,
    pub(crate) hidden: usize,
    /// Dimensions between points at different heights: the plan shortens them,
    /// so they carry the measured value as their text.
    pub(crate) value_overrides: usize,
    /// Dimensions measured straight up or down, which have no length in plan.
    pub(crate) outside_plan: usize,
}

impl PlanAnnotations {
    pub(crate) fn is_empty(&self) -> bool {
        self.texts + self.dimensions + self.leaders + self.lines == 0
    }
}

/// The visible annotations of `snapshot` drawn at `1:scale`, on layers taken from
/// `layers` so that none shares a layer with a part.
pub(crate) fn plan_annotations(
    snapshot: &Snapshot,
    scale: f64,
    layers: &mut R12Layers,
) -> PlanAnnotations {
    let mut plan = PlanAnnotations::default();
    for annotation in snapshot.annotations() {
        if !snapshot.tags_visible(&annotation.tags) {
            plan.hidden += 1;
            continue;
        }
        let source = annotation
            .tags
            .iter()
            .filter_map(|id| snapshot.tag(*id))
            .map(|tag| tag.name())
            .min()
            .unwrap_or(UNTAGGED_LAYER);
        let layer = layers.layer(source);
        match &annotation.mark {
            AnnotationMark::Text {
                at_mm,
                text,
                height_mm,
            } => {
                plan.text(&layer, plan_point(*at_mm), height_mm * scale, text);
                plan.texts += 1;
            }
            AnnotationMark::Dimension {
                from_mm,
                to_mm,
                offset_mm,
                height_mm,
            } => {
                let layout = DimensionLayout::new(*from_mm, *to_mm, *offset_mm);
                if !plan.dimension(
                    &layer,
                    [*from_mm, *to_mm],
                    &layout,
                    height_mm * scale,
                    scale,
                ) {
                    plan.outside_plan += 1;
                    continue;
                }
                plan.dimensions += 1;
            }
            AnnotationMark::Leader {
                point_mm,
                at_mm,
                text,
                height_mm,
            } => {
                plan.leader(&layer, plan_point(*point_mm), plan_point(*at_mm), scale);
                plan.text(&layer, plan_point(*at_mm), height_mm * scale, text);
                plan.leaders += 1;
            }
            AnnotationMark::Line { points_mm, closed } => {
                let points = points_mm
                    .iter()
                    .copied()
                    .map(plan_point)
                    .collect::<Vec<_>>();
                polyline(&mut plan.entities, &layer, &points, *closed);
                plan.lines += 1;
            }
        }
        plan.layers.insert(layer);
    }
    plan
}

impl PlanAnnotations {
    /// A left-aligned text, one TEXT per line, the first on the baseline at `at`;
    /// a line longer than an R12 string holds goes on over the next ones.
    fn text(&mut self, layer: &str, at: [f64; 2], height: f64, text: &str) {
        let mut baseline = at[1];
        for line in text
            .split('\n')
            .flat_map(|line| r12_text::lines(line, f64::INFINITY))
        {
            let place = TextPlace {
                at: [at[0], baseline],
                height,
                angle_degrees: 0.0,
                centred: false,
            };
            text_record(&mut self.entities, layer, &place, &line);
            self.drawn_texts += 1;
            baseline -= height * LINE_PITCH_PER_HEIGHT;
        }
    }

    /// An aligned dimension and its block, drawn as [`paper::dimension_marks`]
    /// draws it at `1:scale`; `false` when it has no length in plan.
    fn dimension(
        &mut self,
        layer: &str,
        measured: [[f64; 3]; 2],
        layout: &DimensionLayout,
        height: f64,
        scale: f64,
    ) -> bool {
        let points = measured.map(plan_point);
        let line = layout.line.map(plan_point);
        if distance(line[0], line[1]) < MIN_DIMENSION_MM {
            return false;
        }
        let Some(marks) = paper::dimension_marks(points, line, scale) else {
            return false;
        };
        let text_at = marks.value_at;
        let value = dimension_text(layout.value_mm);
        let shortened = dimension_text(distance(points[0], points[1])) != value;

        let block = format!("*D{}", self.dimensions + 1);
        write!(
            self.blocks,
            "0\nBLOCK\n8\n0\n2\n{block}\n70\n1\n10\n0\n20\n0\n30\n0\n3\n{block}\n"
        )
        .expect("writing to a String cannot fail");
        for [start, end] in marks.lines {
            line_record(&mut self.blocks, "0", start, end);
        }
        let place = TextPlace {
            at: text_at,
            height,
            angle_degrees: marks.value_angle_degrees,
            centred: true,
        };
        text_record(&mut self.blocks, "0", &place, &value);
        self.drawn_texts += 1;
        self.blocks.push_str("0\nENDBLK\n8\n0\n");

        write!(
            self.entities,
            "0\nDIMENSION\n8\n{layer}\n2\n{block}\n3\nSTANDARD\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n70\n1\n",
            number(line[1][0]),
            number(line[1][1]),
            number(text_at[0]),
            number(text_at[1]),
        )
        .expect("writing to a String cannot fail");
        if shortened {
            write!(self.entities, "1\n{}\n", r12_text::lossy(&value))
                .expect("writing to a String cannot fail");
            self.value_overrides += 1;
        }
        write!(
            self.entities,
            "13\n{}\n23\n{}\n33\n0\n14\n{}\n24\n{}\n34\n0\n",
            number(points[0][0]),
            number(points[0][1]),
            number(points[1][0]),
            number(points[1][1]),
        )
        .expect("writing to a String cannot fail");
        true
    }

    /// The line of a leader from `point` to `at`, with a filled arrowhead at `point`.
    fn leader(&mut self, layer: &str, point: [f64; 2], at: [f64; 2], scale: f64) {
        line_record(&mut self.entities, layer, point, at);
        let Some([tip, left, right]) = paper::leader_arrowhead(point, at, scale) else {
            return;
        };
        // R12 takes a triangle as a SOLID whose last two corners meet.
        let corners = [tip, left, right, right];
        write!(self.entities, "0\nSOLID\n8\n{layer}\n").expect("writing to a String cannot fail");
        for (index, corner) in corners.iter().enumerate() {
            write!(
                self.entities,
                "1{index}\n{}\n2{index}\n{}\n3{index}\n0\n",
                number(corner[0]),
                number(corner[1]),
            )
            .expect("writing to a String cannot fail");
        }
    }
}

/// Where and how a TEXT is written.
struct TextPlace {
    at: [f64; 2],
    height: f64,
    angle_degrees: f64,
    /// Centred on `at` along its baseline rather than starting there.
    centred: bool,
}

fn text_record(output: &mut String, layer: &str, place: &TextPlace, text: &str) {
    let [x, y] = place.at.map(number);
    write!(
        output,
        "0\nTEXT\n8\n{layer}\n10\n{x}\n20\n{y}\n30\n0\n40\n{}\n1\n{}\n",
        number(place.height),
        r12_text::lossy(text),
    )
    .expect("writing to a String cannot fail");
    if place.angle_degrees != 0.0 {
        write!(output, "50\n{}\n", number(place.angle_degrees))
            .expect("writing to a String cannot fail");
    }
    if place.centred {
        write!(output, "72\n1\n11\n{x}\n21\n{y}\n31\n0\n")
            .expect("writing to a String cannot fail");
    }
}

fn line_record(output: &mut String, layer: &str, start: [f64; 2], end: [f64; 2]) {
    write!(
        output,
        "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
        number(start[0]),
        number(start[1]),
        number(end[0]),
        number(end[1]),
    )
    .expect("writing to a String cannot fail");
}

fn polyline(output: &mut String, layer: &str, points: &[[f64; 2]], closed: bool) {
    write!(
        output,
        "0\nPOLYLINE\n8\n{layer}\n66\n1\n10\n0\n20\n0\n30\n0\n70\n{}\n",
        u8::from(closed)
    )
    .expect("writing to a String cannot fail");
    for point in points {
        write!(
            output,
            "0\nVERTEX\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n",
            number(point[0]),
            number(point[1]),
        )
        .expect("writing to a String cannot fail");
    }
    write!(output, "0\nSEQEND\n8\n{layer}\n").expect("writing to a String cannot fail");
}

const fn plan_point(point: [f64; 3]) -> [f64; 2] {
    [point[0], point[1]]
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values of the TEXT records of `dxf`, in file order.
    fn text_values(dxf: &str) -> Vec<&str> {
        let lines: Vec<&str> = dxf.lines().collect();
        let mut values = Vec::new();
        let mut entity = "";
        for pair in lines.chunks(2) {
            let [code, data] = pair else { break };
            match *code {
                "0" => entity = data,
                "1" if entity == "TEXT" => values.push(*data),
                _ => {}
            }
        }
        values
    }

    /// Review 2026-10-24 MF-28: a line of 300 accented characters was written
    /// as one TEXT of 2100 characters, which no R12 reader takes.
    #[test]
    fn a_line_longer_than_an_r12_string_goes_on_over_the_next_lines() {
        let mut plan = PlanAnnotations::default();
        plan.text("plan", [0.0, 1000.0], 250.0, &"č".repeat(300));
        let values = text_values(&plan.entities);
        assert_eq!(values.len(), 9, "{values:?}");
        assert!(values.iter().all(|value| value.len() <= 255));
        assert_eq!(plan.drawn_texts, 9);
        // Each further line a line pitch below the one before.
        let second = 1000.0 - 250.0 * LINE_PITCH_PER_HEIGHT;
        assert!(
            plan.entities
                .contains(&format!("10\n0\n20\n{}\n", number(second))),
            "{}",
            plan.entities
        );
    }

    /// Review 2026-10-24 MF-28: the plan set a dimension's value half its
    /// height above the line where the sheets set it 1 mm on paper above.
    #[test]
    fn a_dimensions_value_sits_one_millimetre_on_paper_above_its_line() {
        let mut plan = PlanAnnotations::default();
        let (from, to) = ([0.0, 0.0, 0.0], [4000.0, 0.0, 0.0]);
        let layout = DimensionLayout::new(from, to, 500.0);
        assert!(plan.dimension("plan", [from, to], &layout, 250.0, 100.0));
        let line_y = layout.line[0][1];
        // The value at 1 mm × 100 above the line, not 250 / 2.
        assert!(
            plan.entities
                .contains(&format!("11\n2000\n21\n{}\n", number(line_y + 100.0))),
            "{}",
            plan.entities
        );
    }
}
