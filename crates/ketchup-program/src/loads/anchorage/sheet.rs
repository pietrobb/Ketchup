//! A diaphragm's sheet held by anchors apart from each other: a rigid sheet
//! on springs (review 2026-11-21 R1121ac-PG-1000, capability review 977: two
//! anchors on the same side of the point, asked each for a force where the
//! sheet meets it, were added up as if side by side, though the sheet turns
//! on them by its length and only their spacing holds the turn).

use super::Anchorage;

/// One anchor of a sheet: how it holds a force where the sheet meets it, how
/// it holds the point of the member when the sheet turns it alone by its
/// length, and how far along the member from the point it meets the sheet.
pub(super) struct Spring {
    pub(super) at_contact: Anchorage,
    pub(super) turned: Anchorage,
    pub(super) arm_mm: f64,
}

/// Arms shorter than this tell nothing of how an anchor twists, mm.
const NO_ARM_MM: f64 = 1.0;

impl Spring {
    /// How stiffly the anchor holds the sheet's turn, N mm per radian: what
    /// the turned anchor gives past the one asked at its contact is its twist
    /// under the sheet's moment, arm² / r. Without an arm, nothing.
    fn turning(&self) -> f64 {
        let arm = self.arm_mm * self.arm_mm;
        if arm < NO_ARM_MM * NO_ARM_MM {
            return 0.0;
        }
        let twist = 1.0 / self.turned.stiffness_n_mm - 1.0 / self.at_contact.stiffness_n_mm;
        if twist > 0.0 {
            arm / twist
        } else {
            f64::INFINITY
        }
    }
}

/// How `springs` hold the point of the member through a rigid sheet: the
/// sheet moves across the member by u and turns by θ about the upright at the
/// point, each anchor holding k (u + θ a) at its arm a and r θ by its twist.
/// Stiffness at the point is K11 − K12² / K22 of
/// K = [[Σ k, Σ k a], [Σ k a, Σ (k a² + r)]]; the force first buckles the
/// anchor taking the most of it for its capacity. Anchors on both sides of
/// the point hold it as their stiffnesses added up, as before; on one side,
/// by their spacing.
pub(super) fn on_springs(springs: &[Spring]) -> Option<Anchorage> {
    let weakest = springs.iter().min_by(|a, b| {
        a.at_contact
            .stiffness_n_mm
            .total_cmp(&b.at_contact.stiffness_n_mm)
    })?;
    let (mut k11, mut k12, mut k22) = (0.0, 0.0, 0.0);
    for spring in springs {
        let (k, a) = (spring.at_contact.stiffness_n_mm, spring.arm_mm);
        k11 += k;
        k12 += k * a;
        k22 += k * a * a + spring.turning();
    }
    let (stiffness, moves, turns) = if k22.is_infinite() {
        (k11, 1.0 / k11, 0.0)
    } else if k22 > 0.0 {
        let det = k11 * k22 - k12 * k12;
        (det / k22, k22 / det, -k12 / det)
    } else {
        (k11, 1.0 / k11, 0.0)
    };
    // Each anchor's share of a force of 1 N at the point.
    let (capacity, buckling) = springs
        .iter()
        .map(|spring| {
            let share = (spring.at_contact.stiffness_n_mm * (moves + turns * spring.arm_mm)).abs();
            (
                spring.at_contact.capacity_n / share,
                spring.at_contact.buckling,
            )
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(Anchorage {
        stiffness_n_mm: stiffness,
        capacity_n: capacity,
        weakest: weakest.at_contact.weakest,
        buckling,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spring(k: f64, turned: f64, arm_mm: f64) -> Spring {
        let anchorage = |stiffness_n_mm| Anchorage {
            stiffness_n_mm,
            capacity_n: 1_000.0,
            weakest: 0,
            buckling: 0,
        };
        Spring {
            at_contact: anchorage(k),
            turned: anchorage(turned),
            arm_mm,
        }
    }

    #[test]
    fn anchors_on_both_sides_add_up() {
        let held = on_springs(&[spring(200.0, 2.0, -400.0), spring(200.0, 2.0, 400.0)]).unwrap();
        assert!((held.stiffness_n_mm - 400.0).abs() < 1.0, "{held:?}");
        assert!((held.capacity_n - 2_000.0).abs() < 1.0, "{held:?}");
    }

    #[test]
    fn one_anchor_gives_as_turned() {
        let held = on_springs(&[spring(200.0, 2.0, 400.0)]).unwrap();
        assert!((held.stiffness_n_mm - 2.0).abs() < 1e-9, "{held:?}");
    }

    /// Two anchors 200 N/mm 220 mm apart, 400 and 620 mm along: the couple
    /// holding the turn is about 2.3 times the force, and the sheet gives at
    /// the point far more than either anchor alone at its contact.
    #[test]
    fn anchors_on_one_side_hold_by_their_spacing() {
        let held = on_springs(&[spring(200.0, 2.0, 400.0), spring(160.0, 0.8, 620.0)]).unwrap();
        assert!(held.stiffness_n_mm < 40.0, "{held:?}");
        assert!(held.capacity_n < 1_000.0, "{held:?}");
    }
}
