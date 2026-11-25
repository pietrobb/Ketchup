//! How stiffly, and up to what force, a path of parts to the floor holds a
//! point sideways (review 2026-11-13 r95, structural point 1; R1113r95-PG-702
//! to PG-704): a unit force across the member at the point it holds, carried
//! part by part to a grounded part fixed on the floor. Each part is a bending member from
//! where the path enters it to where it leaves, bending, twisting and stretching
//! under the moment and force the point puts on it; each joint between two
//! parts slips and turns on its fasteners, turns freely on one (a pivot) or,
//! between two grounded parts, is rigid. Their give adds up in series, and the
//! stiffness is one over the sum (the unit-load method on the path).

use crate::frame::Obb;
use ketchup_geometry::linalg::{cross, dot, length, sub};
use ketchup_model::tolerance::ALONG_AXIS_NEGLIGIBLE;

/// Shear modulus over modulus of elasticity of wood, as EN 338 and EN 14080
/// give them (G_mean = E_0,mean / 16).
const SHEAR_PER_E: f64 = 1.0 / 16.0;

/// The slip modulus of one fastener per shear plane, N/mm: EN 1995-1-1
/// table 7.1, rho_m^1.5 d / 23 for screws and pins, taken for a fastener of
/// d = 6 mm in wood of rho_m = 420 kg/m³, the size of the program unknown.
pub(super) const SLIP_PER_FASTENER: f64 = 2245.0;

/// How a joint between two parts on the path holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tie {
    /// Two grounded parts, both fixed to the ground: nothing gives between
    /// them.
    Rigid,
    /// One fastener in a contact facing `normal` (None where the parts share
    /// no face): it slips, and the parts turn about it freely.
    Pin { normal: Option<[f64; 3]> },
    /// `count` fasteners spread about their middle by `spread` = the sum of
    /// r rᵀ over their offsets r from it (mm²), in a contact facing `normal`
    /// (None where the parts share no face): they slip, and turn against
    /// their slip about the normal only (R1121ac-PG-1001).
    Fasteners {
        count: f64,
        spread: [[f64; 3]; 3],
        normal: Option<[f64; 3]>,
    },
}

impl Tie {
    /// `count` fasteners at `points` in a contact facing `normal`.
    pub(super) fn fasteners(points: &[[f64; 3]], normal: Option<[f64; 3]>) -> Self {
        let count = f64::from(u32::try_from(points.len()).unwrap_or(u32::MAX));
        let middle: [f64; 3] = std::array::from_fn(|i| {
            points.iter().map(|point| point[i]).sum::<f64>() / count.max(1.0)
        });
        let mut spread = [[0.0; 3]; 3];
        for point in points {
            let r = sub(*point, middle);
            for (row, line) in spread.iter_mut().enumerate() {
                for (column, cell) in line.iter_mut().enumerate() {
                    *cell += r[row] * r[column];
                }
            }
        }
        Self::Fasteners {
            count,
            spread,
            normal,
        }
    }
}

/// The sum of squared distances of fasteners spread by `spread` from the
/// axis through their middle along `axis` (unit), mm²: what they turn
/// against, slipping, about it.
pub(super) fn turning_about(spread: &[[f64; 3]; 3], axis: [f64; 3]) -> f64 {
    let trace = spread[0][0] + spread[1][1] + spread[2][2];
    let along: f64 = (0..3)
        .map(|row| {
            (0..3)
                .map(|column| axis[row] * spread[row][column] * axis[column])
                .sum::<f64>()
        })
        .sum();
    (trace - along).max(0.0)
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
    // The force across the axis shears the section, 5/6 of it (Timoshenko): a
    // deep part or a sheet carrying the force in its plane gives as much in
    // shear as in bending (R1119og-PG-801, R1124og-PG-906).
    let across = (dot(sideways, sideways) - pull * pull).max(0.0);
    squared(obb.axes[i]) / (modulus * bending_i)
        + squared(obb.axes[j]) / (modulus * bending_j)
        + squared(obb.axes[along]) / (shear * twisting)
        + pull * pull * long / (modulus * area)
        + across * long / (5.0 / 6.0 * shear * area)
}

/// The largest relative slenderness of a piece that does not buckle, EN
/// 1995-1-1 6.3.2(3): at most this, k_c = 1 and it crushes first.
const STOCKY_SLENDERNESS: f64 = 0.3;

