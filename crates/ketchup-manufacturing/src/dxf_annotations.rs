//! The drawing annotations of a plan DXF: a text as TEXT, a dimension as an aligned
//! DIMENSION with the block that draws it, a leader as a line, an arrowhead and a
//! TEXT, and a drawn line as a POLYLINE. Each annotation lies on the layer of its tag.
//! Text heights are millimetres on paper; at a scale of 1:N the file, which is in
//! model millimetres, writes them N times taller.

use crate::dxf_export::{R12Layers, number};
use ketchup_model::annotation::{AnnotationMark, DimensionLayout, dimension_text};
use ketchup_model::document::Snapshot;
use ketchup_model::tolerance::{MIN_DIMENSION_MM, ROUNDING};
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The scale of a plan DXF whose export names none: 1:100.
pub const DEFAULT_PLAN_SCALE: f64 = 100.0;
/// The largest scale denominator a plan takes, 1:100 000.
pub const MAX_PLAN_SCALE: f64 = 100_000.0;
/// The layer of an annotation without a tag.
const UNTAGGED_LAYER: &str = "Annotations";
/// On paper: the gap between a measured point and its extension line, how far the
/// extension line runs past the dimension line, the length of an oblique tick and of
/// a leader's arrowhead, and the arrowhead's width.
const EXTENSION_GAP_MM: f64 = 1.0;
const EXTENSION_BEYOND_MM: f64 = 2.0;
const TICK_MM: f64 = 3.0;
const ARROW_LENGTH_MM: f64 = 3.0;
const ARROW_WIDTH_MM: f64 = 1.0;

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
    /// A left-aligned text, one TEXT per line, the first on the baseline at `at`.
    fn text(&mut self, layer: &str, at: [f64; 2], height: f64, text: &str) {
        let mut baseline = at[1];
        for line in text.split('\n') {
            let place = TextPlace {
                at: [at[0], baseline],
                height,
                angle_degrees: 0.0,
                centred: false,
            };
            text_record(&mut self.entities, layer, &place, line);
            self.drawn_texts += 1;
            baseline -= height * LINE_PITCH_PER_HEIGHT;
        }
    }

    /// An aligned dimension and its block; `false` when it has no length in plan.
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
        let plan_length = distance(line[0], line[1]);
        if plan_length < MIN_DIMENSION_MM {
            return false;
        }
        let along = [
            (line[1][0] - line[0][0]) / plan_length,
            (line[1][1] - line[0][1]) / plan_length,
        ];
        // The value reads from left to right, or from bottom to top.
        let reading = if along[0] < -ROUNDING || (along[0].abs() <= ROUNDING && along[1] < 0.0) {
            [-along[0], -along[1]]
        } else {
            along
        };
        let up = [-reading[1], reading[0]];
        let middle = [
            (line[0][0] + line[1][0]) / 2.0,
            (line[0][1] + line[1][1]) / 2.0,
        ];
        let text_at = [
            middle[0] + up[0] * height / 2.0,
            middle[1] + up[1] * height / 2.0,
        ];
        let value = dimension_text(layout.value_mm);
        let shortened = dimension_text(distance(points[0], points[1])) != value;

        let block = format!("*D{}", self.dimensions + 1);
        write!(
            self.blocks,
            "0\nBLOCK\n8\n0\n2\n{block}\n70\n1\n10\n0\n20\n0\n30\n0\n3\n{block}\n"
        )
        .expect("writing to a String cannot fail");
        line_record(&mut self.blocks, "0", line[0], line[1]);
        let tick = TICK_MM * scale / 2.0 / std::f64::consts::SQRT_2;
        for end in line {
            let slant = [(along[0] - along[1]) * tick, (along[1] + along[0]) * tick];
            line_record(
                &mut self.blocks,
                "0",
                [end[0] - slant[0], end[1] - slant[1]],
                [end[0] + slant[0], end[1] + slant[1]],
            );
        }
        for (point, end) in points.into_iter().zip(line) {
            let reach = distance(point, end);
            let gap = EXTENSION_GAP_MM * scale;
            if reach <= gap {
                continue;
            }
            let toward = [(end[0] - point[0]) / reach, (end[1] - point[1]) / reach];
            let beyond = EXTENSION_BEYOND_MM * scale;
            line_record(
                &mut self.blocks,
                "0",
                [point[0] + toward[0] * gap, point[1] + toward[1] * gap],
                [end[0] + toward[0] * beyond, end[1] + toward[1] * beyond],
            );
        }
        let place = TextPlace {
            at: text_at,
            height,
            angle_degrees: reading[1].atan2(reading[0]).to_degrees(),
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
            write!(self.entities, "1\n{}\n", dxf_text(&value))
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
        let length = distance(point, at);
        if length < MIN_DIMENSION_MM {
            return;
        }
        let toward = [(at[0] - point[0]) / length, (at[1] - point[1]) / length];
        let arrow = (ARROW_LENGTH_MM * scale).min(length);
        let half = ARROW_WIDTH_MM * scale / 2.0;
        let base = [point[0] + toward[0] * arrow, point[1] + toward[1] * arrow];
        let side = [-toward[1] * half, toward[0] * half];
        let corners = [
            point,
            [base[0] + side[0], base[1] + side[1]],
            [base[0] - side[0], base[1] - side[1]],
            [base[0] - side[0], base[1] - side[1]],
        ];
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
        dxf_text(text),
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

/// A text as an R12 file holds it: ASCII, every other character as `\U+XXXX`
/// (one beyond U+FFFF as `?`), and a percent sign as `%%%` so that it is never
/// read as a control code such as `%%d`.
fn dxf_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '%' => output.push_str("%%%"),
            ' '..='~' => output.push(character),
            _ => match u16::try_from(u32::from(character)) {
                Ok(code) => {
                    write!(output, "\\U+{code:04X}").expect("writing to a String cannot fail");
                }
                Err(_) => output.push('?'),
            },
        }
    }
    output
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

    #[test]
    fn a_text_keeps_its_accents_and_percent_signs_in_an_ascii_file() {
        assert_eq!(
            dxf_text("Kotva č. 5 – 50 %"),
            "Kotva \\U+010D. 5 \\U+2013 50 %%%"
        );
        assert_eq!(dxf_text("S-05"), "S-05");
    }
}
