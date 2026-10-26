//! Round edges back from a drawing's outline. The importer draws arcs and circles as runs
//! of short chords; written as such a run, a circle of a plan is a polygon of a hundred
//! and more sides, more than a profile's sketch solves. A run of corners on one circle,
//! turning one way in steps no longer than the import cuts an arc of its radius into (or
//! [`MAX_STEP`], whichever is longer), is written as arcs through it instead, split at the
//! circle's quarter points so that each arc's ends bound it. The ring is in document
//! millimetres, the drawing's scaled by its placement's scale.

use ketchup_model::underlay::arc_chord_step;
use std::f64::consts::{FRAC_PI_2, TAU};

/// One side of a ring: straight, or an arc from `start` to `end` through `through`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Piece {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    Arc {
        start: [f64; 2],
        end: [f64; 2],
        through: [f64; 2],
    },
}

/// An angle one chord of a drawn arc may always span; a polygon with sides this short is
/// drawn as a curve.
const MAX_STEP: f64 = 10.0 * std::f64::consts::PI / 180.0;
/// The fewest chords a run takes to be an arc.
const MIN_CHORDS: usize = 4;
/// A quarter point this close to an end of an arc does not split it.
const MIN_PIECE: f64 = 0.5 * std::f64::consts::PI / 180.0;

/// The longest chord of a drawn arc of `radius` in a drawing scaled by `scale`: its step as
/// the import cut it, a small circle's in a few long chords (Ø15 in 20), or [`MAX_STEP`].
fn max_chord(radius: f64, scale: f64) -> f64 {
    let step = (arc_chord_step(radius / scale) * (1.0 + 1e-3)).max(MAX_STEP);
    2.0 * radius * (step.min(FRAC_PI_2) / 2.0).sin()
}

/// How far off its circle a corner of an arc may lie.
fn tolerance(radius: f64) -> f64 {
    1e-4 * radius + 1e-3
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The circle through three points, `None` when they lie on a line.
fn circle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let (ab, ac) = (sub(b, a), sub(c, a));
    let d = 2.0 * cross(ab, ac);
    let scale = ab[0].hypot(ab[1]) * ac[0].hypot(ac[1]);
    if d.abs() <= 1e-12 * scale {
        return None;
    }
    let (ab2, ac2) = (ab[0] * ab[0] + ab[1] * ab[1], ac[0] * ac[0] + ac[1] * ac[1]);
    let center = [
        a[0] + (ac[1] * ab2 - ab[1] * ac2) / d,
        a[1] + (ab[0] * ac2 - ac[0] * ab2) / d,
    ];
    Some((center, distance(center, a)))
}

