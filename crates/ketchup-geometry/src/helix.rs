//! A circular helix as a chain of tangent-continuous cubic Bézier quarter
//! turns: the one helix both the assistant protocol and programs sweep along.

use crate::linalg::{CubicBezier, add, cross, dot, normalize_within, scale, sub};
use std::f64::consts::{FRAC_PI_2, TAU};

/// A helix around the axis through `origin_mm` along `axis`. It starts
/// `start_angle_degrees` from the reference direction (the principal axis
/// least aligned with `axis`, made square to it) and rises `pitch_mm` per
/// turn along `axis`, turning counter-clockwise about it seen from its tip
/// (`left_handed`: clockwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Helix {
    pub origin_mm: [f64; 3],
    pub axis: [f64; 3],
    pub radius_mm: f64,
    pub pitch_mm: f64,
    pub turns: f64,
    pub start_angle_degrees: f64,
    pub left_handed: bool,
}

impl Helix {
    /// How many quarter-turn pieces `turns` turns take.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn quarter_turns(turns: f64) -> usize {
        (TAU * turns / FRAC_PI_2).ceil().max(0.0) as usize
    }

    /// The fixed `up` of a sweep along this helix that keeps the profile in
    /// the axial section with its u pointing away from the axis (`u =
    /// tangent × up`): the axis, reversed for a left-handed helix, whose
    /// tangent runs the other way round. A thread profile drawn with its
    /// depth along u and its width along v cuts the same section on every
    /// turn. `None` when the axis has no direction.
    #[must_use]
    pub fn sweep_up(&self) -> Option<[f64; 3]> {
        let axis = normalize_within(self.axis, f64::EPSILON)?;
        Some(if self.left_handed {
            scale(axis, -1.0)
        } else {
            axis
        })
    }

    /// The helix as cubic Béziers, each matching the helix's position and
    /// tangent at both ends of its quarter turn (the last may be shorter).
    /// `None` when the axis has no direction.
    #[must_use]
    pub fn cubic_beziers(&self) -> Option<Vec<CubicBezier<3>>> {
        let axis = normalize_within(self.axis, f64::EPSILON)?;
        let reference = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .into_iter()
            .min_by(|left, right| dot(*left, axis).abs().total_cmp(&dot(*right, axis).abs()))?;
        let frame_u = normalize_within(
            sub(reference, scale(axis, dot(reference, axis))),
            f64::EPSILON,
        )?;
        let frame_v = cross(axis, frame_u);
        let start_angle = self.start_angle_degrees.to_radians();
        let handedness = if self.left_handed { -1.0 } else { 1.0 };
        let rise_per_radian = self.pitch_mm / TAU;
        let point = |angle: f64| {
            let phase = start_angle + handedness * angle;
            add(
                self.origin_mm,
                add(
                    scale(
                        add(scale(frame_u, phase.cos()), scale(frame_v, phase.sin())),
                        self.radius_mm,
                    ),
                    scale(axis, rise_per_radian * angle),
                ),
            )
        };
        let derivative = |angle: f64| {
            let phase = start_angle + handedness * angle;
            add(
                scale(
                    add(scale(frame_u, -phase.sin()), scale(frame_v, phase.cos())),
                    handedness * self.radius_mm,
                ),
                scale(axis, rise_per_radian),
            )
        };
        let total_angle = TAU * self.turns;
        let count = Self::quarter_turns(self.turns);
        Some(
            (0..count)
                .map(|index| {
                    let start = total_angle * index as f64 / count as f64;
                    let end = total_angle * (index + 1) as f64 / count as f64;
                    // The handle length that keeps a cubic within 0.03 % of a
                    // quarter circle (Δ/3 would sag 1.5 % inside it).
                    let handle = 4.0 / 3.0 * ((end - start) / 4.0).tan();
                    let (start_mm, end_mm) = (point(start), point(end));
                    CubicBezier::new([
                        start_mm,
                        add(start_mm, scale(derivative(start), handle)),
                        sub(end_mm, scale(derivative(end), handle)),
                        end_mm,
                    ])
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Helix;
    use crate::linalg::{dot, length, sub};

    #[test]
    fn quarter_turns_stay_on_the_helix_and_join_tangentially() {
        let helix = Helix {
            origin_mm: [5.0, -3.0, 2.0],
            axis: [0.0, 0.0, 2.0],
            radius_mm: 10.0,
            pitch_mm: 4.0,
            turns: 1.5,
            start_angle_degrees: 0.0,
            left_handed: false,
        };
        let pieces = helix.cubic_beziers().unwrap();
        assert_eq!(pieces.len(), 6);
        assert_eq!(pieces[0].points[0], [15.0, -3.0, 2.0]);
        let last = pieces[5].points[3];
        assert!(length(sub(last, [-5.0, -3.0, 8.0])) < 1e-9, "{last:?}");
        for (index, piece) in pieces.iter().enumerate() {
            for step in 0..=20 {
                let at = sub(piece.eval(f64::from(step) / 20.0), helix.origin_mm);
                // Within 0.03 % of the radius everywhere along the quarter turn.
                let radial = at[0].hypot(at[1]);
                assert!((radial - 10.0).abs() < 3e-3, "{radial}");
            }
            // Halfway along a quarter turn it is halfway up it.
            let middle = sub(piece.eval(0.5), helix.origin_mm);
            assert!(
                (middle[2] - (index as f64 + 0.5)).abs() < 1e-9,
                "{middle:?}"
            );
        }
        for pair in pieces.windows(2) {
            let (out, into) = (pair[0].derivative(1.0), pair[1].derivative(0.0));
            assert_eq!(pair[0].points[3], pair[1].points[0]);
            assert!(dot(out, into) > 0.0 && length(sub(out, into)) < 1e-9);
        }
        // Right-handed about +z: the first quarter runs towards +y.
        assert!(pieces[0].points[3][1] > 0.0);
        let left = Helix {
            left_handed: true,
            ..helix
        }
        .cubic_beziers()
        .unwrap();
        assert!(left[0].points[3][1] < 0.0);
    }
}
