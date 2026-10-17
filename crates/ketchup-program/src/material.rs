//! How much of a part's material lies along a straight line from a point on
//! it: the extruded profile less the cuts and pockets in it, not the reach of
//! the profile before them (review 2026-10-17 PG-4). A 12 mm hole in an
//! 18 mm thick part on the line of a 10 mm groove in its back meets the groove
//! after 8 mm; in an L profile it ends where the 18 mm leg does.

use crate::contact::polygon::{self, Rings};
use crate::faces::profile_outline;
use crate::model::{Part, ProgramOperation, ProgramPartBody};
use ketchup_geometry::linalg::{dot, sub};

/// Steps along the line between material tests; a gap thinner than this
/// can be missed.
// not a tolerance: the probe step along the line, refined by bisection.
const STEP_MM: f64 = 0.2;

impl Part {
    /// Distance in mm from `entry` along unit `inward` (both in the part's
    /// frame) to where the line first leaves the part's material, looked for
    /// up to `limit`. `None` when the part is not an extrusion, or has a
    /// shell, boolean or face offset, which this does not model; other holes
    /// and rounded or chamfered edges are left out.
    #[must_use]
    pub fn material_along(&self, entry: [f64; 3], inward: [f64; 3], limit: f64) -> Option<f64> {
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
        let mut cuts: Vec<Option<(Rings, f64)>> = Vec::new();
        for operation in &self.operations {
            cuts.push(match operation {
                ProgramOperation::Shell(_)
                | ProgramOperation::Boolean(_)
                | ProgramOperation::FaceOffset(_) => return None,
                ProgramOperation::Mirror(mirror) if mirror.axis == 2 => {
                    z = [2.0 * mirror.center_mm - z[1], 2.0 * mirror.center_mm - z[0]];
                    None
                }
                ProgramOperation::Cut(cut) => Some((vec![profile_outline(&cut.segments)], z[1])),
                _ => None,
            });
        }
        let outline = vec![profile_outline(segments)];
        // Walked back through the operations, each mirror reflecting the
        // point into the frame before it.
        let material = |t: f64| {
            let mut point: [f64; 3] = std::array::from_fn(|i| entry[i] + inward[i] * t);
            for (operation, cut) in self.operations.iter().zip(&cuts).rev() {
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
        };
        // To a millionth of a mm, so that an 18 mm thick part turned in space
        // is 18 mm, not 17.99999999999.
        let round = |mm: f64| Some((mm * 1.0e6).round() / 1.0e6);
        let mut inside = 0.0;
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
}