/// The ring's sides, runs of chords along one circle as arcs; `None` when it has none.
/// `scale`: how much larger the ring is than the drawing the import cut.
pub(super) fn pieces(ring: &[[f64; 2]], scale: f64) -> Option<Vec<Piece>> {
    let mut ring = ring.to_vec();
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let n = ring.len();
    if n < MIN_CHORDS + 1 {
        return None;
    }
    let at = |index: usize| ring[index % n];
    // An arc from corner `start` along `chords` chords: its circle, through its first
    // three corners, and its signed sweep.
    let arc_from = |start: usize, limit: usize| -> Option<(usize, [f64; 2], f64, f64)> {
        let (center, radius) = circle(at(start), at(start + 1), at(start + 2))?;
        let turn = cross(
            sub(at(start + 1), at(start)),
            sub(at(start + 2), at(start + 1)),
        );
        let max_chord = max_chord(radius, scale);
        let mut sweep = 0.0;
        let mut chords = 0;
        while chords < limit {
            let (from, to) = (at(start + chords), at(start + chords + 1));
            if (distance(to, center) - radius).abs() > tolerance(radius)
                || distance(from, to) > max_chord
            {
                break;
            }
            if chords > 0 {
                let bend = cross(sub(from, at(start + chords - 1)), sub(to, from));
                if bend * turn <= 0.0 {
                    break;
                }
            }
            sweep += cross(sub(from, center), sub(to, center)).atan2(
                (from[0] - center[0]) * (to[0] - center[0])
                    + (from[1] - center[1]) * (to[1] - center[1]),
            );
            chords += 1;
        }
        (chords >= MIN_CHORDS).then_some((chords, center, radius, sweep))
    };

    // Start at a corner no arc passes through, or take the whole ring as one circle.
    let bends = |index: usize| {
        let (before, here, after) = (at(index + n - 1), at(index), at(index + 1));
        circle(before, here, after).is_some_and(|(_, radius)| {
            distance(before, here).max(distance(here, after)) <= max_chord(radius, scale)
        })
    };
    let mut pieces = Vec::new();
    let first = (0..n).find(|index| !bends(*index));
    if first.is_none()
        && let Some((chords, center, radius, sweep)) = arc_from(0, n)
        && chords == n
        && (sweep.abs() - TAU).abs() <= 1e-6
    {
        split(&mut pieces, at(0), at(0), center, radius, sweep);
        return Some(pieces);
    }
    let first = first.unwrap_or(0);
    let mut found = false;
    let mut done = 0;
    while done < n {
        let start = first + done;
        if let Some((chords, center, radius, sweep)) = arc_from(start, n - done) {
            split(
                &mut pieces,
                at(start),
                at(start + chords),
                center,
                radius,
                sweep,
            );
            done += chords;
            found = true;
        } else {
            pieces.push(Piece::Line {
                start: at(start),
                end: at(start + 1),
            });
            done += 1;
        }
    }
    found.then_some(pieces)
}

