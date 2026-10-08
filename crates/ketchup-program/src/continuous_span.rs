//! A straight member on point supports with constant stiffness, continuous
//! over the inner supports, with free overhangs at both ends: support moments
//! by the three-moment equation, then the reactions. Positions run along the
//! member; forces act across it. Shared by the load transfer and the member check.

use ketchup_tolerance::ROUNDING;

/// A force spread evenly over [a, b] (a == b: a point load), newtons.
pub(crate) type Piece = (f64, f64, f64);

/// Point loads closer than this to a support act on it directly, mm.
const AT_SUPPORT_MM: f64 = 0.5;

/// Length of a bedded span in the moment equations, relative to the longest
/// span: short enough to clamp its neighbours, long enough to keep the
/// system solvable.
// not a tolerance: a stiffness ratio.
const BEDDED_LENGTH: f64 = 1e-9;

/// Support moments (hogging negative) and support reactions (upward positive,
/// negative: uplift).
#[derive(Debug, PartialEq)]
pub(crate) struct Solution {
    pub moments: Vec<f64>,
    pub reactions: Vec<f64>,
}

fn is_point(piece: &Piece) -> bool {
    piece.1 - piece.0 <= ROUNDING
}

/// The part of `pieces` strictly between `from` and `to`; point loads at the
/// ends are left out (they act on the supports).
fn within(pieces: &[Piece], from: f64, to: f64) -> Vec<Piece> {
    pieces
        .iter()
        .filter_map(|&(a, b, f)| {
            if is_point(&(a, b, f)) {
                (a > from + AT_SUPPORT_MM && a < to - AT_SUPPORT_MM).then_some((a, a, f))
            } else {
                let (lo, hi) = (a.max(from), b.min(to));
                (hi > lo).then(|| (lo, hi, f * (hi - lo) / (b - a)))
            }
        })
        .collect()
}

/// Mean of a cubic over [a, b] times f (Simpson's rule is exact for it).
fn spread(piece: &Piece, cubic: impl Fn(f64) -> f64) -> f64 {
    let (a, b, f) = *piece;
    f * (cubic(a) + 4.0 * cubic(f64::midpoint(a, b)) + cubic(b)) / 6.0
}

/// Force of the pieces and their moment about `at` (positive for loads on
/// either side pulling that side down).
fn moment_about(pieces: &[Piece], at: f64) -> (f64, f64) {
    pieces.iter().fold((0.0, 0.0), |(force, moment), piece| {
        (
            force + piece.2,
            moment + piece.2 * (f64::midpoint(piece.0, piece.1) - at).abs(),
        )
    })
}

