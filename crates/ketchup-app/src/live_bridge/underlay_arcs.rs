//! Round edges back from a drawing's outline. The importer draws arcs and circles as runs
//! of short chords; written as such a run, a circle of a plan is a polygon of a hundred
//! and more sides, more than a profile's sketch solves. A run of corners on one circle,
//! turning one way in steps no longer than the import cuts an arc of its radius into (or
//! [`MAX_STEP`], whichever is longer), is written as arcs through it instead, split at the
//! circle's quarter points so that each arc's ends bound it. The ring is in document
//! millimetres, the drawing's scaled by its placement's scale.

use ketchup_geometry::linalg::{cross2 as cross, dot2, sub2 as sub};
use ketchup_model::tolerance::{
    APPROXIMATION, CONTACT_FIT_MM, NEGLIGIBLE, UNDERLAY_ARC_OFF_CIRCLE,
    UNDERLAY_ARC_OFF_CIRCLE_MAX_MM, UNDERLAY_ARC_OFF_CIRCLE_MM, UNDERLAY_ARC_STEP_SLACK,
};
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
/// Nor does one that leaves a piece whose middle lies less than this off its chord: twice
/// what the program takes for a line, so the piece written to the thousandth of a
/// millimetre is still an arc (review 2026-10-29 UO-62).
const MIN_BULGE: f64 = CONTACT_FIT_MM;

/// The longest chord of a drawn arc of `radius` in a drawing scaled by `scale`: its step as
/// the import cut it, a small circle's in a few long chords (Ø15 in 20), or [`MAX_STEP`].
fn max_chord(radius: f64, scale: f64) -> f64 {
    let step = (arc_chord_step(radius / scale) * (1.0 + UNDERLAY_ARC_STEP_SLACK)).max(MAX_STEP);
    2.0 * radius * (step.min(FRAC_PI_2) / 2.0).sin()
}

/// How far off its circle a corner of an arc may lie: with the radius, but never more than
/// a tenth of the import's chord error, so a nearly straight wall is no huge circle that
/// other corners lie on (review 2026-10-29 UO-60).
fn tolerance(radius: f64, scale: f64) -> f64 {
    (UNDERLAY_ARC_OFF_CIRCLE * radius).min(UNDERLAY_ARC_OFF_CIRCLE_MAX_MM * scale)
        + UNDERLAY_ARC_OFF_CIRCLE_MM
}