/// The largest force `sideways` the part of box `obb` takes before buckling
/// where it carries it along its axis between `entry` and `exit`, N, by
/// [`buckling_load`] over the length `reach` of the rod it is a piece of:
/// short pieces end to end buckle as one rod
/// (R1113r95-PG-700, R1121ac-PG-1002). Infinite where it carries none.
pub(super) fn part_capacity(
    obb: &Obb,
    strength: (f64, f64),
    entry: [f64; 3],
    exit: [f64; 3],
    sideways: [f64; 3],
    reach: f64,
) -> f64 {
    let run = sub(entry, exit);
    match pushed_along(obb, entry, exit, sideways) {
        Some(along) if reach > f64::EPSILON => buckling_load(obb, strength, run, reach) / along,
        _ => f64::INFINITY,
    }
}

/// The load the piece takes pushed along its axis, N: k_c f_c,0,k A (EN
/// 1995-1-1 6.3.2 (6.25)-(6.29)) at the compressive strength `fc0_k` (N/mm²),
/// with the relative slenderness of [`euler_load`] at the fifth-percentile
/// modulus `modulus_05`. A stocky piece crushes at f_c,0,k A; a slender one
/// buckles below Euler's load. β_c is that of a sawn piece, 0.2, also for
/// glulam (0.1), which it puts on the safe side. Euler's load alone gave a
/// piece just past the relative slenderness 0.3 eleven times its crushing load
/// (R1122ac, review §4 P3; before it, review 2026-11-21 ac P3).
pub(super) fn buckling_load(
    obb: &Obb,
    (modulus_05, fc0_k): (f64, f64),
    run: [f64; 3],
    reach: f64,
) -> f64 {
    let euler = euler_load(obb, modulus_05, run, reach);
    let (_, [(_, wide_i), (_, wide_j)]) = section(obb, run);
    let crushing = fc0_k * wide_i * wide_j;
    crushing * buckling_factor((crushing / euler).sqrt())
}

/// k_c of the relative slenderness `slenderness` (EN 1995-1-1 (6.25)-(6.29)):
/// 1 up to 0.3.
fn buckling_factor(slenderness: f64) -> f64 {
    if slenderness <= STOCKY_SLENDERNESS {
        return 1.0;
    }
    let k = 0.5 * (1.0 + SAWN_STRAIGHTNESS * (slenderness - STOCKY_SLENDERNESS))
        + 0.5 * slenderness * slenderness;
    1.0 / (k + (k * k - slenderness * slenderness).sqrt())
}

/// β_c of a sawn piece (EN 1995-1-1 (6.29)).
const SAWN_STRAIGHTNESS: f64 = 0.2;

/// The share of a force `sideways` the part of box `obb` carries along its
/// axis between `entry` and `exit`, where it carries any. Along the part's
/// axis, as [`part_give`] stretches it, not along the run: an upright hanger
/// bent sideways runs a little across from where the member holds it to its
/// anchor, and was given its Euler load over that share, in MN (review
/// 2026-11-20 ac P3).
pub(super) fn pushed_along(
    obb: &Obb,
    entry: [f64; 3],
    exit: [f64; 3],
    sideways: [f64; 3],
) -> Option<f64> {
    let run = sub(entry, exit);
    if length(run) <= f64::EPSILON {
        return None;
    }
    let (axis, _) = section(obb, run);
    let along = dot(sideways, obb.axes[axis]).abs();
    (along > ALONG_AXIS_NEGLIGIBLE).then_some(along)
}

