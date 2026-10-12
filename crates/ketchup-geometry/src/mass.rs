//! Unit-density mass properties of an exact solid, as the OCCT worker measures them,
//! and the mass of placed solids and of assemblies made from them.

use crate::linalg::{Affine3, Mat3, Vec3};
use ketchup_tolerance::ACCUMULATED_ROUNDING;
use serde::{Deserialize, Serialize};

/// Centre of mass (mm) and inertia tensor (mm⁵) of a solid of density 1, in the
/// solid's own frame. `inertia_mm5` is `[xx, yy, zz, xy, xz, yz]` about the centre
/// of mass in tensor convention: `xx = ∫(y² + z²) dV` and `xy = −∫x·y dV`, the
/// convention of OCCT `GProp_GProps::MatrixOfInertia`. Times a density in kg/mm³
/// (kg/m³ divided by a billion) it is in kg·mm².
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MassProperties {
    pub centroid_mm: [f64; 3],
    pub inertia_mm5: [f64; 6],
}

/// Mass, centre of mass and inertia tensor about that centre (kg·mm², tensor
/// convention as [`MassProperties`]) of a placed solid or an assembly, in world axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mass {
    pub kg: f64,
    pub centroid_mm: [f64; 3],
    pub inertia_kg_mm2: Mat3,
}

fn scaled(matrix: Mat3, factor: f64) -> Mat3 {
    Mat3::from_rows(matrix.rows.map(|row| row.map(|value| value * factor)))
}

impl MassProperties {
    pub fn is_finite(&self) -> bool {
        self.centroid_mm
            .iter()
            .chain(&self.inertia_mm5)
            .all(|value| value.is_finite())
    }

    /// The inertia tensor as a symmetric matrix.
    #[must_use]
    pub fn tensor(&self) -> Mat3 {
        let [xx, yy, zz, xy, xz, yz] = self.inertia_mm5;
        Mat3::from_rows([[xx, xy, xz], [xy, yy, yz], [xz, yz, zz]])
    }

    /// The solid of `volume_mm3` and density `kg_m3` moved by `placement`. A
    /// rotation or a mirror turns the tensor as `R·I·Rᵀ`; `None` when the
    /// placement scales or shears, which changes the solid itself.
    #[must_use]
    pub fn placed(&self, volume_mm3: f64, kg_m3: f64, placement: Affine3) -> Option<Mass> {
        let turn = placement.linear;
        if !turn.is_orthonormal(ACCUMULATED_ROUNDING) {
            return None;
        }
        let kg_mm3 = kg_m3 / 1.0e9;
        Some(Mass {
            kg: volume_mm3 * kg_mm3,
            centroid_mm: placement
                .transform_point(Vec3::from(self.centroid_mm))
                .into(),
            inertia_kg_mm2: scaled(turn * self.tensor() * turn.transpose(), kg_mm3),
        })
    }
}

impl Mass {
    /// The parts as one rigid body: masses add, the centre is their weighted
    /// mean and each part's tensor moves to it by the parallel-axis theorem.
    /// `None` without parts or without mass.
    #[must_use]
    pub fn sum(parts: &[Self]) -> Option<Self> {
        let kg: f64 = parts.iter().map(|part| part.kg).sum();
        if parts.is_empty() || kg <= 0.0 {
            return None;
        }
        let centroid_mm: [f64; 3] = std::array::from_fn(|axis| {
            parts
                .iter()
                .map(|part| part.kg * part.centroid_mm[axis])
                .sum::<f64>()
                / kg
        });
        let mut inertia = [[0.0; 3]; 3];
        for part in parts {
            let d: [f64; 3] =
                std::array::from_fn(|axis| part.centroid_mm[axis] - centroid_mm[axis]);
            let squared = d.iter().map(|value| value * value).sum::<f64>();
            for (row, values) in inertia.iter_mut().enumerate() {
                for (column, value) in values.iter_mut().enumerate() {
                    let shift = if row == column { squared } else { 0.0 } - d[row] * d[column];
                    *value += part.inertia_kg_mm2.rows[row][column] + part.kg * shift;
                }
            }
        }
        Some(Self {
            kg,
            centroid_mm,
            inertia_kg_mm2: Mat3::from_rows(inertia),
        })
    }

    /// `[xx, yy, zz, xy, xz, yz]` of the tensor.
    #[must_use]
    pub fn inertia_components(&self) -> [f64; 6] {
        let m = self.inertia_kg_mm2.rows;
        [m[0][0], m[1][1], m[2][2], m[0][1], m[0][2], m[1][2]]
    }

