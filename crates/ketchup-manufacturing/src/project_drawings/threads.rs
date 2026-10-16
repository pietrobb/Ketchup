//! Threads drawn by ISO 6410-1 on solids that stay plain cylinders: a thin
//! continuous line at the thread root along the thread, a thick line across
//! the major diameter where the full thread ends, and, seen end on, a thin
//! three-quarter circle at the root. An internal thread is hidden in a view
//! and shows only where a section cuts it open.

use super::{CUT_STROKE_MM, EDGE_STROKE_MM, Item, Point, ViewFrame};
use ketchup_geometry::linalg::{add, cross, dot, scale};
use ketchup_model::tolerance::ROUNDING;
use std::f64::consts::FRAC_PI_2;

/// A thread on a cylinder of a solid, in world millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingThread {
    /// Where the thread starts, on its axis.
    pub start_mm: Point,
    /// The unit direction the thread runs in from `start_mm`; it ends there.
    pub direction: Point,
    pub length_mm: f64,
    /// The nominal diameter: the crest of a shaft's thread, the root of a
    /// hole's.
    pub major_diameter_mm: f64,
    /// Where the thin line goes: the minor diameter of a shaft's thread, the
    /// major diameter of a hole's.
    pub root_diameter_mm: f64,
    /// Inside a hole rather than on a shaft.
    pub internal: bool,
}

/// ISO 128-20's thinnest line: the thread root, thinner than the edges.
// not a tolerance: a pen width on the sheet.
const ROOT_STROKE_MM: f64 = 0.13;
/// Straight pieces of a drawn quarter circle.
const CHORDS_PER_QUARTER: u32 = 8;

/// A thread's lines in one view.
#[derive(Default)]
pub(super) struct ThreadMarks {
    /// Lines painted among the faces, with their depth.
    pub(super) sorted: Vec<(f64, Item)>,
    /// Lines on the section, drawn over its fill.
    pub(super) on_cut: Vec<Item>,
}

impl ThreadMarks {
    /// Adds the lines of `thread` seen in `frame`: along its side when its
    /// axis lies square to the view, its root circle when the view looks
    /// along the axis, nothing when the axis is oblique or cut away.
    pub(super) fn add(&mut self, frame: &ViewFrame, thread: &DrawingThread) {
        let start = thread.start_mm;
        let end = add(start, scale(thread.direction, thread.length_mm));
        let along = dot(thread.direction, frame.depth);
        let root = thread.root_diameter_mm / 2.0;
        let crest = thread.major_diameter_mm / 2.0;
        if along.abs() >= 1.0 - ROUNDING {
            let (start_depth, end_depth) = (dot(start, frame.depth), dot(end, frame.depth));
            let (near, far) = (start_depth.min(end_depth), start_depth.max(end_depth));
            let circle = |width_mm| Item::Thread {
                points: three_quarter_circle(frame, start, root),
                width_mm,
            };
            match frame.cut {
                Some(cut) if far < cut => {}
                Some(cut) if near < cut => self.on_cut.push(circle(ROOT_STROKE_MM)),
                // At the near end, in front of the face it ends on.
                _ => self.sorted.push((near - 1.0, circle(ROOT_STROKE_MM))),
            }
        } else if along.abs() <= ROUNDING {
            let across = cross(thread.direction, frame.depth);
            let at = |point: Point, offset: f64| frame.project(add(point, scale(across, offset)));
            let lines = |termination_mm| {
                [
                    Item::Thread {
                        points: vec![at(start, root), at(end, root)],
                        width_mm: ROOT_STROKE_MM,
                    },
                    Item::Thread {
                        points: vec![at(start, -root), at(end, -root)],
                        width_mm: ROOT_STROKE_MM,
                    },
                    Item::Thread {
                        points: vec![at(end, crest), at(end, -crest)],
                        width_mm: termination_mm,
                    },
                ]
            };
            let axis_depth = dot(start, frame.depth);
            match frame.cut {
                Some(cut) if (axis_depth - cut).abs() < crest => {
                    self.on_cut.extend(lines(CUT_STROKE_MM));
                }
                Some(cut) if axis_depth < cut => {}
                _ if thread.internal => {}
                // On the near side of the shaft, in front of its faces.
                _ => self
                    .sorted
                    .extend(lines(EDGE_STROKE_MM).map(|item| (axis_depth - crest - 1.0, item))),
            }
        }
    }
}

