//! The section marks of the project plans (review 2026-11-08 §5.2): where
//! the longitudinal section (A-A) and the cross section (B-B) cut a plan, a
//! dash-dot line across it, thick past its ends, with an arrow at each end
//! toward where the section looks and the section's letter.

use super::{Point, ProjectView, ViewFrame};
use ketchup_geometry::linalg::{dot, dot2};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::tolerance::NEGLIGIBLE;

/// One section's mark on a plan, in view millimetres (y up).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SectionMark {
    pub letter: &'static str,
    /// A point where the section's plane crosses the plan.
    pub through: [f64; 2],
    /// The way the section looks, a unit direction.
    pub toward: [f64; 2],
}

/// The letter of section `view`: A along the model, B across it.
pub(super) const fn letter(view: ProjectView) -> Option<&'static str> {
    match view {
        ProjectView::LongitudinalSection => Some("A"),
        ProjectView::CrossSection => Some("B"),
        _ => None,
    }
}

/// The title of section `view` with its letters ("Longitudinal section
/// A-A"); `title` of any other view.
pub(super) fn lettered(view: ProjectView, title: String) -> String {
    match letter(view) {
        Some(letter) => format!("{title} {letter}-{letter}"),
        None => title,
    }
}

/// The marks on a plan seen along `plan` of the sections among `frames`; a
/// section whose plane lies flat in the plan has none.
pub(super) fn section_marks(
    plan: &ViewFrame,
    frames: &[(ProjectView, ViewFrame)],
) -> Vec<SectionMark> {
    frames
        .iter()
        .filter_map(|(view, frame)| {
            let letter = letter(*view)?;
            let cut = frame.cut?;
            let toward = [dot(frame.depth, plan.right), dot(frame.depth, plan.up)];
            let length = toward[0].hypot(toward[1]);
            if length <= NEGLIGIBLE {
                return None;
            }
            // The plane holds the points p with p · depth = cut.
            let on_plane: Point = frame.depth.map(|value| value * cut);
            Some(SectionMark {
                letter,
                through: [dot(on_plane, plan.right), dot(on_plane, plan.up)],
                toward: toward.map(|value| value / length),
            })
        })
        .collect()
}

/// How far the thin line runs past what the view draws, in page millimetres:
/// with the thick stroke past it, the arrow and its letter stay clear of the
/// overall dimensions 8 mm below and left of the view.
const OVERRUN_MM: f64 = 1.0;
/// The thick stroke past the thin line's end.
const END_MM: f64 = 2.0;
const ARROW_MM: f64 = 4.0;
const LETTER_MM: f64 = 3.5;
// not a tolerance: the pen width of the section line.
const LINE_STROKE_MM: f64 = 0.18;
// not a tolerance: the pen width of its ends past the view.
const END_STROKE_MM: f64 = 0.7;

/// Draws `mark` across the view `min`..`max` (view millimetres), which `at`
/// puts on the page (page millimetres, y down); nothing when the plane
/// misses the view.
pub(super) fn draw_section_mark(
    page: &mut Page,
    mark: &SectionMark,
    [min, max]: [[f64; 2]; 2],
    at: impl Fn([f64; 2]) -> [f64; 2],
) {
    let along = [-mark.toward[1], mark.toward[0]];
    let corners = [min, [max[0], min[1]], max, [min[0], max[1]]];
    let offsets = corners.map(|corner| {
        let from = [corner[0] - mark.through[0], corner[1] - mark.through[1]];
        (dot2(from, along), dot2(from, mark.toward))
    });
    let (mut first, mut last) = (f64::INFINITY, f64::NEG_INFINITY);
    for (along_line, _) in offsets {
        first = first.min(along_line);
        last = last.max(along_line);
    }
    let crosses = offsets.iter().any(|(_, across)| *across <= 0.0)
        && offsets.iter().any(|(_, across)| *across >= 0.0);
    if !crosses {
        return;
    }
    let on_line = |at_along: f64| {
        [
            mark.through[0] + along[0] * at_along,
            mark.through[1] + along[1] * at_along,
        ]
    };
    let (start, end) = (at(on_line(first)), at(on_line(last)));
    let span = (end[0] - start[0]).hypot(end[1] - start[1]);
    if span <= NEGLIGIBLE {
        return;
    }
    let unit = [(end[0] - start[0]) / span, (end[1] - start[1]) / span];
    let past = |point: [f64; 2], outward: f64, by: f64| {
        [
            point[0] + unit[0] * outward * by,
            point[1] + unit[1] * outward * by,
        ]
    };
    page.line(
        vec![past(start, -1.0, OVERRUN_MM), past(end, 1.0, OVERRUN_MM)],
        Stroke {
            width_mm: LINE_STROKE_MM,
            color: [0, 0, 0],
            dash_mm: vec![8.0, 1.5, 1.0, 1.5],
        },
    );
    // On the page y runs down.
    let toward = [mark.toward[0], -mark.toward[1]];
    for (point, outward) in [(start, -1.0), (end, 1.0)] {
        let inner = past(point, outward, OVERRUN_MM);
        let outer = past(point, outward, OVERRUN_MM + END_MM);
        page.line(vec![inner, outer], Stroke::solid(END_STROKE_MM));
        draw_arrow(page, outer, toward, mark.letter);
    }
}

