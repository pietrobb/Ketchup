//! Review 2026-11-25 R1125og-PG-1006: two drilled parts overlapping by
//! 50 x 100 x 18 mm passed with exit 0 and only a `collision_unverified`
//! warning, although their holes remove at most about 1 000 mm³ of the
//! 90 000 mm³ their boxes share. Boxes settle such a pair: holes, pockets and
//! subtracted tools can only take material away, so the solids share at
//! least what the boxes share less all they could remove from it.
//!
//! Review 2026-11-26 R1126og-PG-1102: what a hole, pocket or tool removes
//! counts only where its box reaches into the shared box; counted whole, six
//! through holes along a long part excused a second part running through it
//! away from them.

use crate::frame::{self, Obb};
use crate::model::{Part, ProgramBooleanKind};
use crate::validate;
use std::f64::consts::PI;

/// Shared volume below this is left to the exact solids.
const MIN_CERTAIN_MM3: f64 = 1.0;

/// Axis-aligned bounds `(min, max)` in one part's frame.
type Bounds = ([f64; 3], [f64; 3]);

/// The least volume the solids of `a` and `b` share when their boxes alone
/// prove they overlap: both are plain boxes of one orientation whose holes,
/// pockets and subtracted tools together remove less than the volume the
/// boxes share. `None` when only the exact solids can tell.
pub(crate) fn certain_overlap_mm3(a: &Part, b: &Part) -> Option<f64> {
    if !validate::is_box(a) || !validate::is_box(b) {
        return None;
    }
    let shared = clipped(a.local_bounds(), validate::in_frame_of(a, b)?);
    let region = Obb::new(a.at_mm, &a.rotation, shared.0, shared.1);
    let left = volume(shared) - removable_mm3(a, region)? - removable_mm3(b, region)?;
    (left > MIN_CERTAIN_MM3).then_some(left)
}

/// The most volume the holes, pockets and subtracted tools of `part` can
/// remove from the world box `region`; `None` when another boolean (a union
/// or intersect) makes its box something other than its material.
fn removable_mm3(part: &Part, region: Obb) -> Option<f64> {
    let within = bounds_of(region.corners().map(|corner| part.to_local(corner)));
    let (min, max) = part.local_bounds();
    let diagonal = (0..3)
        .map(|axis| (max[axis] - min[axis]).powi(2))
        .sum::<f64>()
        .sqrt();
    // A drill's point reaches past the depth by less than its diameter.
    let holes: f64 = part
        .finished_holes()
        .map(|(hole, (entry, inward))| {
            let length = if hole.through {
                diagonal
            } else {
                (hole.depth_mm + hole.diameter_mm).min(diagonal)
            };
            let radius = hole.diameter_mm / 2.0;
            let whole = PI * radius.powi(2) * length;
            let Some(direction) = frame::normalized(inward) else {
                return whole;
            };
            let end: [f64; 3] = std::array::from_fn(|i| entry[i] + direction[i] * length);
            let reach: [f64; 3] =
                std::array::from_fn(|i| radius * (1.0 - direction[i].powi(2)).max(0.0).sqrt());
            let cylinder = (
                std::array::from_fn(|i| entry[i].min(end[i]) - reach[i]),
                std::array::from_fn(|i| entry[i].max(end[i]) + reach[i]),
            );
            whole.min(volume(clipped(cylinder, within)))
        })
        .sum();
    let pockets: f64 = part
        .finished_pockets()
        .map(|(pocket, corners)| {
            let [du, dv] = pocket.extent_mm();
            let whole = (du * dv).abs() * pocket.depth_mm.min(diagonal);
            whole.min(volume(clipped(bounds_of(corners), within)))
        })
        .sum();
    let mut tools = 0.0;
    for (boolean, finished) in part.finished_booleans() {
        if boolean.kind != ProgramBooleanKind::Subtract {
            return None;
        }
        let whole = 8.0 * finished.half.iter().product::<f64>();
        let tool = bounds_of(finished.corners().map(|corner| part.to_local(corner)));
        tools += whole.min(volume(clipped(tool, within)));
    }
    Some(holes + pockets + tools)
}

fn bounds_of(points: [[f64; 3]; 8]) -> Bounds {
    points.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(min, max), point| {
            (
                std::array::from_fn(|i| min[i].min(point[i])),
                std::array::from_fn(|i| max[i].max(point[i])),
            )
        },
    )
}

fn clipped((a_min, a_max): Bounds, (b_min, b_max): Bounds) -> Bounds {
    (
        std::array::from_fn(|i| a_min[i].max(b_min[i])),
        std::array::from_fn(|i| a_max[i].min(b_max[i])),
    )
}

fn volume((min, max): Bounds) -> f64 {
    (0..3).map(|i| (max[i] - min[i]).max(0.0)).product()
}
