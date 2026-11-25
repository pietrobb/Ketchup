//! Review 2026-11-25 R1125og-PG-1006: two drilled parts overlapping by
//! 50 x 100 x 18 mm passed with exit 0 and only a `collision_unverified`
//! warning, although their holes remove at most about 1 000 mm³ of the
//! 90 000 mm³ their boxes share. Boxes settle such a pair: holes, pockets and
//! subtracted tools can only take material away, so the solids share at
//! least what the boxes share less all they could remove.

use crate::model::{Part, ProgramBooleanKind};
use crate::validate;
use std::f64::consts::PI;

/// Shared volume below this is left to the exact solids.
const MIN_CERTAIN_MM3: f64 = 1.0;

/// The least volume the solids of `a` and `b` share when their boxes alone
/// prove they overlap: both are plain boxes of one orientation whose holes,
/// pockets and subtracted tools together remove less than the volume the
/// boxes share. `None` when only the exact solids can tell.
pub(crate) fn certain_overlap_mm3(a: &Part, b: &Part) -> Option<f64> {
    if !validate::is_box(a) || !validate::is_box(b) {
        return None;
    }
    let (a_min, a_max) = a.local_bounds();
    let (b_min, b_max) = validate::in_frame_of(a, b)?;
    let shared: f64 = (0..3)
        .map(|axis| (a_max[axis].min(b_max[axis]) - a_min[axis].max(b_min[axis])).max(0.0))
        .product();
    let left = shared - removable_mm3(a)? - removable_mm3(b)?;
    (left > MIN_CERTAIN_MM3).then_some(left)
}

/// The most volume the holes, pockets and subtracted tools of `part` can
/// remove; `None` when another boolean (a union or intersect) makes its box
/// something other than its material.
fn removable_mm3(part: &Part) -> Option<f64> {
    let (min, max) = part.local_bounds();
    let diagonal = (0..3)
        .map(|axis| (max[axis] - min[axis]).powi(2))
        .sum::<f64>()
        .sqrt();
    // A drill's point reaches past the depth by less than its diameter.
    let holes: f64 = part
        .holes()
        .map(|hole| {
            let length = if hole.through {
                diagonal
            } else {
                (hole.depth_mm + hole.diameter_mm).min(diagonal)
            };
            PI * (hole.diameter_mm / 2.0).powi(2) * length
        })
        .sum();
    let pockets: f64 = part
        .pockets()
        .map(|pocket| {
            let [du, dv] = pocket.extent_mm();
            (du * dv).abs() * pocket.depth_mm.min(diagonal)
        })
        .sum();
    let mut tools = 0.0;
    for (boolean, finished) in part.finished_booleans() {
        if boolean.kind != ProgramBooleanKind::Subtract {
            return None;
        }
        tools += 8.0 * finished.half.iter().product::<f64>();
    }
    Some(holes + pockets + tools)
}
