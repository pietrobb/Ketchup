//! The level marks of the sections and elevations (Měchenice MECH-6): a building
//! authority reads heights against ±0,000, the floor of the ground storey. Each
//! level is a line right of the view with a triangle standing on it and its
//! height above ±0,000 in metres beside, as Czech and Slovak drawings write it.

use ketchup_model::annotation::DecimalMark;
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};

/// Levels nearer than this to a lower one are marked once (not a tolerance: a
/// millimetre is the last place a level is written to).
const SAME_LEVEL_MM: f64 = 1.0;
const LEVEL_TEXT_MM: f64 = 2.5;
// Page millimetres: the gap to the view, the length of the line, the triangle.
const GAP_MM: f64 = 2.0;
const LINE_MM: f64 = 14.0;
const TRIANGLE_MM: f64 = 1.6;
// Page millimetres: how far apart two marks stand at least (a triangle and its
// text over the line), and each run of a line bent to a mark moved off it.
const MARK_MM: f64 = 4.0;
const KINK_MM: f64 = 1.5;
// not a tolerance: the pen width of a level mark on the sheet.
const STROKE_MM: f64 = 0.25;

/// The heights a section or an elevation marks, model millimetres lowest
/// first: ±0,000 (`datum_mm`) and each floor the view reaches from `low` to
/// `high`, its top, and its bottom when that is below ±0,000 (the
/// foundations, review 2026-11-04 MF-141).
pub(super) fn level_heights(datum_mm: f64, floors: &[f64], low: f64, high: f64) -> Vec<f64> {
    let mut heights = std::iter::once(datum_mm)
        .chain(floors.iter().copied())
        .filter(|height| (low - SAME_LEVEL_MM..=high).contains(height))
        .chain((low < datum_mm - SAME_LEVEL_MM).then_some(low))
        .chain(std::iter::once(high))
        .collect::<Vec<_>>();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|upper, lower| *upper - *lower < SAME_LEVEL_MM);
    heights
}

/// A height above ±0,000 as a level mark writes it: metres to the millimetre,
/// signed, with `separator` before the decimals ("±0,000", "+2,850", "-0,450").
pub(super) fn level_text(above_datum_mm: f64, separator: DecimalMark) -> String {
    let whole_mm = above_datum_mm.round();
    let sign = if whole_mm > 0.0 {
        "+"
    } else if whole_mm < 0.0 {
        "-"
    } else {
        "±"
    };
    let metres = format!("{:.3}", whole_mm.abs() / 1000.0);
    format!("{sign}{}", separator.written(&metres))
}

/// Draws the level marks right of `right` (page mm): each `(y, text)` a line
/// from `y` with a triangle standing on it, the text after the triangle. Marks
/// nearer together than a mark is high (review VRAT-8) are spread apart about
/// where they belong, each line bending from its own height up or down to its
/// mark.
pub(super) fn draw_levels(page: &mut Page, right: f64, levels: &[(f64, String)]) {
    let stroke = Stroke::solid(STROKE_MM);
    let start = right + GAP_MM;
    let tip = start + KINK_MM * 2.0 + TRIANGLE_MM;
    let mut order = (0..levels.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| levels[*b].0.total_cmp(&levels[*a].0));
    let marks = spread(
        &order
            .iter()
            .map(|index| levels[*index].0)
            .collect::<Vec<_>>(),
    );
    let mut at = vec![0.0; levels.len()];
    for (index, mark) in order.into_iter().zip(marks) {
        at[index] = mark;
    }
    for ((y, text), mark) in levels.iter().zip(at) {
        page.line(
            vec![
                [start, *y],
                [start + KINK_MM, *y],
                [start + KINK_MM * 2.0, mark],
                [start + LINE_MM, mark],
            ],
            stroke.clone(),
        );
        // The page runs y down: the triangle stands above the line, tip on it.
        page.marks.push(Mark::Line {
            points: vec![
                [tip, mark],
                [tip - TRIANGLE_MM, mark - TRIANGLE_MM * 1.5],
                [tip + TRIANGLE_MM, mark - TRIANGLE_MM * 1.5],
            ],
            closed: true,
            stroke: stroke.clone(),
        });
        page.text(
            [tip + TRIANGLE_MM * 1.5, mark - 0.8],
            text,
            LEVEL_TEXT_MM,
            Anchor::Start,
        );
    }
}

