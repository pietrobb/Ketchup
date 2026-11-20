//! How stiffly, and up to what force, a path of parts to the floor holds a
//! point sideways (review 2026-11-13 r95, structural point 1; R1113r95-PG-702
//! to PG-704): a unit force across the member at the point it holds, carried
//! part by part to a grounded part fixed on the floor. Each part is a beam from
//! where the path enters it to where it leaves, bending, twisting and stretching
//! under the moment and force the point puts on it; each joint between two
//! parts slips and turns on its fasteners, turns freely on one (a hinge) or,
//! between two grounded parts, is rigid. Their give adds up in series, and the
//! stiffness is one over the sum (the unit-load method on the path).

use crate::frame::Obb;
use ketchup_geometry::linalg::{cross, dot, length, scale, sub};
use ketchup_model::tolerance::ALONG_AXIS_NEGLIGIBLE;

/// Shear modulus over modulus of elasticity of timber, as EN 338 and EN 14080
/// give them (G_mean = E_0,mean / 16).
const SHEAR_PER_E: f64 = 1.0 / 16.0;

/// The slip modulus of one fastener per shear plane, N/mm: EN 1995-1-1
/// table 7.1, rho_m^1.5 d / 23 for screws and dowels, taken for a fastener of
/// d = 6 mm in timber of rho_m = 420 kg/m³, the size of the program unknown.
pub(super) const SLIP_PER_FASTENER: f64 = 2245.0;

/// How a joint between two parts on the path holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tie {
    /// Two grounded parts, both fixed to the ground: nothing gives between
    /// them.
    Rigid,
    /// One fastener: it slips, and the parts turn about it freely.
    Pin,
    /// `count` fasteners `turning` = the sum of their squared distances from
    /// their middle (mm²) apart: they slip, and turn against their slip.
    Fasteners { count: f64, turning: f64 },
}

/// The moment a unit force `sideways` at `held` puts on a section at `at`,
/// N mm per N.
pub(super) fn moment(held: [f64; 3], at: [f64; 3], sideways: [f64; 3]) -> [f64; 3] {
    cross(sub(held, at), sideways)
}

