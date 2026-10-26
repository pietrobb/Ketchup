//! A drawing sheet as AutoCAD R12 (AC1009) DXF in paper millimetres, origin at
//! the sheet's lower left corner: every line as LINE entities on the layer of
//! its pen, the outlines of filled areas (the parts a cut passes through) on
//! layer CUT and every text as TEXT on layer TEXT. R12 has no hatch, so a fill
//! keeps its outline only; the sheet's PDF shows the fill.
//!
//! A text is written as [`r12_text`] writes it, so the file needs no code
//! page; a character beyond the Basic Multilingual Plane, which no escape
//! holds, is written as "?", as the PDF prints it.

use crate::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::r12_text;
use ketchup_model::underlay::text::LINE_PITCH_PER_HEIGHT;
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// The layer of the outlines of filled areas.
pub const CUT_LAYER: &str = "CUT";
/// The layer of the outlines of fills drawn without one (arrowheads).
pub const FILL_LAYER: &str = "FILL";
pub const TEXT_LAYER: &str = "TEXT";

fn number(value: f64) -> String {
    let mut text = format!("{value:.4}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

/// The layer of a pen: its width in hundredths of a millimetre, and DASHED
/// for a dashed one, e.g. `PEN-0_18`; R12 layer names allow no point.
#[must_use]
pub fn pen_layer(stroke: &Stroke) -> String {
    let mut name = format!("PEN-{}", number(stroke.width_mm).replace('.', "_"));
    if !stroke.dash_mm.is_empty() {
        name.push_str("-DASHED");
    }
    name
}

struct Writer {
    height: f64,
    out: String,
}

impl Writer {
    fn point(&self, [x, y]: [f64; 2]) -> [String; 2] {
        [number(x), number(self.height - y)]
    }

    fn line(&mut self, layer: &str, a: [f64; 2], b: [f64; 2]) {
        let ([ax, ay], [bx, by]) = (self.point(a), self.point(b));
        let _ = write!(
            self.out,
            "0\nLINE\n8\n{layer}\n10\n{ax}\n20\n{ay}\n30\n0\n11\n{bx}\n21\n{by}\n31\n0\n"
        );
    }

    /// The sides of a polyline as lines; a side of no length draws nothing.
    fn path(&mut self, layer: &str, points: &[[f64; 2]], closed: bool) {
        let closing = (closed && points.len() > 2).then(|| [points[points.len() - 1], points[0]]);
        for [a, b] in points
            .windows(2)
            .map(|pair| [pair[0], pair[1]])
            .chain(closing)
        {
            if a != b {
                self.line(layer, a, b);
            }
        }
    }

    /// A text as one TEXT, or, longer than an R12 string holds, as several,
    /// each a line pitch further down its own up direction.
    fn text(&mut self, at: [f64; 2], text: &str, height_mm: f64, anchor: Anchor, angle_deg: f64) {
        if text.trim().is_empty() {
            return;
        }
        let turn = if angle_deg == 0.0 {
            String::new()
        } else {
            format!("50\n{}\n", number(angle_deg))
        };
        // Down the text on a page that runs y down.
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let step = LINE_PITCH_PER_HEIGHT * height_mm;
        let mut down = 0.0;
        for piece in r12_text::lines(text, f64::INFINITY) {
            let [x, y] = self.point([at[0] + sin * down, at[1] + cos * down]);
            // R12 reads a justified text at its alignment point (11/21).
            let justified = match anchor {
                Anchor::Start => String::new(),
                Anchor::Middle => format!("72\n1\n11\n{x}\n21\n{y}\n31\n0\n"),
                Anchor::End => format!("72\n2\n11\n{x}\n21\n{y}\n31\n0\n"),
            };
            let _ = write!(
                self.out,
                "0\nTEXT\n8\n{TEXT_LAYER}\n10\n{x}\n20\n{y}\n30\n0\n40\n{}\n1\n{}\n{turn}{justified}",
                number(height_mm),
                r12_text::lossy(&piece)
            );
            down += step;
        }
    }
}

/// The layers `page` draws on, each once, in name order, with whether it is dashed.
fn layers(page: &Page) -> BTreeSet<(String, bool)> {
    let mut layers = BTreeSet::new();
    for mark in &page.marks {
        match mark {
            Mark::Line { stroke, .. } => {
                layers.insert((pen_layer(stroke), !stroke.dash_mm.is_empty()));
            }
            Mark::Fill { outline, .. } => {
                let layer = if outline.is_some() {
                    CUT_LAYER
                } else {
                    FILL_LAYER
                };
                layers.insert((layer.to_owned(), false));
            }
            Mark::Text { .. } => {
                layers.insert((TEXT_LAYER.to_owned(), false));
            }
        }
    }
    layers
}

/// `page` as an R12 DXF in paper millimetres, y up from its lower left corner.
#[must_use]
pub fn page_dxf(page: &Page) -> Vec<u8> {
    let [width, height] = page.size_mm;
    let layers = layers(page);
    let mut out = format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$DWGCODEPAGE\n3\nANSI_1252\n9\n$INSUNITS\n70\n4\n9\n$EXTMIN\n10\n0\n20\n0\n30\n0\n9\n$EXTMAX\n10\n{}\n20\n{}\n30\n0\n0\nENDSEC\n0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLTYPE\n70\n2\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0\n0\nLTYPE\n2\nDASHED\n70\n0\n3\nDashed line\n72\n65\n73\n2\n40\n4.5\n49\n3\n49\n-1.5\n0\nENDTAB\n0\nTABLE\n2\nLAYER\n70\n{}\n",
        number(width),
        number(height),
        layers.len()
    );
    for (layer, dashed) in &layers {
        let linetype = if *dashed { "DASHED" } else { "CONTINUOUS" };
        let _ = write!(out, "0\nLAYER\n2\n{layer}\n70\n0\n62\n7\n6\n{linetype}\n");
    }
    out.push_str("0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    let mut writer = Writer { height, out };
    for mark in &page.marks {
        match mark {
            Mark::Line {
                points,
                closed,
                stroke,
            } => writer.path(&pen_layer(stroke), points, *closed),
            Mark::Fill { loops, outline, .. } => {
                let layer = if outline.is_some() {
                    CUT_LAYER
                } else {
                    FILL_LAYER
                };
                for points in loops {
                    writer.path(layer, points, true);
                }
            }
            Mark::Text {
                at,
                text,
                height_mm,
                anchor,
                angle_deg,
                ..
            } => writer.text(*at, text, *height_mm, *anchor, *angle_deg),
        }
    }
    let mut out = writer.out;
    out.push_str("0\nENDSEC\n0\nEOF\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_becomes_lines_cut_outlines_and_texts_in_paper_millimetres() {
        let mut page = Page {
            size_mm: [420.0, 297.0],
            marks: Vec::new(),
        };
        page.rect([10.0, 10.0], [410.0, 287.0], Stroke::solid(0.7));
        page.marks.push(Mark::Fill {
            loops: vec![vec![[50.0, 50.0], [80.0, 50.0], [80.0, 70.0]]],
            color: [100, 80, 55],
            outline: Some(Stroke::solid(0.5)),
        });
        page.text([200.0, 20.0], "Pôdorys 😀", 3.5, Anchor::Middle);
        let dxf = String::from_utf8(page_dxf(&page)).unwrap();
        assert!(dxf.contains("$ACADVER\n1\nAC1009"));
        assert!(dxf.contains("0\nLAYER\n2\nPEN-0_7\n"), "{dxf}");
        assert!(dxf.contains("0\nLAYER\n2\nCUT\n"));
        assert!(dxf.contains("0\nLAYER\n2\nTEXT\n"));
        let lines = |layer: &str| dxf.matches(&format!("0\nLINE\n8\n{layer}\n")).count();
        assert_eq!(lines("PEN-0_7"), 4, "the closed frame");
        assert_eq!(lines("CUT"), 3, "the outline of the filled triangle");
        // y is measured up from the lower edge: the frame's top at 297 - 10.
        assert!(dxf.contains("10\n10\n20\n287\n30\n0\n11\n410\n21\n287\n"));
        // Centred on its alignment point, the letters escaped, the emoji as "?".
        assert!(
            dxf.contains("1\nP\\U+00F4dorys ?\n72\n1\n11\n200\n21\n277\n"),
            "{dxf}"
        );
        assert!(dxf.ends_with("0\nENDSEC\n0\nEOF\n"));
    }

    /// Review 2026-10-24 MF-28: a sheet wrote "%%d" as it is, so a reader drew
    /// a degree sign, and wrote a text of any length as one TEXT.
    #[test]
    fn a_percent_sign_stays_one_and_a_long_text_goes_on_over_the_next_lines() {
        let mut page = Page {
            size_mm: [420.0, 297.0],
            marks: Vec::new(),
        };
        page.text([10.0, 20.0], "50 %%d", 3.5, Anchor::Start);
        page.text([10.0, 100.0], &"ž".repeat(40), 2.0, Anchor::Start);
        let dxf = String::from_utf8(page_dxf(&page)).unwrap();
        assert!(dxf.contains("\n1\n50 %%%%%%d\n"), "{dxf}");
        // 36 letters of seven characters fill one R12 string, the rest go
        // one line pitch further down the page.
        let first = format!("\n1\n{}\n", "\\U+017E".repeat(36));
        let second = format!(
            "20\n{}\n30\n0\n40\n2\n1\n{}\n",
            number(297.0 - 100.0 - 2.0 * LINE_PITCH_PER_HEIGHT),
            "\\U+017E".repeat(4)
        );
        assert!(dxf.contains(&first) && dxf.contains(&second), "{dxf}");
    }
}