/// The angle of an arc of `radius` whose middle lies [`MIN_BULGE`] off its chord.
fn min_piece(radius: f64) -> f64 {
    (2.0 * (1.0 - MIN_BULGE / radius).max(-1.0).acos()).max(MIN_PIECE)
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The circle through three points, `None` when they lie on a line.
fn circle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let (ab, ac) = (sub(b, a), sub(c, a));
    let d = 2.0 * cross(ab, ac);
    let scale = ab[0].hypot(ab[1]) * ac[0].hypot(ac[1]);
    if d.abs() <= NEGLIGIBLE * scale {
        return None;
    }
    let (ab2, ac2) = (dot2(ab, ab), dot2(ac, ac));
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
    // How many of the `limit` chords from corner `start` lie on the circle `center`,
    // `radius`, each no longer than the import cuts it and all turning one way.
    let along = |start: usize, limit: usize, center: [f64; 2], radius: f64| -> usize {
        let turn = cross(
            sub(at(start + 1), at(start)),
            sub(at(start + 2), at(start + 1)),
        );
        let (max_chord, tolerance) = (max_chord(radius, scale), tolerance(radius, scale));
        let on = |index: usize| (distance(at(index), center) - radius).abs() <= tolerance;
        if !on(start) {
            return 0;
        }
        let mut chords = 0;
        while chords < limit {
            let (from, to) = (at(start + chords), at(start + chords + 1));
            if !on(start + chords + 1) || distance(from, to) > max_chord {
                break;
            }
            if chords > 0 {
                let bend = cross(sub(from, at(start + chords - 1)), sub(to, from));
                if bend * turn <= 0.0 {
                    break;
                }
            }
            chords += 1;
        }
        chords
    };
    // An arc from corner `start`: its chords, circle and signed sweep. The run is found
    // along the circle through its first three corners; its circle is then the one through
    // its ends and middle, which every corner of the run must lie on (review 2026-10-29
    // UO-60), or the run is shortened until they do.
    let arc_from = |start: usize, limit: usize| -> Option<(usize, [f64; 2], f64, f64)> {
        let (center, radius) = circle(at(start), at(start + 1), at(start + 2))?;
        let mut chords = along(start, limit, center, radius);
        while chords >= MIN_CHORDS {
            let middle = if chords == n { chords / 3 } else { chords / 2 };
            let last = if chords == n { 2 * chords / 3 } else { chords };
            if let Some((center, radius)) = circle(at(start), at(start + middle), at(start + last))
                && along(start, chords, center, radius) == chords
            {
                let sweep = (0..chords)
                    .map(|chord| {
                        let (from, to) = (
                            sub(at(start + chord), center),
                            sub(at(start + chord + 1), center),
                        );
                        cross(from, to).atan2(dot2(from, to))
                    })
                    .sum();
                return Some((chords, center, radius, sweep));
            }
            chords -= 1;
        }
        None
    };

    // Start at a corner no arc passes through, or take the whole ring as one circle. The
    // import cuts an arc in equal chords: where a long wall meets an arc, the nearly
    // straight circle through the corner is none (review 2026-10-29 UO-60).
    let bends = |index: usize| {
        let (before, here, after) = (at(index + n - 1), at(index), at(index + 1));
        let (into, out) = (distance(before, here), distance(here, after));
        (into - out).abs() <= UNDERLAY_ARC_STEP_SLACK * into.max(out) + 2.0 * tolerance(0.0, scale)
            && circle(before, here, after)
                .is_some_and(|(_, radius)| into.max(out) <= max_chord(radius, scale))
    };
    let mut pieces = Vec::new();
    let first = (0..n).find(|index| !bends(*index));
    if first.is_none()
        && let Some((chords, center, radius, sweep)) = arc_from(0, n)
        && chords == n
        && (sweep.abs() - TAU).abs() <= APPROXIMATION
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
    let min_piece = min_piece(radius);
    loop {
        let along = (quarter * FRAC_PI_2 - from) * direction;
        if along >= sweep.abs() - min_piece {
            break;
        }
        if along >= min_piece {
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

    /// An arc as the import cuts it: chords of [`arc_chord_step`], the end included.
    fn drawn_arc(center: [f64; 2], radius: f64, from: f64, to: f64) -> Vec<[f64; 2]> {
        let sides = ((to - from).abs() / arc_chord_step(radius)).ceil() as usize;
        polygon(center, radius, sides, from, to)
    }

    /// `ring` started at each of its corners in turn.
    fn starts(ring: &[[f64; 2]]) -> impl Iterator<Item = Vec<[f64; 2]>> + '_ {
        (0..ring.len()).map(|start| {
            ring[start..]
                .iter()
                .chain(&ring[..start])
                .copied()
                .collect()
        })
    }

    fn polygon_area(ring: &[[f64; 2]]) -> f64 {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(a, b)| cross(*a, *b) / 2.0)
            .sum()
    }

    /// From whichever corner it starts, `ring` is read as arcs and lines bounding `exact`.
    fn assert_round(ring: &[[f64; 2]], exact: f64) {
        for ring in starts(ring) {
            let pieces = pieces(&ring, 1.0).unwrap_or_else(|| panic!("arcs in {ring:?}"));
            let area = area(&pieces);
            assert!(
                (area - exact).abs() <= 1e-6 * exact,
                "{area} for {exact} from {:?}: {pieces:?}",
                ring[0]
            );
        }
    }

    /// Review 2026-10-29 UO-60, UO-61: a room with a corner in the middle of a wall, nudged
    /// out by up to 0.01 mm, keeps its straight sides from whichever corner it starts; the
    /// big circle through the nudged corner swallowed the room's corners.
    #[test]
    fn a_room_with_a_corner_nudged_off_a_wall_keeps_its_straight_sides() {
        let rooms: [&[[f64; 2]]; 3] = [
            // An L of 21 m².
            &[
                [0.0, 0.0],
                [6000.0, 0.0],
                [6000.0, 2000.0],
                [3000.0, 2000.0],
                [3000.0, 5000.0],
                [0.0, 5000.0],
            ],
            &[[0.0, 0.0], [4000.0, 0.0], [4000.0, 3000.0], [0.0, 3000.0]],
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        ];
        for room in rooms {
            for wall in 0..room.len() {
                let (a, b) = (room[wall], room[(wall + 1) % room.len()]);
                let along = sub(b, a);
                let length = along[0].hypot(along[1]);
                for nudge in [1e-7, 1e-3, 0.01] {
                    // Out of the counterclockwise room: to the right of the wall.
                    let corner = [
                        (a[0] + b[0]) / 2.0 + along[1] / length * nudge,
                        (a[1] + b[1]) / 2.0 - along[0] / length * nudge,
                    ];
                    let mut ring = room.to_vec();
                    ring.insert(wall + 1, corner);
                    for ring in starts(&ring) {
                        assert_eq!(pieces(&ring, 1.0), None, "{ring:?}");
                    }
                }
            }
        }
    }

    /// Review 2026-10-29 UO-60: round edges drawn as the import cuts them stay where they
    /// were and leave the walls beside them straight, from whichever corner the ring starts.
    #[test]
    fn round_edges_beside_long_walls_keep_the_area() {
        use std::f64::consts::PI;
        // A room of 5 × 4 m with one corner R1000.
        let mut room = vec![[0.0, 0.0], [5000.0, 0.0]];
        room.extend(drawn_arc([4000.0, 3000.0], 1000.0, 0.0, FRAC_PI_2));
        room.push([0.0, 4000.0]);
        assert_round(&room, 20.0e6 - (1.0 - PI / 4.0) * 1.0e6);
        // A room of 4 × 3 m with an apse R2000 on its long side.
        let mut apse = vec![[0.0, 0.0], [4000.0, 0.0]];
        apse.extend(drawn_arc([2000.0, 3000.0], 2000.0, 0.0, PI));
        assert_round(&apse, 12.0e6 + PI * 2.0e6);
        // A slot 3000 long with ends R50.
        let mut slot = drawn_arc([3000.0, 50.0], 50.0, -FRAC_PI_2, FRAC_PI_2);
        slot.extend(drawn_arc([0.0, 50.0], 50.0, FRAC_PI_2, 3.0 * FRAC_PI_2));
        assert_round(&slot, 3000.0 * 100.0 + PI * 2500.0);
    }

    /// Review 2026-10-29 UO-60: a slot 1000 long whose ends are drawn in chords of 10°
    /// bounds no more than its round ends would, from whichever corner it starts.
    #[test]
    fn a_slot_in_chords_of_ten_degrees_bounds_no_more_than_its_round_ends() {
        let mut slot = polygon([1000.0, 50.0], 50.0, 18, -FRAC_PI_2, FRAC_PI_2);
        slot.extend(polygon([0.0, 50.0], 50.0, 18, FRAC_PI_2, 3.0 * FRAC_PI_2));
        let round = 1000.0 * 100.0 + std::f64::consts::PI * 2500.0;
        let drawn = polygon_area(&slot);
        for ring in starts(&slot) {
            let area = pieces(&ring, 1.0).map_or(drawn, |pieces| area(&pieces));
            assert!(
                drawn * (1.0 - 1e-9) <= area && area <= round * (1.0 + 1e-9),
                "{area} of {drawn} to {round} from {:?}",
                ring[0]
            );
        }
    }

    /// Review 2026-10-29 UO-62: every arc of a hole, turned any way, is one the program
    /// takes as it is written, to the thousandth of a millimetre: its through point clear of
    /// the line between its ends.
    #[test]
    fn every_arc_of_a_turned_hole_is_one_the_program_takes() {
        use ketchup_model::tolerance::CONTACT_FIT_MM;
        let written = |point: [f64; 2]| point.map(|value| (value * 1000.0).round() / 1000.0);
        for radius in [2.0, 7.5, 50.0] {
            for turn in 0..500 {
                let from = f64::from(turn) * 0.0123;
                let mut hole = drawn_arc([123.0, 45.0], radius, from, from + TAU);
                hole.pop();
                for piece in pieces(&hole, 1.0).unwrap() {
                    let Piece::Arc {
                        start,
                        end,
                        through,
                    } = piece
                    else {
                        panic!("a line in a hole r = {radius}");
                    };
                    let [start, end, through] = [start, end, through].map(written);
                    let half = distance(start, end) / 2.0;
                    let bulge = cross(sub(through, start), sub(end, through));
                    assert!(
                        half > CONTACT_FIT_MM && bulge.abs() > CONTACT_FIT_MM * half,
                        "r = {radius} turned {from}: {piece:?}"
                    );
                }
            }
        }
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