/// Supports at `supports` (ascending, at least one), loads `pieces` anywhere
/// along the member. A span marked in `bedded` (one flag per span, or none)
/// lies on its support along its whole length: it stays straight, so it
/// clamps the spans next to it.
pub(crate) fn solve(supports: &[f64], bedded: &[bool], pieces: &[Piece]) -> Solution {
    let n = supports.len();
    let (first, last) = (supports[0], supports[n - 1]);
    let mut reactions = vec![0.0; n];
    // Point loads on a support go straight into it.
    for &(a, _, f) in pieces.iter().filter(|piece| is_point(piece)) {
        if let Some(k) = supports.iter().position(|s| (a - s).abs() <= AT_SUPPORT_MM) {
            reactions[k] += f;
        }
    }
    let left = within(pieces, f64::NEG_INFINITY, first);
    let right = within(pieces, last, f64::INFINITY);
    let (left_force, left_moment) = moment_about(&left, first);
    let (right_force, right_moment) = moment_about(&right, last);
    if n == 1 {
        reactions[0] += left_force + right_force;
        // One support holds nothing in balance; it takes everything.
        return Solution {
            moments: vec![0.0],
            reactions,
        };
    }
    reactions[0] += left_force;
    reactions[n - 1] += right_force;
    let spans: Vec<(f64, Vec<Piece>)> = supports
        .windows(2)
        .map(|pair| {
            let local = within(pieces, pair[0], pair[1])
                .into_iter()
                .map(|(a, b, f)| (a - pair[0], b - pair[0], f))
                .collect();
            (pair[1] - pair[0], local)
        })
        .collect();
    // EI times the end rotations of each span simply supported, times its length.
    let rotations: Vec<(f64, f64)> = spans
        .iter()
        .map(|(l, local)| {
            let l = *l;
            local.iter().fold((0.0, 0.0), |(left, right), piece| {
                (
                    left + spread(piece, |x| x * (l - x) * (2.0 * l - x) / 6.0),
                    right + spread(piece, |x| x * (l - x) * (l + x) / 6.0),
                )
            })
        })
        .collect();
    let mut moments = vec![0.0; n];
    moments[0] = -left_moment;
    moments[n - 1] = -right_moment;
    // M[i-1] L[i] + 2 M[i] (L[i] + L[i+1]) + M[i+1] L[i+1]
    //   = -6 (rotation at the right end of span i + at the left end of span i+1).
    // A bedded span takes part as an infinitely stiff one: no length, no rotation.
    let longest = spans.iter().map(|span| span.0).fold(0.0, f64::max);
    let is_bedded = |j: usize| bedded.get(j).copied().unwrap_or(false);
    let flexible = |j: usize| -> (f64, f64, f64) {
        if is_bedded(j) {
            (longest * BEDDED_LENGTH, 0.0, 0.0)
        } else {
            let l = spans[j].0;
            (l, rotations[j].0 / l, rotations[j].1 / l)
        }
    };
    // A bed only rests on its support: at its edge it can hold the member
    // down against hogging, not against sagging. A sagging edge lifts and
    // turns into a pivot.
    let unknown = n - 2;
    let mut pivoted = vec![false; unknown];
    // Each round pivots one more edge, so it ends.
    let rounds = if unknown > 0 { unknown + 1 } else { 0 };
    for _ in 0..rounds {
        let mut lower = vec![0.0; unknown];
        let mut diagonal = vec![0.0; unknown];
        let mut upper = vec![0.0; unknown];
        let mut rhs = vec![0.0; unknown];
        for row in 0..unknown {
            if pivoted[row] {
                diagonal[row] = 1.0;
                continue;
            }
            let (l1, _, right_end) = flexible(row);
            let (l2, left_end, _) = flexible(row + 1);
            lower[row] = l1;
            diagonal[row] = 2.0 * (l1 + l2);
            upper[row] = l2;
            rhs[row] = -6.0 * (right_end + left_end);
        }
        rhs[0] -= lower[0] * moments[0];
        rhs[unknown - 1] -= upper[unknown - 1] * moments[n - 1];
        // Thomas algorithm: the system is diagonally dominant.
        for row in 1..unknown {
            let w = lower[row] / diagonal[row - 1];
            diagonal[row] -= w * upper[row - 1];
            rhs[row] -= w * rhs[row - 1];
        }
        moments[unknown] = rhs[unknown - 1] / diagonal[unknown - 1];
        for row in (0..unknown - 1).rev() {
            moments[row + 1] = (rhs[row] - upper[row] * moments[row + 2]) / diagonal[row];
        }
        let lifting = (0..unknown).find(|&row| {
            !pivoted[row] && (is_bedded(row) || is_bedded(row + 1)) && moments[row + 1] > ROUNDING
        });
        let Some(row) = lifting else {
            break;
        };
        pivoted[row] = true;
    }
    for (j, (l, local)) in spans.iter().enumerate() {
        let simple_left: f64 = local
            .iter()
            .map(|&(a, b, f)| f * (l - f64::midpoint(a, b)) / l)
            .sum();
        let total: f64 = local.iter().map(|piece| piece.2).sum();
        let correction = (moments[j + 1] - moments[j]) / l;
        reactions[j] += simple_left + correction;
        reactions[j + 1] += total - simple_left - correction;
    }
    Solution { moments, reactions }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-6 * expected.abs().max(1.0),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn two_equal_continuous_spans_put_five_quarters_on_the_middle_support() {
        let (w, l) = (2.0, 3000.0);
        let solution = solve(&[0.0, l, 2.0 * l], &[], &[(0.0, 2.0 * l, w * 2.0 * l)]);
        close(solution.moments[1], -w * l * l / 8.0);
        close(solution.reactions[0], 0.375 * w * l);
        close(solution.reactions[1], 1.25 * w * l);
        close(solution.reactions[2], 0.375 * w * l);
    }

    #[test]
    fn a_load_on_an_overhang_lifts_the_far_support() {
        let p = 1000.0;
        let solution = solve(&[0.0, 3000.0], &[], &[(4500.0, 4500.0, p)]);
        close(solution.moments[1], -1500.0 * p);
        close(solution.reactions[0], -0.5 * p);
        close(solution.reactions[1], 1.5 * p);
    }

    #[test]
    fn three_unequal_spans_balance_the_load_and_match_the_hand_calculation() {
        // Spans 2000, 3000, 2500 under 1 N/mm; three-moment equations solved by hand:
        // 10000 M1 + 3000 M2 = -6 (2000³/24 + 3000³/24) and
        // 3000 M1 + 11000 M2 = -6 (3000³/24 + 2500³/24).
        let supports = [0.0, 2000.0, 5000.0, 7500.0];
        let solution = solve(&supports, &[], &[(0.0, 7500.0, 7500.0)]);
        let (a, b, c, d) = (10000.0, 3000.0, 3000.0, 11000.0);
        let r1 = -6.0 * (2000f64.powi(3) + 3000f64.powi(3)) / 24.0;
        let r2 = -6.0 * (3000f64.powi(3) + 2500f64.powi(3)) / 24.0;
        let det = a * d - b * c;
        close(solution.moments[1], (r1 * d - b * r2) / det);
        close(solution.moments[2], (a * r2 - c * r1) / det);
        close(solution.reactions.iter().sum::<f64>(), 7500.0);
    }

    #[test]
    fn a_long_bed_clamps_the_span_next_to_it_instead_of_spanning_itself() {
        // A 6 m bed, then a free 3 m span to a pin: the span is a propped
        // cantilever, M at the bed end = -w L² / 8, and a short gap between
        // two beds stays nearly unloaded instead of carrying the bed's moment.
        let (w, l) = (2.0, 3000.0);
        let solution = solve(
            &[0.0, 6000.0, 9000.0],
            &[true, false],
            &[(0.0, 9000.0, w * 9000.0)],
        );
        close(solution.moments[1], -w * l * l / 8.0);
        let gap = solve(
            &[0.0, 6000.0, 6090.0, 12000.0],
            &[true, false, true],
            &[(0.0, 12000.0, w * 12000.0)],
        );
        assert!(gap.moments[1].abs() < w * 90.0 * 90.0, "{:?}", gap.moments);
        assert!(gap.moments[2].abs() < w * 90.0 * 90.0, "{:?}", gap.moments);
    }

    #[test]
    fn a_bed_edge_that_would_have_to_hold_sagging_lets_go_and_acts_as_a_pivot() {
        // A load on the overhang left of a pin: clamped, the bed edge would
        // carry +M/2 (sagging), which a resting bed cannot.
        let p = 1000.0;
        let solution = solve(&[500.0, 1000.0, 5000.0], &[false, true], &[(0.0, 0.0, p)]);
        close(solution.moments[0], -500.0 * p);
        close(solution.moments[1], 0.0);
        close(solution.reactions[0], 2.0 * p);
    }

    #[test]
    fn a_point_load_on_a_support_goes_into_it() {
        let solution = solve(&[0.0, 1000.0, 2000.0], &[], &[(1000.0, 1000.0, 500.0)]);
        assert_eq!(solution.reactions, vec![0.0, 500.0, 0.0]);
    }
}
