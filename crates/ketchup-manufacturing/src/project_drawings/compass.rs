//! The compass of the project sheets (review 2026-11-08 §5.2, roadmap
//! WP-1.6): the north arrow on each plan and the side of the world each
//! elevation looks at, which names it ("South elevation"). Both follow the
//! direction of north the drawing settings keep; without it a plan draws no
//! arrow and an elevation keeps the name of the view.

use super::{Point, ViewFrame};
use ketchup_geometry::linalg::{dot, dot2};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, Stroke};
use ketchup_model::tolerance::NEGLIGIBLE;
use serde::Serialize;

/// The eight sides of the world, clockwise from north.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompassSide {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl CompassSide {
    /// Clockwise from north, 45° apart.
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
    ];
}

/// North in the plan of the model: +y turned counter-clockwise by `north_deg`.
fn north(north_deg: f64) -> [f64; 2] {
    let angle = north_deg.to_radians();
    [-angle.sin(), angle.cos()]
}

/// The side of the world `facing`, a direction in the model, points to: the
/// nearest of the eight, none when it has no horizontal part.
#[must_use]
pub fn facing_side(facing: Point, north_deg: f64) -> Option<CompassSide> {
    let toward_north = north(north_deg);
    let toward_east = [toward_north[1], -toward_north[0]];
    let horizontal = [facing[0], facing[1]];
    let (east, north) = (
        dot2(horizontal, toward_east),
        dot2(horizontal, toward_north),
    );
    if east.hypot(north) <= NEGLIGIBLE {
        return None;
    }
    let bearing = east.atan2(north).to_degrees();
    // How far the bearing lies from side `index`, 45° apart, either way round.
    let off = |index: u32| {
        let apart = (bearing - 45.0 * f64::from(index)).rem_euclid(360.0);
        apart.min(360.0 - apart)
    };
    CompassSide::ALL
        .into_iter()
        .zip(0_u32..)
        .min_by(|left, right| off(left.1).total_cmp(&off(right.1)))
        .map(|(side, _)| side)
}

/// The side an elevation seen along `frame` shows: the faces turned toward
/// the viewer, so against the direction of view.
pub(super) fn elevation_side(frame: &ViewFrame, north_deg: f64) -> Option<CompassSide> {
    facing_side(frame.depth.map(|value| -value), north_deg)
}

/// The north arrow of a plan: which way north points on the sheet, in view
/// millimetres (y up), and the letter written at its tip.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NorthArrow {
    pub direction: [f64; 2],
    pub letter: String,
}

impl NorthArrow {
    /// The arrow of a plan seen along `frame` with north at `north_deg`; none
    /// when north does not lie in the plane of the view.
    pub(super) fn of(frame: &ViewFrame, north_deg: f64, letter: &str) -> Option<Self> {
        let [x, y] = north(north_deg);
        let toward = [x, y, 0.0];
        let direction = [dot(toward, frame.right), dot(toward, frame.up)];
        let length = direction[0].hypot(direction[1]);
        (length > NEGLIGIBLE).then(|| Self {
            direction: direction.map(|value| value / length),
            letter: if letter.trim().is_empty() {
                "N".to_owned()
            } else {
                letter.trim().to_owned()
            },
        })
    }
}

/// The radius of the arrow's circle, in page millimetres.
const ARROW_RADIUS_MM: f64 = 6.0;
const ARROW_LETTER_MM: f64 = 3.5;
// not a tolerance: the pen width of the arrow's circle and outline.
const ARROW_STROKE_MM: f64 = 0.25;
/// Segments of the arrow's circle.
const CIRCLE_SEGMENTS: u32 = 48;