/// An arrow from `foot` along `toward` (page), its head filled, and `letter`
/// past its tip.
fn draw_arrow(page: &mut Page, foot: [f64; 2], toward: [f64; 2], letter: &str) {
    let across = [-toward[1], toward[0]];
    let point = |forward: f64, side: f64| {
        [
            foot[0] + toward[0] * forward + across[0] * side,
            foot[1] + toward[1] * forward + across[1] * side,
        ]
    };
    page.line(
        vec![foot, point(ARROW_MM, 0.0)],
        Stroke::solid(LINE_STROKE_MM),
    );
    let head = ARROW_MM * 0.4;
    page.marks.push(Mark::Fill {
        loops: vec![vec![
            point(ARROW_MM, 0.0),
            point(ARROW_MM - head, head / 2.5),
            point(ARROW_MM - head, -head / 2.5),
        ]],
        color: [0, 0, 0],
        // An arrowhead: a DXF draws its outline on the FILL layer.
        outline: None,
    });
    let [x, y] = point(ARROW_MM + LETTER_MM * 0.8, 0.0);
    // Lowered by about a third of its height, so it is centred where it is put.
    page.text([x, y + LETTER_MM / 3.0], letter, LETTER_MM, Anchor::Middle);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters_at(page: &Page) -> Vec<(String, [f64; 2])> {
        page.marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text { at, text, .. } => Some((text.clone(), *at)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_plan_marks_section_a_along_the_model_and_b_across_it_looking_where_they_look() {
        let frames = super::super::view_frames([0.0; 3], [6000.0, 3000.0, 2500.0], 1000.0, false);
        let plan = frames[0].1;
        let marks = section_marks(&plan, &frames);
        assert_eq!(marks.len(), 2, "{marks:?}");
        let (a, b) = (&marks[0], &marks[1]);
        assert_eq!(a.letter, "A");
        // Across the middle of the 3000 mm width, looking toward +y.
        assert!((a.through[1] - 1500.0).abs() < 1e-9, "{a:?}");
        assert!((a.toward[1] - 1.0).abs() < 1e-12, "{a:?}");
        assert_eq!(b.letter, "B");
        assert!((b.through[0] - 3000.0).abs() < 1e-9, "{b:?}");
        assert!((b.toward[0] - 1.0).abs() < 1e-12, "{b:?}");

        // At 1:100, the view's lower left at page (50, 200), y down.
        let at = |point: [f64; 2]| [50.0 + point[0] / 100.0, 200.0 - point[1] / 100.0];
        let view = [[0.0, 0.0], [6000.0, 3000.0]];
        let mut page = Page::default();
        draw_section_mark(&mut page, a, view, at);
        let letters = letters_at(&page);
        assert_eq!(letters.len(), 2, "{letters:?}");
        for (letter, [x, y]) in &letters {
            assert_eq!(letter, "A");
            // Past the left and the right of the view, above its line (185).
            assert!(*x < 50.0 || *x > 110.0, "{letters:?}");
            assert!(*y < 185.0, "{letters:?}");
        }
        let mut page = Page::default();
        draw_section_mark(&mut page, b, view, at);
        for (letter, [x, y]) in letters_at(&page) {
            assert_eq!(letter, "B");
            // Below and above the view, right of its line (80).
            assert!(!(170.0..=200.0).contains(&y), "{y}");
            assert!(x > 80.0, "{x}");
        }

        // A plane past the view draws nothing.
        let mut page = Page::default();
        let away = SectionMark {
            through: [0.0, 9000.0],
            ..a.clone()
        };
        draw_section_mark(&mut page, &away, view, at);
        assert!(page.marks.is_empty());
    }
}