/// The arc from `start` to `end` about `center`, sweeping `sweep` radians, as arcs between
/// the quarter points it passes.
fn split(
    pieces: &mut Vec<Piece>,
    start: [f64; 2],
    end: [f64; 2],
    center: [f64; 2],
    radius: f64,
    sweep: f64,
) {
    let from = (start[1] - center[1]).atan2(start[0] - center[0]);
    let point = |angle: f64| {
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    };
    // Angles along the sweep, from 0 at the start, where it passes a quarter point.
    let mut cuts = vec![0.0];
    let direction = sweep.signum();
    let mut quarter = if direction > 0.0 {
        (from / FRAC_PI_2).floor() + 1.0
    } else {
        (from / FRAC_PI_2).ceil() - 1.0
    };
    loop {
        let along = (quarter * FRAC_PI_2 - from) * direction;
        if along >= sweep.abs() - MIN_PIECE {
            break;
        }
        if along >= MIN_PIECE {
            cuts.push(along);
        }
        quarter += direction;
    }
    cuts.push(sweep.abs());
    let ends = cuts
        .iter()
        .enumerate()
        .map(|(index, along)| match index {
            0 => start,
            _ if index == cuts.len() - 1 => end,
            _ => point(from + direction * along),
        })
        .collect::<Vec<_>>();
    for (pair, angles) in ends.windows(2).zip(cuts.windows(2)) {
        pieces.push(Piece::Arc {
            start: pair[0],
            end: pair[1],
            through: point(from + direction * (angles[0] + angles[1]) / 2.0),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(center: [f64; 2], radius: f64, sides: usize, from: f64, to: f64) -> Vec<[f64; 2]> {
        (0..=sides)
            .map(|index| {
                let angle = from + (to - from) * index as f64 / sides as f64;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            })
            .collect()
    }

    fn area(pieces: &[Piece]) -> f64 {
        // Chords plus each arc's circular segment beyond its chord.
        pieces
            .iter()
            .map(|piece| match *piece {
                Piece::Line { start, end } => cross(start, end) / 2.0,
                Piece::Arc {
                    start,
                    end,
                    through,
                } => {
                    // Arcs are at most a quarter turn: the angle is twice the half chord's.
                    let (_, radius) = circle(start, through, end).unwrap();
                    let half = (distance(start, end) / (2.0 * radius)).asin();
                    let bulge = radius * radius * (2.0 * half - (2.0 * half).sin()) / 2.0;
                    let left = cross(sub(through, start), sub(end, through)).signum();
                    cross(start, end) / 2.0 + bulge * left
                }
            })
            .sum()
    }

    #[test]
    fn a_drawn_circle_is_four_arcs_between_its_quarter_points() {
        let mut ring = polygon([6570.0, 5646.0], 400.0, 142, 0.3, 0.3 + TAU);
        ring.pop();
        let pieces = pieces(&ring, 1.0).unwrap();
        assert!(
            pieces
                .iter()
                .all(|piece| matches!(piece, Piece::Arc { .. }))
        );
        assert_eq!(pieces.len(), 5, "{pieces:?}");
        let disc = std::f64::consts::PI * 400.0 * 400.0;
        assert!(
            (area(&pieces) - disc).abs() < 1e-6 * disc,
            "{}",
            area(&pieces)
        );
        // Clockwise as a hole: the same arcs, the other way.
        ring.reverse();
        let hole = super::pieces(&ring, 1.0).unwrap();
        assert!((area(&hole) + disc).abs() < 1e-6 * disc);
    }

    #[test]
    fn a_slot_keeps_its_straight_sides_and_rounds_its_ends() {
        // 200 long, ends of radius 50, each drawn as 36 chords.
        let mut ring = polygon([200.0, 50.0], 50.0, 36, -FRAC_PI_2, FRAC_PI_2);
        ring.extend(polygon([0.0, 50.0], 50.0, 36, FRAC_PI_2, 3.0 * FRAC_PI_2));
        let pieces = pieces(&ring, 1.0).unwrap();
        let lines = pieces
            .iter()
            .filter(|piece| matches!(piece, Piece::Line { .. }))
            .count();
        assert_eq!(lines, 2, "{pieces:?}");
        assert_eq!(
            pieces.len(),
            6,
            "two sides and two half circles of two arcs each"
        );
        let slot = 200.0 * 100.0 + std::f64::consts::PI * 50.0 * 50.0;
        assert!((area(&pieces) - slot).abs() < 1e-6 * slot);
        for pair in pieces.windows(2) {
            let (Piece::Line { end, .. } | Piece::Arc { end, .. }) = pair[0];
            let (Piece::Line { start, .. } | Piece::Arc { start, .. }) = pair[1];
            assert_eq!(end, start);
        }
    }

    /// A hole Ø15 on a sheet: the import cuts it in 20 chords of 18°. Read at the
    /// drawing's size or at twice it, it is a circle; a 20-gon Ø30 drawn at full size, whose
    /// sides the import would have cut finer, stays one.
    #[test]
    fn a_small_circle_in_few_chords_is_round_at_the_drawings_scale() {
        let mut small = polygon([0.0, 0.0], 7.5, 20, 0.0, TAU);
        small.pop();
        let disc = std::f64::consts::PI * 7.5 * 7.5;
        let read = pieces(&small, 1.0).unwrap();
        assert!((area(&read) - disc).abs() < 1e-9 * disc);
        let doubled = small
            .iter()
            .map(|[x, y]| [2.0 * x, 2.0 * y])
            .collect::<Vec<_>>();
        let read = pieces(&doubled, 2.0).unwrap();
        assert!((area(&read) - 4.0 * disc).abs() < 1e-9 * disc);
        assert_eq!(pieces(&doubled, 1.0), None);
    }

    #[test]
    fn polygons_drawn_as_such_stay_polygons() {
        let mut octagon = polygon([0.0, 0.0], 100.0, 8, 0.0, TAU);
        octagon.pop();
        assert_eq!(pieces(&octagon, 1.0), None);
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert_eq!(pieces(&square, 1.0), None);
        // A wave of short chords turning both ways is no arc.
        let wave = (0..40)
            .map(|index| [f64::from(index), if index % 2 == 0 { 0.0 } else { 0.01 }])
            .chain([[39.0, -50.0], [0.0, -50.0]])
            .collect::<Vec<_>>();
        assert_eq!(pieces(&wave, 1.0), None);
    }
}