/// Euler's load of the part of box `obb` pushed along its axis nearest `run`
/// over the length `reach`, about its weaker axis, at the modulus
/// `modulus_05`, N; infinite over no length.
fn euler_load(obb: &Obb, modulus_05: f64, run: [f64; 3], reach: f64) -> f64 {
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

    /// The give of a cantilever `width` x `deep` x `long` pushed at its top
    /// across `deep`: bending L³ / (3 E I) and shear L / (5/6 G A).
    fn cantilever_give(width: f64, deep: f64, long: f64) -> (f64, f64) {
        let inertia = width * deep.powi(3) / 12.0;
        let bending = long.powi(3) / (3.0 * 11_500.0 * inertia);
        let shear = long / (5.0 / 6.0 * 11_500.0 * SHEAR_PER_E * width * deep);
        (bending, shear)
    }

    /// A post fixed at its foot and pushed at its top is a cantilever that
    /// bends and shears.
    #[test]
    fn a_post_pushed_at_its_top_bends_as_a_cantilever() {
        let obb = post(50.0, 100.0, 2400.0);
        let top = [25.0, 50.0, 2400.0];
        let give = part_give(&obb, 11_500.0, top, [25.0, 50.0, 0.0], top, [0.0, 1.0, 0.0]);
        let (bending, shear) = cantilever_give(50.0, 100.0, 2400.0);
        assert!(
            (give - bending - shear).abs() < 1e-9 * give,
            "{}",
            1.0 / give
        );
    }

    /// R1124og-PG-906: a part on the path deep for its length gives in shear
    /// as much as in bending. A wall 1 200 x 40 x 2 400 pushed in its plane at
    /// its top was held 2.2 times as stiffly as it is, by bending alone.
    #[test]
    fn a_wall_pushed_in_its_plane_shears() {
        let obb = post(1200.0, 40.0, 2400.0);
        let top = [600.0, 20.0, 2400.0];
        let give = part_give(
            &obb,
            11_500.0,
            top,
            [600.0, 20.0, 0.0],
            top,
            [1.0, 0.0, 0.0],
        );
        let (bending, shear) = cantilever_give(40.0, 1200.0, 2400.0);
        assert!(
            (give - bending - shear).abs() < 1e-9 * give,
            "{}",
            1.0 / give
        );
        assert!(give > 2.0 * bending, "{give} against {bending}");
    }

    /// Pushed along its length a rod only stretches, and buckles just under
    /// Euler's load over the length the path has run.
    #[test]
    fn a_rod_pushed_along_it_stretches_and_buckles() {
        let obb = post(7.0, 400.0, 7.0);
        let (entry, exit) = ([3.5, 0.0, 3.5], [3.5, 400.0, 3.5]);
        let give = part_give(&obb, 11_500.0, entry, exit, entry, [0.0, 1.0, 0.0]);
        assert!((give - 400.0 / (11_500.0 * 49.0)).abs() < 1e-12);
        let capacity = part_capacity(&obb, (9_600.0, 24.0), entry, exit, [0.0, 1.0, 0.0], 2400.0);
        let euler =
            std::f64::consts::PI.powi(2) * 9_600.0 * 7.0_f64.powi(4) / 12.0 / 2400.0_f64.powi(2);
        assert!(capacity < euler && capacity > 0.98 * euler, "{capacity}");
        let across = part_capacity(
            &obb,
            (9_600.0, 24.0),
            entry,
            [3.5, 0.0, -400.0],
            [0.0, 1.0, 0.0],
            2400.0,
        );
        assert!(across.is_infinite());
    }

    /// An upright 50 x 100 x 600 held at its foot 100 mm to the side of where
    /// it is held at its top bends: nothing pushes along it, so it does not
    /// buckle (it was given about 1 MN, review 2026-11-20 ac P3).
    #[test]
    fn an_upright_bent_sideways_on_a_slanting_run_does_not_buckle() {
        let obb = post(50.0, 100.0, 600.0);
        let capacity = part_capacity(
            &obb,
            (7_400.0, 21.0),
            [25.0, 0.0, 550.0],
            [25.0, 100.0, 150.0],
            [0.0, 1.0, 0.0],
            412.0,
        );
        assert!(capacity.is_infinite(), "{capacity}");
    }

    /// A rod 50 x 50 pushed along it takes k_c f_c,0,k A: over 200 mm (relative
    /// slenderness about 0.24) it crushes at f_c,0,k A, just past 0.3 it takes
    /// nearly that, not Euler's eleven times as much, and at 0.49 and 1 it takes
    /// k_c of it, 0.953 and 0.689 (R1122ac, review §4 P3).
    #[test]
    fn a_rod_takes_its_buckling_factor_of_its_crushing_load() {
        let obb = post(50.0, 1000.0, 50.0);
        let run = [0.0, 1000.0, 0.0];
        let crushing = 21.0 * 2500.0;
        let inertia = 50.0_f64.powi(4) / 12.0;
        // The length over which Euler's load is f_c,0,k A over `slenderness` squared.
        let reach = |slenderness: f64| {
            std::f64::consts::PI * (7_400.0 * inertia / crushing).sqrt() * slenderness
        };
        let load = |reach: f64| buckling_load(&obb, (7_400.0, 21.0), run, reach);
        assert!((load(200.0) - crushing).abs() < 1e-9 * crushing);
        for (slenderness, factor) in [(0.301, 0.999_78), (0.49, 0.953_04), (1.0, 0.689_34)] {
            let taken = load(reach(slenderness)) / crushing;
            assert!((taken - factor).abs() < 1e-4, "{slenderness}: {taken}");
        }
        let euler = crushing / 4.0;
        assert!(load(reach(2.0)) < euler, "{}", load(reach(2.0)));
    }
}