/// Draws `arrow` centred at `centre`, in page millimetres (y down): a
/// circle, a kite pointing north with its left half filled, and the letter
/// past its tip.
pub(super) fn draw_north_arrow(page: &mut Page, centre: [f64; 2], arrow: &NorthArrow) {
    // On the page y runs down.
    let toward = [arrow.direction[0], -arrow.direction[1]];
    let across = [-toward[1], toward[0]];
    let point = |forward: f64, side: f64| {
        [
            centre[0] + toward[0] * forward + across[0] * side,
            centre[1] + toward[1] * forward + across[1] * side,
        ]
    };
    let stroke = Stroke::solid(ARROW_STROKE_MM);
    let circle = (0..CIRCLE_SEGMENTS)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / f64::from(CIRCLE_SEGMENTS);
            [
                centre[0] + ARROW_RADIUS_MM * angle.cos(),
                centre[1] + ARROW_RADIUS_MM * angle.sin(),
            ]
        })
        .collect();
    page.marks.push(Mark::Line {
        points: circle,
        closed: true,
        stroke: stroke.clone(),
    });
    let (tip, tail) = (
        point(ARROW_RADIUS_MM, 0.0),
        point(-ARROW_RADIUS_MM * 0.7, 0.0),
    );
    let half_width = ARROW_RADIUS_MM * 0.4;
    page.marks.push(Mark::Fill {
        loops: vec![vec![tip, point(-ARROW_RADIUS_MM * 0.3, -half_width), tail]],
        color: [0, 0, 0],
        outline: None,
    });
    page.marks.push(Mark::Line {
        points: vec![
            tip,
            point(-ARROW_RADIUS_MM * 0.3, -half_width),
            tail,
            point(-ARROW_RADIUS_MM * 0.3, half_width),
        ],
        closed: true,
        stroke,
    });
    let [x, y] = point(ARROW_RADIUS_MM + ARROW_LETTER_MM * 0.9, 0.0);
    // The text sits on its baseline: lower it by about a third of its height
    // so the letter is centred where it is put.
    page.text(
        [x, y + ARROW_LETTER_MM / 3.0],
        &arrow.letter,
        ARROW_LETTER_MM,
        Anchor::Middle,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direction_is_named_by_the_nearest_of_eight_sides() {
        // North along +y.
        assert_eq!(facing_side([0.0, -1.0, 0.0], 0.0), Some(CompassSide::South));
        assert_eq!(facing_side([1.0, 0.0, 0.0], 0.0), Some(CompassSide::East));
        assert_eq!(
            facing_side([1.0, 1.0, 0.0], 0.0),
            Some(CompassSide::NorthEast)
        );
        assert_eq!(facing_side([-1.0, 0.1, 0.0], 0.0), Some(CompassSide::West));
        // North turned 90° counter-clockwise, along -x: +y now points east.
        assert_eq!(facing_side([0.0, 1.0, 0.0], 90.0), Some(CompassSide::East));
        assert_eq!(facing_side([0.0, -1.0, 0.0], 90.0), Some(CompassSide::West));
        // North turned 20°: -y lies 20° west of south, still nearest south;
        // turned 65°, it lies nearest south-west.
        assert_eq!(
            facing_side([0.0, -1.0, 0.0], 20.0),
            Some(CompassSide::South)
        );
        assert_eq!(
            facing_side([0.0, -1.0, 0.0], 65.0),
            Some(CompassSide::SouthWest)
        );
        // Straight up faces no side.
        assert_eq!(facing_side([0.0, 0.0, 1.0], 0.0), None);
    }

    #[test]
    fn the_arrow_points_north_on_the_page_whatever_the_plan_turns() {
        let plan = ViewFrame {
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            depth: [0.0, 0.0, -1.0],
            cut: None,
        };
        let arrow = NorthArrow::of(&plan, 90.0, "S").unwrap();
        assert!((arrow.direction[0] + 1.0).abs() < 1e-12, "{arrow:?}");
        assert_eq!(arrow.letter, "S");
        // A plan that runs along y has +y to the right: north (+y) points right.
        let turned = ViewFrame {
            right: [0.0, 1.0, 0.0],
            up: [-1.0, 0.0, 0.0],
            ..plan
        };
        let arrow = NorthArrow::of(&turned, 0.0, "").unwrap();
        assert!((arrow.direction[0] - 1.0).abs() < 1e-12, "{arrow:?}");
        assert_eq!(arrow.letter, "N");

        let mut page = Page::default();
        draw_north_arrow(&mut page, [100.0, 100.0], &arrow);
        let letter = page
            .marks
            .iter()
            .find_map(|mark| match mark {
                Mark::Text { at, text, .. } => Some((*at, text.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(letter.1, "N");
        assert!(
            letter.0[0] > 100.0 + ARROW_RADIUS_MM,
            "past the tip: {letter:?}"
        );
    }
}
