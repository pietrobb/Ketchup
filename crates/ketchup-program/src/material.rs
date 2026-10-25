//! How much of a part's material lies along a straight line from a point on
//! it: the extruded profile less the cuts and pockets in it, not the reach of
//! the profile before them (review 2026-10-17 PG-4). A 12 mm hole in an
//! 18 mm thick part on the line of a 10 mm groove in its back meets the groove
//! after 8 mm; in an L profile it ends where the 18 mm leg does. A drill of
//! radius r is followed on its axis and on its rim (review 2026-10-18 PG-11):
//! a 12 mm hole whose axis misses a groove by 4 mm still cuts 2 mm into it.

use crate::contact::polygon::{self, Rings};
use crate::faces::profile_outline;
use crate::model::{Part, ProgramOperation, ProgramPartBody};
use ketchup_geometry::linalg::{add, cross, dot, length, scale, sub};
use ketchup_model::tolerance::CONTACT_FIT_MM;

/// Steps along the line between material tests; a gap thinner than this
/// can be missed.
// not a tolerance: the probe step along the line, refined by bisection.
const STEP_MM: f64 = 0.2;

/// Lines on a drill's rim; a cut reaching less than r·(1 − cos(π/32)),
/// 0.03 mm for a 12 mm drill, into the rim between two of them is missed.
// not a tolerance: a sampling count.
const RIM_LINES: u32 = 32;

impl Part {
    /// Distance in mm from `entry` along unit `inward` (both in the part's
    /// frame) to where a drill of `radius` (0 for the line alone) first
    /// leaves the part's material, on its axis or its rim, looked for up to
    /// `limit`. The rim is taken `CONTACT_FIT_MM` inside, so that a hole
    /// touching a face's edge keeps the material of that face. `None` when
    /// the part is not an extrusion, or has a shell, boolean or face offset,
    /// which this does not model; other holes and rounded or chamfered edges
    /// are left out.
    #[must_use]
    pub fn material_along(
        &self,
        entry: [f64; 3],
        inward: [f64; 3],
        radius: f64,
        limit: f64,
    ) -> Option<f64> {
        let material = self.material_test()?;
        let mut least = leaves(&material, entry, inward, limit);
        let rim = radius - CONTACT_FIT_MM;
        if rim > 0.0 {
            // Two unit vectors across `inward`, from the world axis least
            // along it.
            let axis = if inward[0].abs() < 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 1.0, 0.0]
            };
            let u = cross(inward, axis);
            let u = scale(u, 1.0 / length(u));
            let v = cross(inward, u);
            for i in 0..RIM_LINES {
                let angle = std::f64::consts::TAU * f64::from(i) / f64::from(RIM_LINES);
                let across = add(scale(u, rim * angle.cos()), scale(v, rim * angle.sin()));
                least = least.min(leaves(&material, add(entry, across), inward, least));
            }
        }
        Some(least)
    }

    /// Whether a point in the part's frame is in its material, built once for
    /// all the lines of a drill. Holes and edge finishes are left out of it,
    /// so a part with thousands of holes is not walked through them for
    /// every point (review PRG-7).
    fn material_test(&self) -> Option<impl Fn([f64; 3]) -> bool + '_> {
        let ProgramPartBody::Extrusion {
            segments,
            distance_mm,
            ..
        } = &self.body
        else {
            return None;
        };
        let body_z = [distance_mm.min(0.0), distance_mm.max(0.0)];
        // Each cut's outline and the top it is cut down from: the body's
        // top where the mirrors before it left it.
        let mut z = body_z;
        let mut steps: Vec<(&ProgramOperation, Option<(Rings, f64)>)> = Vec::new();
        for operation in &self.operations {
            match operation {
                ProgramOperation::Shell(_)
                | ProgramOperation::Boolean(_)
                | ProgramOperation::FaceOffset(_) => return None,
                ProgramOperation::Mirror(mirror) => {
                    if mirror.axis == 2 {
                        z = [2.0 * mirror.center_mm - z[1], 2.0 * mirror.center_mm - z[0]];
                    }
                    steps.push((operation, None));
                }
                ProgramOperation::Cut(cut) => {
                    steps.push((
                        operation,
                        Some((vec![profile_outline(&cut.segments)], z[1])),
                    ));
                }
                ProgramOperation::Pocket(_) => steps.push((operation, None)),
                _ => {}
            }
        }
        let outline = vec![profile_outline(segments)];
        // Walked back through the operations, each mirror reflecting the
        // point into the frame before it.
        Some(move |mut point: [f64; 3]| {
            for (operation, cut) in steps.iter().rev() {
                let removed = match (operation, cut) {
                    (ProgramOperation::Mirror(mirror), _) => {
                        point[mirror.axis] = 2.0 * mirror.center_mm - point[mirror.axis];
                        false
                    }
                    (ProgramOperation::Cut(cut), Some((rings, top))) => {
                        cut.depth_mm.is_none_or(|depth| point[2] >= top - depth)
                            && polygon::contains(rings, [point[0], point[1]])
                    }
                    (ProgramOperation::Pocket(pocket), _) => {
                        let offset = sub(point, pocket.corner_mm);
                        let [du, dv] = pocket.extent_mm();
                        [
                            (pocket.u, du),
                            (pocket.v, dv),
                            (pocket.inward, pocket.depth_mm),
                        ]
                        .iter()
                        .all(|&(axis, length)| (0.0..=length).contains(&dot(offset, axis)))
                    }
                    _ => false,
                };
                if removed {
                    return false;
                }
            }
            (body_z[0]..=body_z[1]).contains(&point[2])
                && polygon::contains(&outline, [point[0], point[1]])
        })
    }
}

/// Distance from `entry` along unit `inward` to where the line first leaves
/// `material`, looked for up to `limit`. A line that starts in a recess of
/// the entry face, as the rim of a hole reaching a groove in the face it is
/// drilled from, is followed from the recess's floor: the drill meets
/// material there, not 0 mm in (review 2026-10-24 PG-23).
fn leaves(
    material: &impl Fn([f64; 3]) -> bool,
    entry: [f64; 3],
    inward: [f64; 3],
    limit: f64,
) -> f64 {
    let material = |t: f64| material(add(entry, scale(inward, t)));
    // To a millionth of a mm, so that an 18 mm thick part turned in space
    // is 18 mm, not 17.99999999999.
    let round = |mm: f64| (mm * 1.0e6).round() / 1.0e6;
    let mut inside = 0.0;
    // Just past the face, so that a skin thinner than a step still counts.
    if !material(CONTACT_FIT_MM) {
        loop {
            inside += STEP_MM;
            if inside > limit {
                return round(limit);
            }
            if material(inside) {
                break;
            }
        }
    }
    loop {
        let next = inside + STEP_MM;
        if next > limit {
            return round(limit);
        }
        if !material(next) {
            let (mut a, mut b) = (inside, next);
            for _ in 0..30 {
                let half = (a + b) / 2.0;
                if material(half) { a = half } else { b = half }
            }
            return round(b);
        }
        inside = next;
    }
}