/// Three quarters of the circle of `radius` about `centre` in the view, open
/// in its upper right quadrant.
fn three_quarter_circle(frame: &ViewFrame, centre: Point, radius: f64) -> Vec<[f64; 2]> {
    let [x, y] = frame.project(centre);
    let chords = 3 * CHORDS_PER_QUARTER;
    (0..=chords)
        .map(|index| {
            let angle = FRAC_PI_2 * (1.0 + 3.0 * f64::from(index) / f64::from(chords));
            [x + radius * angle.cos(), y + radius * angle.sin()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking along +y with x to the right and z up, cut at `cut`.
    fn front(cut: Option<f64>) -> ViewFrame {
        ViewFrame {
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            depth: [0.0, 1.0, 0.0],
            cut,
        }
    }

    /// An M8 thread, 20 mm along +x from (0, 50, 0).
    fn m8(internal: bool) -> DrawingThread {
        DrawingThread {
            start_mm: [0.0, 50.0, 0.0],
            direction: [1.0, 0.0, 0.0],
            length_mm: 20.0,
            major_diameter_mm: 8.0,
            root_diameter_mm: if internal { 8.0 } else { 6.466 },
            internal,
        }
    }

    fn lines(items: &[&Item]) -> Vec<(Vec<[f64; 2]>, f64)> {
        items
            .iter()
            .map(|item| match item {
                Item::Thread { points, width_mm } => (points.clone(), *width_mm),
                _ => panic!("a thread draws thread lines only"),
            })
            .collect()
    }

    #[test]
    fn a_shaft_seen_from_the_side_shows_its_root_lines_and_where_the_thread_ends() {
        let mut marks = ThreadMarks::default();
        marks.add(&front(None), &m8(false));
        assert!(marks.on_cut.is_empty());
        // In front of the shaft's near side (y = 46).
        assert!(marks.sorted.iter().all(|(depth, _)| *depth < 46.0));
        let drawn = lines(
            &marks
                .sorted
                .iter()
                .map(|(_, item)| item)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            drawn,
            [
                (vec![[0.0, 3.233], [20.0, 3.233]], ROOT_STROKE_MM),
                (vec![[0.0, -3.233], [20.0, -3.233]], ROOT_STROKE_MM),
                (vec![[20.0, 4.0], [20.0, -4.0]], EDGE_STROKE_MM),
            ]
        );
    }

    #[test]
    fn a_tapped_hole_is_hidden_in_a_view_and_drawn_where_a_section_opens_it() {
        let mut view = ThreadMarks::default();
        view.add(&front(None), &m8(true));
        assert!(view.sorted.is_empty() && view.on_cut.is_empty());

        let mut section = ThreadMarks::default();
        section.add(&front(Some(50.0)), &m8(true));
        assert!(section.sorted.is_empty());
        let drawn = lines(&section.on_cut.iter().collect::<Vec<_>>());
        assert_eq!(
            drawn,
            [
                (vec![[0.0, 4.0], [20.0, 4.0]], ROOT_STROKE_MM),
                (vec![[0.0, -4.0], [20.0, -4.0]], ROOT_STROKE_MM),
                (vec![[20.0, 4.0], [20.0, -4.0]], CUT_STROKE_MM),
            ]
        );

        // A section in front of the hole has cut it away.
        let mut beyond = ThreadMarks::default();
        beyond.add(&front(Some(60.0)), &m8(true));
        assert!(beyond.sorted.is_empty() && beyond.on_cut.is_empty());
    }

    #[test]
    fn seen_end_on_a_thread_is_a_three_quarter_root_circle_open_upper_right() {
        // Looking along the thread from x < 0, y to the right.
        let along = ViewFrame {
            right: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            depth: [1.0, 0.0, 0.0],
            cut: None,
        };
        let mut marks = ThreadMarks::default();
        marks.add(&along, &m8(false));
        let [(depth, Item::Thread { points, width_mm })] = marks.sorted.as_slice() else {
            panic!("one circle");
        };
        assert!(*depth < 0.0 && (*width_mm - ROOT_STROKE_MM).abs() < f64::EPSILON);
        assert!(points.iter().all(|[y, z]| {
            ((y - 50.0).hypot(*z) - 3.233).abs() < 1e-9 && !(*y > 50.0 + 1e-9 && *z > 1e-9)
        }));
        let first = points.first().unwrap();
        let last = points.last().unwrap();
        assert!((first[0] - 50.0).abs() < 1e-9 && (first[1] - 3.233).abs() < 1e-9);
        assert!((last[0] - 53.233).abs() < 1e-9 && last[1].abs() < 1e-9);

        // A section through the thread draws the circle on the cut.
        let mut cut = ThreadMarks::default();
        cut.add(
            &ViewFrame {
                cut: Some(10.0),
                ..along
            },
            &m8(true),
        );
        assert!(cut.sorted.is_empty() && cut.on_cut.len() == 1);
    }

    #[test]
    fn an_oblique_thread_draws_nothing() {
        let mut marks = ThreadMarks::default();
        let mut thread = m8(false);
        thread.direction = [0.6, 0.8, 0.0];
        marks.add(&front(None), &thread);
        assert!(marks.sorted.is_empty() && marks.on_cut.is_empty());
    }
}
