//! A bare contact between two grounded parts holds only the force along its
//! normal (review 2026-11-25 R1125og-PG-1003): a column between two grounded
//! blocks bearing bare on its ends, everything turned 0.2 degrees in plan from
//! the force, held a brace at 3.64 N/mm where the column holds 0.665 its weak
//! way, the contact taken as rigid every way. A path pushing through a contact
//! slanting to the force carries the force's part along the contact's normal
//! through it, and the rest, along the contact, as the parts hold without it.
//!
//! The two parts are two ways the force goes at once, each with the gives
//! g_n and g_t of its own path, and where they share parts their stresses add
//! up: the give is c² g_n + s² g_t + 2 c s g_nt, c and s the cosine and sine
//! of the force to the normal, the shared term no larger than √(g_n g_t). So
//! the path gives at most (c √g_n + s √g_t)². Leaving it out held a column
//! less stiffly with blocks bearing on its ends than without them, both
//! parts twisting it about the point held 940 mm off, their twists cancelling
//! in the force itself (review 2026-11-26 R1126og-PG-1100: 54.6 N/mm against
//! 98.0). A contact that only bears holds besides the path without it, never
//! instead of it: the path holds at least as stiffly as without the contact.

use super::{Anchorage, Footing, Probe};
use ketchup_geometry::linalg::{dot, length, scale, sub};
use ketchup_model::tolerance::ALONG_AXIS_NEGLIGIBLE;

/// How many bare contacts slanting to the force one path is split at, one
/// within the other.
const SPLITS: usize = 3;

/// A bare contact a path pushes through slanting to the force.
struct Slant {
    /// The two parts bearing on each other, lesser index first.
    pair: (usize, usize),
    /// The cosine of the force to the contact's normal.
    cosine: f64,
    /// The normal the force pushes along.
    normal: [f64; 3],
    /// The sine of the force to the contact's normal.
    sine: f64,
    /// The way along the contact the rest of the force slides.
    tangent: [f64; 3],
}

/// One anchor's search for a path holding `member`, kept off the parts
/// `apart`, having run `start` before the anchor.
struct Search<'a, 'f> {
    footing: &'a Footing<'f>,
    anchor: usize,
    member: usize,
    apart: &'a [usize],
    start: f64,
}

impl Search<'_, '_> {
    /// The path under the force `probe` kept off the contacts `cut`, split
    /// at up to `left` bare contacts slanting to the force.
    fn split(
        &self,
        probe: Probe,
        cut: &[(usize, usize)],
        left: usize,
    ) -> Option<(Anchorage, Vec<usize>, bool)> {
        let footing = self.footing;
        let (found, route) =
            footing.search(self.anchor, self.member, self.apart, probe, self.start, cut)?;
        let pushes = footing.pushes_through(&route, probe.along);
        let Some(slant) = (left > 0)
            .then(|| footing.slanting(&route, probe.along))
            .flatten()
        else {
            return Some((found, route, pushes));
        };
        let along = |way: [f64; 3]| Probe {
            along: way,
            ..probe
        };
        let below = left - 1;
        let (pushed, mut parts, _) = self.split(along(slant.normal), cut, below)?;
        let (slid, beside, _) = self.split(along(slant.tangent), cut, below)?;
        parts.extend(beside);
        let split = combined(&slant, &pushed, &slid);
        let mut kept_off = cut.to_vec();
        kept_off.push(slant.pair);
        match self.split(probe, &kept_off, below) {
            Some(without) if without.0.stiffness_n_mm > split.stiffness_n_mm => Some(without),
            _ => Some((split, parts, true)),
        }
    }
}

impl Footing<'_> {
    /// [`Self::search`] under the force `probe`, a bare contact slanting to it
    /// on the path holding only the part along its normal: the path, the
    /// parts the paths run through, and whether it pushes through a bare
    /// contact ([`Self::bears`]), so holds the other way without it.
    pub(super) fn bearing_search(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
        probe: Probe,
        start: f64,
    ) -> Option<(Anchorage, Vec<usize>, bool)> {
        Search {
            footing: self,
            anchor,
            member,
            apart,
            start,
        }
        .split(probe, &[], SPLITS)
    }

    /// The first bare contact `route` pushes through that slants to the force
    /// `along`.
    fn slanting(&self, route: &[usize], along: [f64; 3]) -> Option<Slant> {
        let unit = scale(along, 1.0 / length(along));
        route
            .windows(2)
            .filter(|pair| self.bears(pair[0], pair[1], unit) == Some(true))
            .find_map(|pair| {
                let normal = self.contact_normal(pair[0], pair[1])?;
                let cosine = dot(normal, unit);
                let rest = sub(unit, scale(normal, cosine));
                let sine = length(rest);
                (sine > ALONG_AXIS_NEGLIGIBLE).then(|| Slant {
                    pair: (pair[0].min(pair[1]), pair[0].max(pair[1])),
                    cosine: cosine.abs(),
                    normal: scale(normal, cosine.signum()),
                    sine,
                    tangent: scale(rest, 1.0 / sine),
                })
            })
    }

    /// Whether the path `route` (its parts in order) pushes through a bare
    /// contact ([`Self::bears`]) under a force `along`.
    fn pushes_through(&self, route: &[usize], along: [f64; 3]) -> bool {
        route
            .windows(2)
            .any(|pair| self.bears(pair[0], pair[1], along) == Some(true))
    }
}

/// How a path holds the force split at the contact `slant` into its part
/// along the normal, held as `pushed`, and the rest, held as `slid`: giving
/// at most (c √g_n + s √g_t)², buckling where either part first buckles.
fn combined(slant: &Slant, pushed: &Anchorage, slid: &Anchorage) -> Anchorage {
    let give = |anchorage: &Anchorage| {
        if anchorage.stiffness_n_mm.is_finite() {
            1.0 / anchorage.stiffness_n_mm
        } else {
            0.0
        }
    };
    let (across, sliding) = (
        slant.cosine * give(pushed).sqrt(),
        slant.sine * give(slid).sqrt(),
    );
    let total = (across + sliding).powi(2);
    let (pushed_capacity, slid_capacity) = (
        pushed.capacity_n / slant.cosine,
        slid.capacity_n / slant.sine,
    );
    Anchorage {
        stiffness_n_mm: if total > 0.0 {
            1.0 / total
        } else {
            f64::INFINITY
        },
        capacity_n: pushed_capacity.min(slid_capacity),
        weakest: if across >= sliding {
            pushed.weakest
        } else {
            slid.weakest
        },
        buckling: if pushed_capacity <= slid_capacity {
            pushed.buckling
        } else {
            slid.buckling
        },
        over_mm: pushed.over_mm.max(slid.over_mm),
    }
}
