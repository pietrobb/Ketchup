//! The ISO metric thread a modelled thread cuts: the basic profile of ISO 68-1
//! (60° flanks) at the middle of the ISO 965-1 tolerance band 6g for a bolt
//! and 6H for a nut or tapped hole, so a modelled pair keeps the clearance a
//! real pair has. Tolerances come from the ISO 965-1 formulas, not its rounded
//! tables (they differ by a few µm). The thread is given as the groove it
//! removes: a section in the axial half-plane, radius from the axis and offset
//! along it, which a screw motion carries round the axis.

use crate::linalg::{Affine3, Vec3};
use ketchup_tolerance::ROUNDING;

/// tan 30°: an ISO 68-1 flank's run along the axis per unit of radius.
const FLANK_SLOPE: f64 = 0.577_350_269_189_625_8;

/// Not a tolerance: how far a groove reaches past the surface it cuts, as a
/// fraction of the pitch, so none of its faces lies on that surface.
const OVERCUT_PER_PITCH: f64 = 1.0 / 16.0;

/// Not a tolerance: the narrowest gap left between neighbouring turns of a
/// groove, as a fraction of the pitch.
const TURN_GAP_PER_PITCH: f64 = 1.0 / 32.0;

/// Not a tolerance: ISO 965-1 gives its deviations in µm.
const MICROMETRES_PER_MM: f64 = 1000.0;

/// ISO 965-1 nominal diameter ranges, mm: a size over one limit up to and
/// including the next takes the geometric mean of the two.
const DIAMETER_RANGES_MM: [f64; 11] = [
    0.99, 1.4, 2.8, 5.6, 11.2, 22.4, 45.0, 90.0, 180.0, 355.0, 600.0,
];

/// An ISO metric thread by its nominal diameter and pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricThread {
    pub nominal_diameter_mm: f64,
    pub pitch_mm: f64,
}

/// Diameters of one side of a thread at the middle of its tolerance band.
/// The root of each side (the minor diameter of a bolt, the major of a nut)
/// keeps the basic profile's distance from its pitch diameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadDiameters {
    pub major_mm: f64,
    pub pitch_mm: f64,
    pub minor_mm: f64,
}

impl MetricThread {
    /// H of ISO 68-1: the height of the fundamental triangle.
    #[must_use]
    pub fn triangle_height_mm(&self) -> f64 {
        3.0_f64.sqrt() / 2.0 * self.pitch_mm
    }

    /// ISO 965-1 grade 6 tolerances and the g deviation, mm, or `None`
    /// outside the standard's diameter range: (es of g, Td, Td2, TD2, TD1).
    fn grade_6(&self) -> Option<(f64, f64, f64, f64, f64)> {
        let p = self.pitch_mm;
        let d = self.nominal_diameter_mm;
        let range = DIAMETER_RANGES_MM
            .windows(2)
            .find(|range| range[0] < d && d <= range[1])?;
        if !(p.is_finite() && p > 0.0) {
            return None;
        }
        let micrometres = 1.0 / MICROMETRES_PER_MM;
        let g_deviation = -(15.0 + 11.0 * p) * micrometres;
        let major = (180.0 * p.powf(2.0 / 3.0) - 3.15 / p.sqrt()) * micrometres;
        let pitch = 90.0 * p.powf(0.4) * (range[0] * range[1]).sqrt().powf(0.1) * micrometres;
        let minor = if p < 1.0 {
            433.0 * p - 190.0 * p.powf(1.22)
        } else {
            230.0 * p.powf(0.7)
        } * micrometres;
        Some((g_deviation, major, pitch, 1.32 * pitch, minor))
    }

    /// A bolt's diameters at the middle of 6g.
    #[must_use]
    pub fn external_6g(&self) -> Option<ThreadDiameters> {
        let (deviation, major, pitch, _, _) = self.grade_6()?;
        let h = self.triangle_height_mm();
        let pitch_mm = self.nominal_diameter_mm - 0.75 * h + deviation - pitch / 2.0;
        Some(ThreadDiameters {
            major_mm: self.nominal_diameter_mm + deviation - major / 2.0,
            pitch_mm,
            // ISO 68-1: the root of a bolt lies 2H/3 under its pitch diameter
            // across (d3 = d − 17H/12).
            minor_mm: pitch_mm - 2.0 * h / 3.0,
        })
    }

    /// A nut's or tapped hole's diameters at the middle of 6H.
    #[must_use]
    pub fn internal_6h(&self) -> Option<ThreadDiameters> {
        let (_, _, _, pitch, minor) = self.grade_6()?;
        let h = self.triangle_height_mm();
        let pitch_mm = self.nominal_diameter_mm - 0.75 * h + pitch / 2.0;
        Some(ThreadDiameters {
            major_mm: pitch_mm + 0.75 * h,
            pitch_mm,
            minor_mm: self.nominal_diameter_mm - 1.25 * h + minor / 2.0,
        })
    }

