//! Unit-density mass properties of an exact solid, as the OCCT worker measures them.

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

impl MassProperties {
    pub fn is_finite(&self) -> bool {
        self.centroid_mm
            .iter()
            .chain(&self.inertia_mm5)
            .all(|value| value.is_finite())
    }
}
