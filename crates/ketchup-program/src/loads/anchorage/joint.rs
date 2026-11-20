//! How a joint holds, one model for every path to the ground: from an anchor
//! down to the floor ([`super::Footing`]'s search) and for a brace from the
//! member to its anchor alike (review 2026-11-20 R1120ac-PG-900: a hanger on
//! one screw to its anchor was taken as fixed there and held the member, though
//! the search took the same joint for one the parts turn about). A joint
//! either carries the moment a force sideways puts on it, or the parts turn
//! about it freely; a brace turning about a joint at either end is a strut
//! pushing only along its length (R1120ac-PG-903: one pinned 45° across the
//! force was four times too stiff and twice too strong).

use super::stiffness::{SLIP_PER_FASTENER, Tie, euler_load, moment, section};
use crate::frame::Obb;
use ketchup_geometry::linalg::{dot, length, sub};
use ketchup_model::tolerance::ALONG_AXIS_NEGLIGIBLE;

/// How a joint holds a unit force sideways at the point a path holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Hold {
    /// It carries the moment, giving this much, mm per N.
    Fixed(f64),
    /// The parts turn about it freely: it passes on only a force through it,
    /// slipping this much along it, mm per N.
    Turning(f64),
}

impl Hold {
    /// What the joint gives, mm per N.
    pub(super) fn give(self) -> f64 {
        match self {
            Self::Fixed(give) | Self::Turning(give) => give,
        }
    }
}

/// How a joint `tie` at `at` holds a unit force `sideways` at `held`: the parts
/// turn about it freely under a moment beyond the thinner part's thickness
/// `thin` there, which the parts bearing on each other carry no further
/// (R1113r95-PG-702: a free post on one screw at its foot held the member).
pub(super) fn hold(tie: Tie, at: [f64; 3], held: [f64; 3], sideways: [f64; 3], thin: f64) -> Hold {
    let lever = length(moment(held, at, sideways));
    let slip = |count: f64| 1.0 / (count * SLIP_PER_FASTENER);
    let short = lever <= thin;
    match tie {
        Tie::Rigid => Hold::Fixed(0.0),
        Tie::Pin if short => Hold::Fixed(slip(1.0)),
        Tie::Pin => Hold::Turning(slip(1.0)),
        // Fasteners in one spot turn like one.
        Tie::Fasteners { count, turning } if turning < thin * thin / 4.0 => {
            if short {
                Hold::Fixed(slip(count))
            } else {
                Hold::Turning(slip(count))
            }
        }
        Tie::Fasteners { count, turning } => {
            Hold::Fixed(slip(count) + lever * lever / (SLIP_PER_FASTENER * turning))
        }
    }
}

/// A brace turning about a joint at either end, from the point of the member
/// it holds to its anchor: it pushes and pulls only along its axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Strut {
    /// The cosine between its axis and the force.
    cosine: f64,
    /// How far it stretches under a unit force along it, mm per N.
    pub(super) give: f64,
    /// Euler's load along it, N.
    pub(super) capacity: f64,
}

impl Strut {
    /// The strut of box `obb`, modulus `modulus` and fifth-percentile modulus
    /// `modulus_05` (N/mm²) between `entry` and `exit`, buckling over the
    /// length `reach`, held by a force `sideways`.
    pub(super) fn of(
        obb: &Obb,
        (modulus, modulus_05): (f64, f64),
        (entry, exit): ([f64; 3], [f64; 3]),
        sideways: [f64; 3],
        reach: f64,
    ) -> Self {
        let run = sub(entry, exit);
        let (along, [(_, wide_i), (_, wide_j)]) = section(obb, run);
        Self {
            cosine: dot(sideways, obb.axes[along]).abs(),
            give: length(run) / (modulus * wide_i * wide_j),
            capacity: euler_load(obb, modulus_05, run, reach),
        }
    }

    /// Whether its axis lies across the force: a pendulum, holding nothing
    /// that way and pushed by nothing.
    pub(super) fn across(self) -> bool {
        self.cosine <= ALONG_AXIS_NEGLIGIBLE
    }