/// Where the marks of levels at page `ys` (lowest first, so `y` falling)
/// stand so that each is at least [`MARK_MM`] above the one below: a run of
/// marks too near together is spaced [`MARK_MM`] apart about the mean of where
/// its levels are, the runs merged until none crowds the next.
fn spread(ys: &[f64]) -> Vec<f64> {
    // Each run: its levels `ys[first..end]`, and where its lowest mark stands.
    let mut runs: Vec<(usize, usize, f64)> = Vec::new();
    for (index, y) in ys.iter().enumerate() {
        runs.push((index, index + 1, *y));
        while let [.., (first, middle, low), (_, end, next)] = runs[..] {
            let top = stacked(low, middle - first).last().unwrap_or(low);
            if top - next >= MARK_MM {
                break;
            }
            // The mean of each level less the step its mark is above the lowest.
            let (sum, count) = ys[first..end]
                .iter()
                .zip(stacked(0.0, end - first))
                .fold((0.0, 0.0), |(sum, count), (y, step)| {
                    (sum + y - step, count + 1.0)
                });
            runs.truncate(runs.len() - 2);
            runs.push((first, end, sum / count));
        }
    }
    runs.into_iter()
        .flat_map(|(first, end, low)| stacked(low, end - first))
        .collect()
}

/// `count` marks [`MARK_MM`] apart up the page from `low`.
fn stacked(low: f64, count: usize) -> impl Iterator<Item = f64> {
    std::iter::successors(Some(low), |y| Some(y - MARK_MM)).take(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_written_in_metres_against_zero() {
        assert_eq!(level_text(0.0, DecimalMark::Comma), "±0,000");
        assert_eq!(level_text(0.4, DecimalMark::Comma), "±0,000");
        assert_eq!(level_text(2850.0, DecimalMark::Comma), "+2,850");
        assert_eq!(level_text(-450.0, DecimalMark::Comma), "-0,450");
        assert_eq!(level_text(12_345.6, DecimalMark::Point), "+12.346");
    }

    #[test]
    fn the_view_marks_zero_its_floors_and_its_top_once_each() {
        // Floors at 0 and 3000, the top at 7400, the foundations down to -450.
        assert_eq!(
            level_heights(0.0, &[0.0, 3000.0], -450.0, 7400.0),
            [-450.0, 0.0, 3000.0, 7400.0]
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

    /// Review VRAT-8: ±0,000 and +0,150 at 1:200 are 0.75 mm apart, the eaves
    /// and the ridge of a low roof not much more; their marks stood over each
    /// other and neither height could be read.
    #[test]
    fn levels_too_near_together_are_marked_apart_each_from_its_height() {
        let mut page = Page::default();
        let levels = [
            (100.0, "±0,000".to_owned()),
            (99.25, "+0,150".to_owned()),
            (98.5, "+0,300".to_owned()),
            (60.0, "+8,000".to_owned()),
        ];
        draw_levels(&mut page, 10.0, &levels);
        let mut texts = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text { at, .. } => Some(at[1]),
                _ => None,
            })
            .collect::<Vec<_>>();
        texts.sort_by(f64::total_cmp);
        assert_eq!(texts.len(), 4);
        for pair in texts.windows(2) {
            assert!(pair[1] - pair[0] >= LEVEL_TEXT_MM, "{texts:?}");
        }
        let triangles = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line {
                    points,
                    closed: true,
                    ..
                } => Some(points[0][1]),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut tips = triangles.clone();
        tips.sort_by(f64::total_cmp);
        for pair in tips.windows(2) {
            assert!(pair[1] - pair[0] >= TRIANGLE_MM * 1.5, "{tips:?}");
        }
        // Each line still starts at its own height by the view.
        let starts = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line {
                    points,
                    closed: false,
                    ..
                } => Some(points[0][1]),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(starts, [100.0, 99.25, 98.5, 60.0]);
        // A mark far from the others stays on its own line.
        assert!(triangles.contains(&60.0), "{triangles:?}");
    }

    /// Vratimov VRAT-11: the levels of a hall at 1:400, from the ground to its
    /// crown 25 mm above it, its doors, burner and tank between 0.1 mm and
    /// 7 mm up; every mark stands at least 4 mm from the next.
    #[test]
    fn the_levels_of_a_hall_at_1_400_stand_4_mm_apart() {
        let heights = [0.0, 40.0, 140.0, 1370.0, 1520.0, 2222.0, 2792.0, 9990.0];
        let levels = heights.map(|height| (200.0 - height / 400.0, String::new()));
        let mut page = Page::default();
        draw_levels(&mut page, 10.0, &levels);
        let mut tips = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line {
                    points,
                    closed: true,
                    ..
                } => Some(points[0][1]),
                _ => None,
            })
            .collect::<Vec<_>>();
        tips.sort_by(f64::total_cmp);
        assert_eq!(tips.len(), heights.len());
        for pair in tips.windows(2) {
            assert!(pair[1] - pair[0] >= 4.0 - 1e-9, "{tips:?}");
        }
    }
}
