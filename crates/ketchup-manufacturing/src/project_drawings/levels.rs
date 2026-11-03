//! The level marks of the sections and elevations (Měchenice MECH-6): a building
//! authority reads heights against ±0,000, the floor of the lowest storey. Each
//! level is a line right of the view with a triangle standing on it and its
//! height above ±0,000 in metres beside, as Czech and Slovak drawings write it.

use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};

/// Levels nearer than this to a lower one are marked once (not a tolerance: a
/// millimetre is the last place a level is written to).
const SAME_LEVEL_MM: f64 = 1.0;
const LEVEL_TEXT_MM: f64 = 2.5;
// Page millimetres: the gap to the view, the length of the line, the triangle.
const GAP_MM: f64 = 2.0;
const LINE_MM: f64 = 14.0;
const TRIANGLE_MM: f64 = 1.6;
// not a tolerance: the pen width of a level mark on the sheet.
const STROKE_MM: f64 = 0.25;

/// The heights a section or an elevation marks, model millimetres lowest
/// first: ±0,000 (`datum_mm`) and each floor the view reaches from `low` to
/// `high`, and its top.
pub(super) fn level_heights(datum_mm: f64, floors: &[f64], low: f64, high: f64) -> Vec<f64> {
    let mut heights = std::iter::once(datum_mm)
        .chain(floors.iter().copied())
        .filter(|height| (low - SAME_LEVEL_MM..=high).contains(height))
        .chain(std::iter::once(high))
        .collect::<Vec<_>>();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|upper, lower| *upper - *lower < SAME_LEVEL_MM);
    heights
}

/// A height above ±0,000 as a level mark writes it: metres to the millimetre,
/// signed, with `separator` before the decimals ("±0,000", "+2,850", "-0,450").
pub(super) fn level_text(above_datum_mm: f64, separator: char) -> String {
    let whole_mm = above_datum_mm.round();
    let sign = if whole_mm > 0.0 {
        "+"
    } else if whole_mm < 0.0 {
        "-"
    } else {
        "±"
    };
    let metres = format!("{:.3}", whole_mm.abs() / 1000.0);
    format!("{sign}{}", metres.replace('.', &separator.to_string()))
}

/// Draws the level marks right of `right` (page mm): each `(y, text)` a line at
/// `y` with a triangle standing on it, the text after the triangle.
pub(super) fn draw_levels(page: &mut Page, right: f64, levels: &[(f64, String)]) {
    let stroke = Stroke::solid(STROKE_MM);
    let start = right + GAP_MM;
    let tip = start + TRIANGLE_MM * 2.0;
    for (y, text) in levels {
        page.line(vec![[start, *y], [start + LINE_MM, *y]], stroke.clone());
        // The page runs y down: the triangle stands above the line, tip on it.
        page.marks.push(Mark::Line {
            points: vec![
                [tip, *y],
                [tip - TRIANGLE_MM, *y - TRIANGLE_MM * 1.5],
                [tip + TRIANGLE_MM, *y - TRIANGLE_MM * 1.5],
            ],
            closed: true,
            stroke: stroke.clone(),
        });
        page.text(
            [tip + TRIANGLE_MM * 1.5, *y - 0.8],
            text,
            LEVEL_TEXT_MM,
            Anchor::Start,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_written_in_metres_against_zero() {
        assert_eq!(level_text(0.0, ','), "±0,000");
        assert_eq!(level_text(0.4, ','), "±0,000");
        assert_eq!(level_text(2850.0, ','), "+2,850");
        assert_eq!(level_text(-450.0, ','), "-0,450");
        assert_eq!(level_text(12_345.6, '.'), "+12.346");
    }

    #[test]
    fn the_view_marks_zero_its_floors_and_its_top_once_each() {
        // Floors at 0 and 3000, the top at 7400.
        assert_eq!(
            level_heights(0.0, &[0.0, 3000.0], -450.0, 7400.0),
            [0.0, 3000.0, 7400.0]
        );
        // A floor out of the view is not marked; one at the top is the top.
        assert_eq!(
            level_heights(0.0, &[9000.0, 7400.0], 0.0, 7400.0),
            [0.0, 7400.0]
        );
        // A model above ±0 marks its top alone.
        assert_eq!(level_heights(0.0, &[], 1000.0, 4000.0), [4000.0]);
    }

    #[test]
    fn each_level_draws_a_line_a_triangle_and_its_height() {
        let mut page = Page::default();
        draw_levels(&mut page, 100.0, &[(50.0, "±0,000".to_owned())]);
        let texts = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text { at, text, .. } => Some((*at, text.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].1, "±0,000");
        assert!(texts[0].0[0] > 100.0, "right of the view");
        assert!(texts[0].0[1] < 50.0, "over its line");
        let triangle = page.marks.iter().find_map(|mark| match mark {
            Mark::Line {
                points,
                closed: true,
                ..
            } => Some(points.clone()),
            _ => None,
        });
        let triangle = triangle.expect("a triangle");
        assert_eq!(triangle.len(), 3);
        assert!((triangle[0][1] - 50.0).abs() < 1e-9, "its tip on the line");
    }
}