    /// How stiffly (N/mm) and up to what force (N) it holds sideways, its
    /// joints slipping `slip` (mm per N) along it and its anchor giving
    /// `anchor_give` and holding `anchor_capacity` there: the force along it
    /// is the force sideways over the cosine, and the point moves sideways the
    /// stretch over the cosine.
    pub(super) fn holds(self, slip: f64, anchor_give: f64, anchor_capacity: f64) -> (f64, f64) {
        if self.across() {
            return (0.0, f64::INFINITY);
        }
        let along = self.give + slip + anchor_give;
        (
            self.cosine * self.cosine / along,
            self.capacity.min(anchor_capacity) * self.cosine,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::IDENTITY;

    /// One fastener lets the parts turn: a moment beyond the part's thickness
    /// finds nothing holding it. Two fasteners apart hold it against their slip.
    #[test]
    fn one_fastener_is_a_pin_two_hold_against_their_slip() {
        let (held, sideways) = ([0.0, 0.0, 2400.0], [0.0, 1.0, 0.0]);
        let foot = [0.0, 0.0, 10.0];
        let slip = 1.0 / SLIP_PER_FASTENER;
        assert_eq!(
            hold(Tie::Pin, foot, held, sideways, 50.0),
            Hold::Turning(slip)
        );
        assert_eq!(
            hold(Tie::Pin, [0.0, 0.0, 2390.0], held, sideways, 50.0),
            Hold::Fixed(slip)
        );
        let two = Tie::Fasteners {
            count: 2.0,
            turning: 2.0 * 25.0 * 25.0,
        };
        let Hold::Fixed(give) = hold(two, foot, held, sideways, 50.0) else {
            panic!("two fasteners apart turn freely");
        };
        let expected = slip / 2.0 + 2390.0_f64.powi(2) / (SLIP_PER_FASTENER * 1250.0);
        assert!((give - expected).abs() < 1e-9);
        assert_eq!(
            hold(Tie::Rigid, foot, held, sideways, 50.0),
            Hold::Fixed(0.0)
        );
    }

    /// R1120ac-PG-903: a strut 50 x 50 x 1 000 at 45° to the force holds with
    /// cos² of its stretching stiffness E A / L and cos of its Euler load; along
    /// the force with all of both; across it, a pendulum, with nothing.
    #[test]
    fn a_strut_holds_with_its_force_along_it() {
        let moduli = (11_000.0, 7_400.0);
        let rod = Obb::new([0.0; 3], &IDENTITY, [0.0; 3], [50.0, 1000.0, 50.0]);
        let ends = ([25.0, 1000.0, 25.0], [25.0, 0.0, 25.0]);
        let stretching = 11_000.0 * 2500.0 / 1000.0;
        let euler = std::f64::consts::PI.powi(2) * 7_400.0 * 50.0_f64.powi(4) / 12.0 / 1.0e6;
        let along = Strut::of(&rod, moduli, ends, [0.0, 1.0, 0.0], 1000.0);
        let (stiffness, capacity) = along.holds(0.0, 0.0, f64::INFINITY);
        assert!(
            (stiffness - stretching).abs() < 1e-9 * stretching,
            "{stiffness}"
        );
        assert!((capacity - euler).abs() < 1e-9 * euler, "{capacity}");
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let diagonal = Strut::of(&rod, moduli, ends, [half, half, 0.0], 1000.0);
        let (stiffness, capacity) = diagonal.holds(0.0, 0.0, f64::INFINITY);
        assert!(
            (stiffness - stretching / 2.0).abs() < 1e-9 * stretching,
            "{stiffness}"
        );
        assert!((capacity - euler * half).abs() < 1e-9 * euler, "{capacity}");
        let pendulum = Strut::of(&rod, moduli, ends, [0.0, 0.0, 1.0], 1000.0);
        assert!(pendulum.across());
        assert_eq!(
            pendulum.holds(0.0, 0.0, f64::INFINITY),
            (0.0, f64::INFINITY)
        );
    }
}