    /// Principal moments of inertia about the centre of mass, ascending: the
    /// eigenvalues of the symmetric tensor (closed form of a 3×3 matrix).
    #[must_use]
    pub fn principal_kg_mm2(&self) -> [f64; 3] {
        let m = self.inertia_kg_mm2.rows;
        let off = m[0][1] * m[0][1] + m[0][2] * m[0][2] + m[1][2] * m[1][2];
        let mean = (m[0][0] + m[1][1] + m[2][2]) / 3.0;
        let spread = ((0..3).map(|i| (m[i][i] - mean).powi(2)).sum::<f64>() + 2.0 * off) / 6.0;
        let spread = spread.sqrt();
        if spread <= 0.0 {
            return [mean; 3];
        }
        let shifted = Mat3::from_rows(std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                (m[row][column] - if row == column { mean } else { 0.0 }) / spread
            })
        }));
        let angle = (shifted.determinant() / 2.0).clamp(-1.0, 1.0).acos() / 3.0;
        let largest = mean + 2.0 * spread * angle.cos();
        let smallest = mean + 2.0 * spread * (angle + 2.0 * std::f64::consts::FRAC_PI_3).cos();
        [smallest, 3.0 * mean - largest - smallest, largest]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100×50×20 mm steel box (7850 kg/m³): 0.785 kg, I = m(b² + c²)/12 …
    fn steel_box() -> MassProperties {
        let (a, b, c) = (100.0_f64, 50.0_f64, 20.0_f64);
        let v = a * b * c;
        MassProperties {
            centroid_mm: [50.0, 25.0, 10.0],
            inertia_mm5: [
                v * (b * b + c * c) / 12.0,
                v * (a * a + c * c) / 12.0,
                v * (a * a + b * b) / 12.0,
                0.0,
                0.0,
                0.0,
            ],
        }
    }

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() <= 1.0e-9 * expected.abs().max(1.0)
    }

    #[test]
    fn a_steel_box_weighs_and_turns_as_computed_by_hand() {
        let at_rest = steel_box()
            .placed(100_000.0, 7850.0, Affine3::IDENTITY)
            .unwrap();
        assert!(close(at_rest.kg, 0.785));
        assert_eq!(at_rest.centroid_mm, [50.0, 25.0, 10.0]);
        let [xx, yy, zz, ..] = at_rest.inertia_components();
        assert!(close(xx, 0.785 * 2900.0 / 12.0) && close(yy, 0.785 * 10_400.0 / 12.0));
        assert!(close(zz, 0.785 * 12_500.0 / 12.0));

        // Turned 45° about Z and moved: Ixy = (Iyy − Ixx)/2·… in tensor convention
        // −m(a² − b²)/24, the centre turns with the box.
        let (s, c) = std::f64::consts::FRAC_PI_4.sin_cos();
        let turned = Affine3 {
            linear: Mat3::from_rows([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]),
            translation: Vec3::new(1000.0, 0.0, 0.0),
        };
        let moved = steel_box().placed(100_000.0, 7850.0, turned).unwrap();
        let [_, _, _, xy, xz, yz] = moved.inertia_components();
        assert!(
            close(xy, -0.785 * (100.0 * 100.0 - 50.0 * 50.0) / 24.0),
            "{xy}"
        );
        assert!(xz.abs() < 1.0e-12 && yz.abs() < 1.0e-12);
        assert!(close(moved.centroid_mm[0], 1000.0 + (50.0 * c - 25.0 * s)));
        // Principal moments do not depend on the placement.
        let rest = at_rest.principal_kg_mm2();
        let turned = moved.principal_kg_mm2();
        assert!(
            (0..3).all(|axis| close(turned[axis], rest[axis])),
            "{turned:?} {rest:?}"
        );
        assert!(close(rest[0], xx) && close(rest[2], zz));

        // A mirror keeps the moments; a scale is not a rigid placement.
        let mirror = Affine3 {
            linear: Mat3::from_rows([[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            translation: Vec3::ZERO,
        };
        let mirrored = steel_box().placed(100_000.0, 7850.0, mirror).unwrap();
        assert!(close(mirrored.inertia_components()[0], xx));
        let doubled = Affine3 {
            linear: Mat3::from_rows([[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
            translation: Vec3::ZERO,
        };
        assert!(steel_box().placed(100_000.0, 7850.0, doubled).is_none());
    }

    #[test]
    fn two_boxes_side_by_side_add_by_the_parallel_axis_theorem() {
        let left = steel_box()
            .placed(100_000.0, 7850.0, Affine3::IDENTITY)
            .unwrap();
        let right = steel_box()
            .placed(
                100_000.0,
                7850.0,
                Affine3 {
                    linear: Mat3::IDENTITY,
                    translation: Vec3::new(100.0, 0.0, 0.0),
                },
            )
            .unwrap();
        let both = Mass::sum(&[left, right]).unwrap();
        assert!(close(both.kg, 1.57));
        assert!(close(both.centroid_mm[0], 100.0));
        // The same as one 200×50×20 box.
        let [xx, yy, zz, xy, ..] = both.inertia_components();
        assert!(close(xx, 1.57 * 2900.0 / 12.0));
        assert!(close(yy, 1.57 * 40_400.0 / 12.0) && close(zz, 1.57 * 42_500.0 / 12.0));
        assert!(xy.abs() < 1.0e-12);
        assert!(Mass::sum(&[]).is_none());
    }
}