    /// The groove of a bolt's thread cut into a shank of
    /// `surface_diameter_mm`, from its 6g root out past the shank. `None`
    /// when the shank does not reach above the root or the groove would be
    /// as wide as the pitch there.
    #[must_use]
    pub fn external_groove(&self, surface_diameter_mm: f64) -> Option<Vec<[f64; 2]>> {
        let diameters = self.external_6g()?;
        let pitch_radius = diameters.pitch_mm / 2.0;
        let width = |radius: f64| self.pitch_mm / 2.0 + 2.0 * FLANK_SLOPE * (radius - pitch_radius);
        let root = diameters.minor_mm / 2.0;
        let outside = surface_diameter_mm / 2.0 + OVERCUT_PER_PITCH * self.pitch_mm;
        (surface_diameter_mm / 2.0 > root).then_some(())?;
        // The bolt's tooth, not its groove, lies on the phase helix.
        let center = self.pitch_mm / 2.0;
        self.groove(center, [root, width(root)], [outside, width(outside)])
    }

    /// A bolt's thread on a shank of `surface_diameter_mm`: the crest and
    /// outer radius between which the shank is first turned down to its 6g
    /// major diameter, and the groove then cut into that. `None` when the
    /// shank is not wider than the 6g major diameter.
    #[must_use]
    pub fn turned_shank(&self, surface_diameter_mm: f64) -> Option<([f64; 2], Vec<[f64; 2]>)> {
        let crest = self.external_6g()?.major_mm / 2.0;
        let outer = surface_diameter_mm / 2.0 + OVERCUT_PER_PITCH * self.pitch_mm;
        (surface_diameter_mm / 2.0 > crest).then_some(())?;
        Some(([crest, outer], self.external_groove(2.0 * crest)?))
    }

    /// The groove of a nut's or tapped hole's thread cut out of a bore of
    /// `bore_diameter_mm`, from inside the bore out to its 6H root. `None`
    /// when the bore reaches the root or the groove would be as wide as the
    /// pitch there.
    #[must_use]
    pub fn internal_groove(&self, bore_diameter_mm: f64) -> Option<Vec<[f64; 2]>> {
        let diameters = self.internal_6h()?;
        let pitch_radius = diameters.pitch_mm / 2.0;
        let width = |radius: f64| self.pitch_mm / 2.0 - 2.0 * FLANK_SLOPE * (radius - pitch_radius);
        let root = diameters.major_mm / 2.0;
        let inside = bore_diameter_mm / 2.0 - OVERCUT_PER_PITCH * self.pitch_mm;
        (bore_diameter_mm / 2.0 < root).then_some(())?;
        self.groove(0.0, [inside, width(inside)], [root, width(root)])
    }

    /// The trapezoid centred on offset `center` from radius `inner` to
    /// `outer`, each given with its width along the axis, counter-clockwise
    /// in (radius, offset).
    fn groove(&self, center: f64, inner: [f64; 2], outer: [f64; 2]) -> Option<Vec<[f64; 2]>> {
        let widest = self.pitch_mm * (1.0 - TURN_GAP_PER_PITCH);
        let valid = |width: f64| width > 0.0 && width <= widest;
        let ([inner, inner_width], [outer, outer_width]) = (inner, outer);
        (inner > 0.0 && inner < outer && valid(inner_width) && valid(outer_width)).then(|| {
            vec![
                [inner, center - inner_width / 2.0],
                [outer, center - outer_width / 2.0],
                [outer, center + outer_width / 2.0],
                [inner, center + inner_width / 2.0],
            ]
        })
    }
}

/// Where a thread's helix lies, which fixes its phase: the helix crosses
/// the half-line from `origin` along `reference` (square to `axis`), and at
/// offset s along `axis` it has turned 2π·s/pitch about the axis
/// (clockwise for a left-handed thread). The helix runs along the middle of a
/// hole's groove and of a bolt's tooth, so a bolt and a hole that share a
/// phase mesh. A tapped hole takes its entry point
/// as origin and the axis into the material; a bolt that sits in it cuts the
/// hole's phase carried into its own coordinates by [`Self::in_part`], so the
/// pair meshes at any depth and any turn of the bolt about its axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadPhase {
    pub origin: Vec3,
    pub axis: Vec3,
    pub reference: Vec3,
}

