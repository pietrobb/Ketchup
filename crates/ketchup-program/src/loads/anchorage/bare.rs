//! A bare contact between two grounded parts holds only the force along its
//! normal (review 2026-11-25 R1125og-PG-1003): a column between two grounded
//! blocks bearing bare on its ends, everything turned 0.2 degrees in plan from
//! the force, held a brace at 3.64 N/mm where the column holds 0.665 its weak
//! way, the contact taken as rigid every way. A path pushing through a contact
//! slanting to the force carries the force's part along the contact's normal
//! through it, and the rest, along the contact, as the parts hold without it:
//! it gives c² g_n + s² g_t, c and s the cosine and sine of the force to the
//! normal, k / cos² beta for the column.

use super::{Anchorage, Footing, Probe};
use ketchup_geometry::linalg::{dot, length, scale, sub};
use ketchup_model::tolerance::ALONG_AXIS_NEGLIGIBLE;

/// How many bare contacts slanting to the force one path is split at, one
/// within the other.
const SPLITS: usize = 3;

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
        self.split(anchor, member, apart, probe, start, SPLITS)
    }

    fn split(
        &self,
        anchor: usize,
        member: usize,
        apart: &[usize],
        probe: Probe,
        start: f64,
        left: usize,
    ) -> Option<(Anchorage, Vec<usize>, bool)> {
        let (found, route) = self.search(anchor, member, apart, probe, start)?;
        let pushes = self.pushes_through(&route, probe.along);
        let Some((cosine, normal, sine, tangent)) = (left > 0)
            .then(|| self.slanting(&route, probe.along))
            .flatten()
        else {
            return Some((found, route, pushes));
        };
        let along = |way: [f64; 3]| Probe {
            along: way,
            ..probe
        };
        let (pushed, mut parts, _) =
            self.split(anchor, member, apart, along(normal), start, left - 1)?;
        let (slid, beside, _) =
            self.split(anchor, member, apart, along(tangent), start, left - 1)?;
        parts.extend(beside);
        let give = |anchorage: &Anchorage| {
            if anchorage.stiffness_n_mm.is_finite() {
                1.0 / anchorage.stiffness_n_mm
            } else {
                0.0
            }
        };
        let (across, sliding) = (cosine * cosine * give(&pushed), sine * sine * give(&slid));
        let (pushed_capacity, slid_capacity) = (pushed.capacity_n / cosine, slid.capacity_n / sine);
        let anchorage = Anchorage {
            stiffness_n_mm: if across + sliding > 0.0 {
                1.0 / (across + sliding)
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
        };
        Some((anchorage, parts, true))
    }

    /// The first bare contact `route` pushes through that slants to the force
    /// `along`: the cosine of the force to its normal and the normal it pushes
    /// along, the sine and the way along the contact the rest of the force
    /// slides.
    fn slanting(&self, route: &[usize], along: [f64; 3]) -> Option<(f64, [f64; 3], f64, [f64; 3])> {
        let unit = scale(along, 1.0 / length(along));
        route
            .windows(2)
            .filter(|pair| self.bears(pair[0], pair[1], unit) == Some(true))
            .find_map(|pair| {
                let normal = self.contact_normal(pair[0], pair[1])?;
                let cosine = dot(normal, unit);
                let rest = sub(unit, scale(normal, cosine));
                let sine = length(rest);
                (sine > ALONG_AXIS_NEGLIGIBLE).then(|| {
                    (
                        cosine.abs(),
                        scale(normal, cosine.signum()),
                        sine,
                        scale(rest, 1.0 / sine),
                    )
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