/// A part's section across the axis of `obb` nearest `direction`: that axis
/// and the two across it with the part's widths along them.
pub(super) fn section(obb: &Obb, direction: [f64; 3]) -> (usize, [(usize, f64); 2]) {
    let along = (0..3)
        .max_by(|a, b| {
            dot(obb.axes[*a], direction)
                .abs()
                .total_cmp(&dot(obb.axes[*b], direction).abs())
        })
        .unwrap_or(2);
    let [i, j] = match along {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    (along, [(i, 2.0 * obb.half[i]), (j, 2.0 * obb.half[j])])
}

/// How far a unit force `sideways` at `held` moves it through the part of box
/// `obb` and modulus `modulus` (N/mm²) between `entry`, where the path enters
/// it, and `exit`, where it leaves for the ground, mm per N.
pub(super) fn part_give(
    obb: &Obb,
    modulus: f64,
    entry: [f64; 3],
    exit: [f64; 3],
    held: [f64; 3],
    sideways: [f64; 3],
) -> f64 {
    let run = sub(entry, exit);
    let long = length(run);
    if long <= f64::EPSILON {
        return 0.0;
    }
    let (along, [(i, wide_i), (j, wide_j)]) = section(obb, run);
    let area = wide_i * wide_j;
    // About axis i the section bends across its width along j.
    let bending_i = wide_i * wide_j.powi(3) / 12.0;
    let bending_j = wide_j * wide_i.powi(3) / 12.0;
    let (wide, thin) = (wide_i.max(wide_j), wide_i.min(wide_j));
    let twisting = (1.0 - 0.63 * thin / wide) / 3.0 * wide * thin.powi(3);
    let (start, end) = (moment(held, exit, sideways), moment(held, entry, sideways));
    // The integral of a moment's square running straight from one end to the other.
    let squared = |axis: [f64; 3]| {
        let (a, b) = (dot(start, axis), dot(end, axis));
        long * (a * a + a * b + b * b) / 3.0
    };
    let shear = modulus * SHEAR_PER_E;
    let pull = dot(sideways, obb.axes[along]);
    squared(obb.axes[i]) / (modulus * bending_i)
        + squared(obb.axes[j]) / (modulus * bending_j)
        + squared(obb.axes[along]) / (shear * twisting)
        + pull * pull * long / (modulus * area)
}

/// [`part_give`] of a brace or a diaphragm's sheet from the point of the
/// member it holds, `entry`, to where it meets its anchor, `exit`, with its
/// shear (5/6 of the section, Timoshenko) as well: a sheet carries the force
/// across its plane, where shear gives as much as bending (R1119og-PG-801).
pub(super) fn brace_give(
    obb: &Obb,
    modulus: f64,
    entry: [f64; 3],
    exit: [f64; 3],
    sideways: [f64; 3],
) -> f64 {
    let run = sub(entry, exit);
    let long = length(run);
    if long <= f64::EPSILON {
        return 0.0;
    }
    let (_, [(_, wide_i), (_, wide_j)]) = section(obb, run);
    let along = dot(sideways, scale(run, 1.0 / long));
    let across = 1.0 - along * along;
    part_give(obb, modulus, entry, exit, entry, sideways)
        + across.max(0.0) * long / (5.0 / 6.0 * modulus * SHEAR_PER_E * wide_i * wide_j)
}

/// The largest force `sideways` the part of box `obb` takes before buckling
/// where it carries it along its axis between `entry` and `exit`, N, by
/// Euler's load at the fifth-percentile modulus `modulus_05` over the length
/// `reach` the path has run from the point held to `exit`: short pieces end to
/// end buckle as one rod (R1113r95-PG-700). Infinite where it carries none.
pub(super) fn part_capacity(
    obb: &Obb,
    modulus_05: f64,
    entry: [f64; 3],
    exit: [f64; 3],
    sideways: [f64; 3],
    reach: f64,
) -> f64 {
    let run = sub(entry, exit);
    let long = length(run);
    if long <= f64::EPSILON || reach <= f64::EPSILON {
        return f64::INFINITY;
    }
    let along = dot(sideways, scale(run, 1.0 / long)).abs();
    if along <= ALONG_AXIS_NEGLIGIBLE {
        return f64::INFINITY;
    }
    euler_load(obb, modulus_05, run, reach) / along
}

/// Euler's load of the part of box `obb` pushed along its axis nearest `run`
/// over the length `reach`, about its weaker axis, at the modulus
/// `modulus_05`, N; infinite over no length.
pub(super) fn euler_load(obb: &Obb, modulus_05: f64, run: [f64; 3], reach: f64) -> f64 {
    if reach <= f64::EPSILON {
        return f64::INFINITY;
    }
    let (_, [(_, wide_i), (_, wide_j)]) = section(obb, run);
    let least = (wide_i * wide_j.powi(3)).min(wide_j * wide_i.powi(3)) / 12.0;
    std::f64::consts::PI.powi(2) * modulus_05 * least / (reach * reach)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::IDENTITY;

    fn post(width: f64, thick: f64, height: f64) -> Obb {
        Obb::new([0.0; 3], &IDENTITY, [0.0; 3], [width, thick, height])
    }

    /// A post fixed at its foot and pushed at its top is a cantilever:
    /// 3 E I / L³.
    #[test]
    fn a_post_pushed_at_its_top_bends_as_a_cantilever() {
        let obb = post(50.0, 100.0, 2400.0);
        let top = [25.0, 50.0, 2400.0];
        let give = part_give(&obb, 11_500.0, top, [25.0, 50.0, 0.0], top, [0.0, 1.0, 0.0]);
        let inertia = 50.0 * 100.0_f64.powi(3) / 12.0;
        let cantilever = 3.0 * 11_500.0 * inertia / 2400.0_f64.powi(3);
        assert!(
            (1.0 / give - cantilever).abs() < 1e-6 * cantilever,
            "{}",
            1.0 / give
        );
    }

    /// Pushed along its length a rod only stretches, and buckles at Euler's
    /// load over the length the path has run.
    #[test]
    fn a_rod_pushed_along_it_stretches_and_buckles() {
        let obb = post(7.0, 400.0, 7.0);
        let (entry, exit) = ([3.5, 0.0, 3.5], [3.5, 400.0, 3.5]);
        let give = part_give(&obb, 11_500.0, entry, exit, entry, [0.0, 1.0, 0.0]);
        assert!((give - 400.0 / (11_500.0 * 49.0)).abs() < 1e-12);
        let capacity = part_capacity(&obb, 9_600.0, entry, exit, [0.0, 1.0, 0.0], 2400.0);
        let euler =
            std::f64::consts::PI.powi(2) * 9_600.0 * 7.0_f64.powi(4) / 12.0 / 2400.0_f64.powi(2);
        assert!((capacity - euler).abs() < 1e-9, "{capacity}");
        let across = part_capacity(
            &obb,
            9_600.0,
            entry,
            [3.5, 0.0, -400.0],
            [0.0, 1.0, 0.0],
            2400.0,
        );
        assert!(across.is_infinite());
    }
}