impl ThreadPhase {
    /// The phase a thread takes from where it starts in its part: `origin`
    /// on its axis, the unit `axis` it runs along, and the part's x
    /// direction square to it (its y direction for a thread along x).
    /// `None` for an axis that is not a unit vector.
    #[must_use]
    pub fn in_own_part(origin: Vec3, axis: Vec3) -> Option<Self> {
        ((axis.length() - 1.0).abs() <= ROUNDING).then_some(())?;
        let square = |direction: Vec3| direction - axis * direction.dot(axis);
        let reference = [Vec3::X, Vec3::Y]
            .into_iter()
            .map(square)
            .find(|direction| direction.length() > 0.5)?;
        Some(Self {
            origin,
            axis,
            reference: reference / reference.length(),
        })
    }

    /// The same helix in the coordinates of a part whose `placement` maps
    /// its coordinates to these. `None` for a placement that is not a
    /// rotation and a move: a mirror would turn the thread the other way.
    #[must_use]
    pub fn in_part(&self, placement: Affine3) -> Option<Self> {
        let proper = placement.linear.is_orthonormal(ROUNDING) && placement.determinant() > 0.0;
        let local = placement.invert().ok()?;
        proper.then(|| Self {
            origin: local.transform_point(self.origin),
            axis: local.transform_vector(self.axis),
            reference: local.transform_vector(self.reference),
        })
    }

    /// Offset of `point` along the axis from the origin: where it sits on
    /// the helix's range.
    #[must_use]
    pub fn offset_of(&self, point: Vec3) -> f64 {
        (point - self.origin).dot(self.axis)
    }
}

/// The volume per unit length along the axis that a groove of `profile`
/// (see [`MetricThread::external_groove`]) occupies between radii `from`
/// and `to`, mm²: at radius r it takes the share width(r)/pitch of the
/// circle. A screw motion keeps every cross-section square to the axis the
/// same, so this times a length is the exact volume a full-length groove
/// removes there.
#[must_use]
pub fn groove_area_per_length(profile: &[[f64; 2]], pitch_mm: f64, from: f64, to: f64) -> f64 {
    let width = |radius: f64| {
        let mut crossings = Vec::new();
        for (index, start) in profile.iter().enumerate() {
            let end = profile[(index + 1) % profile.len()];
            if (start[0] <= radius) != (end[0] <= radius) {
                let t = (radius - start[0]) / (end[0] - start[0]);
                crossings.push(start[1] + t * (end[1] - start[1]));
            }
        }
        crossings.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - crossings.iter().copied().fold(f64::INFINITY, f64::min)
    };
    // Between the profile's corner radii the width is linear in r, so the
    // integrand r·width is quadratic and two-point Gauss is exact; its nodes
    // lie inside the piece, away from the corners.
    let mut radii = profile
        .iter()
        .map(|point| point[0].clamp(from, to))
        .chain([from, to])
        .collect::<Vec<_>>();
    radii.sort_by(f64::total_cmp);
    radii.dedup();
    let node = 1.0 / 3.0_f64.sqrt();
    radii
        .windows(2)
        .map(|piece| {
            let middle = (piece[0] + piece[1]) / 2.0;
            let half = (piece[1] - piece[0]) / 2.0;
            [middle - half * node, middle + half * node]
                .into_iter()
                .map(|radius| {
                    let share = width(radius);
                    if share.is_finite() {
                        half * 2.0 * std::f64::consts::PI * radius * share / pitch_mm
                    } else {
                        0.0
                    }
                })
                .sum::<f64>()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M5: MetricThread = MetricThread {
        nominal_diameter_mm: 5.0,
        pitch_mm: 0.8,
    };

    #[test]
    fn m5_diameters_sit_in_the_iso_965_2_bands() {
        // ISO 965-2 limits for M5 6g: d 4.826..4.976, d2 4.361..4.456; 6H:
        // D2 4.480..4.605, D1 4.134..4.334.
        let bolt = M5.external_6g().unwrap();
        let nut = M5.internal_6h().unwrap();
        for (value, low, high) in [
            (bolt.major_mm, 4.826, 4.976),
            (bolt.pitch_mm, 4.361, 4.456),
            (nut.pitch_mm, 4.480, 4.605),
            (nut.minor_mm, 4.134, 4.334),
        ] {
            let middle = (low + high) / 2.0;
            assert!(
                (value - middle).abs() < 0.003,
                "{value} not mid of {low}..{high}"
            );
        }
        assert!(nut.major_mm > 5.0 && bolt.minor_mm < 5.0 - 1.226_869 * 0.8 + 0.001);
    }

    /// The bolt's tooth at radius r is pitch − groove width; the nut's
    /// groove must hold it with room on both flanks at every radius they
    /// share, and the bolt's crest and root must clear the nut's root and
    /// bore.
    fn flank_clearance(thread: MetricThread, tap_drill_mm: f64) -> f64 {
        let bolt = thread
            .external_groove(thread.external_6g().unwrap().major_mm)
            .unwrap();
        let nut = thread.internal_groove(tap_drill_mm).unwrap();
        let width_at = |profile: &[[f64; 2]], radius: f64| {
            let t = (radius - profile[0][0]) / (profile[1][0] - profile[0][0]);
            let side = |from: [f64; 2], to: [f64; 2]| from[1] + t * (to[1] - from[1]);
            side(profile[3], profile[2]) - side(profile[0], profile[1])
        };
        let crest = thread.external_6g().unwrap().major_mm / 2.0;
        let root = thread.external_6g().unwrap().minor_mm / 2.0;
        assert!(crest < thread.internal_6h().unwrap().major_mm / 2.0);
        assert!(root < tap_drill_mm / 2.0);
        (0..=100)
            .map(|step| {
                let radius =
                    tap_drill_mm / 2.0 + (crest - tap_drill_mm / 2.0) * f64::from(step) / 100.0;
                let tooth = thread.pitch_mm - width_at(&bolt, radius);
                (width_at(&nut, radius) - tooth) / 2.0
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn a_6g_bolt_clears_a_6h_hole_on_both_flanks() {
        let clearance = flank_clearance(M5, 4.2);
        assert!(
            (0.03..0.05).contains(&clearance),
            "M5 flank clearance {clearance}"
        );
        for (diameter, pitch) in [
            (1.6, 0.35),
            (3.0, 0.5),
            (8.0, 1.25),
            (12.0, 1.75),
            (24.0, 3.0),
        ] {
            let thread = MetricThread {
                nominal_diameter_mm: diameter,
                pitch_mm: pitch,
            };
            assert!(
                flank_clearance(thread, diameter - pitch) > 0.01,
                "M{diameter}"
            );
        }
    }

    #[test]
    fn grooves_stay_narrower_than_the_pitch_and_reach_past_the_surface() {
        let crest = M5.external_6g().unwrap().major_mm;
        let bolt = M5.external_groove(crest).unwrap();
        assert!(bolt[2][0] > crest / 2.0 && bolt[2][1] - bolt[1][1] < 0.8);
        let hole = M5.internal_groove(4.2).unwrap();
        assert!(hole[0][0] < 2.1 && hole[3][1] - hole[0][1] < 0.8);
        assert!(M5.external_groove(3.9).is_none());
        assert!(M5.internal_groove(5.1).is_none());
        assert!(
            MetricThread {
                nominal_diameter_mm: 700.0,
                pitch_mm: 6.0
            }
            .external_6g()
            .is_none()
        );
    }

    /// The point of a right-handed helix of radius 1 at offset `s`.
    fn helix_point(phase: &ThreadPhase, pitch: f64, s: f64) -> Vec3 {
        let angle = 2.0 * std::f64::consts::PI * s / pitch;
        phase.origin
            + phase.axis * s
            + phase.reference * angle.cos()
            + phase.axis.cross(phase.reference) * angle.sin()
    }

    #[test]
    fn a_phase_carried_into_a_turned_and_moved_part_is_the_same_helix() {
        let hole = ThreadPhase {
            origin: Vec3::new(3.0, -2.0, 10.0),
            axis: Vec3::new(0.0, 0.0, -1.0),
            reference: Vec3::X,
        };
        let (sin, cos) = 37.0_f64.to_radians().sin_cos();
        let placement = Affine3 {
            linear: crate::linalg::Mat3::from_rows([
                [cos, -sin, 0.0],
                [sin, cos, 0.0],
                [0.0, 0.0, 1.0],
            ]),
            translation: Vec3::new(3.0, -2.0, 2.7),
        };
        let bolt = hole.in_part(placement).unwrap();
        for s in [-1.3, 0.0, 0.37, 7.3] {
            let world = helix_point(&hole, 0.8, s);
            let carried = placement.transform_point(helix_point(&bolt, 0.8, s));
            assert!(world.distance(carried) < 1.0e-12, "{world:?} {carried:?}");
        }
        assert!((bolt.offset_of(Vec3::ZERO) - 7.3).abs() < 1.0e-12);

        let mut mirror = placement;
        mirror.linear.rows[2][2] = -1.0;
        assert_eq!(hole.in_part(mirror), None);
    }

    #[test]
    fn groove_area_is_exact_for_a_rectangle() {
        let profile = [[1.0, -0.2], [2.0, -0.2], [2.0, 0.2], [1.0, 0.2]];
        let area = groove_area_per_length(&profile, 1.0, 1.0, 2.0);
        let expected = std::f64::consts::PI * (4.0 - 1.0) * 0.4;
        assert!((area - expected).abs() < 1.0e-9, "{area} {expected}");
    }
}
