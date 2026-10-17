//! Face area two solids share, by the direction it faces.
use crate::tolerance::ROUNDING;
use serde::{Deserialize, Serialize};

/// Face area two solids share in one direction. `normal` is the first solid's
/// outward unit normal there, or zero where the shared faces are curved and
/// face no single direction.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContactPatch {
    pub normal: [f64; 3],
    pub area_mm2: f64,
}

impl ContactPatch {
    /// The same patch seen from the second solid.
    #[must_use]
    pub fn reversed(self) -> Self {
        Self {
            normal: self.normal.map(|component| -component),
            area_mm2: self.area_mm2,
        }
    }

    #[must_use]
    pub fn is_directed(&self) -> bool {
        self.normal.iter().any(|component| *component != 0.0)
    }

    /// True when `patches` have finite positive areas, unit or zero normals, and
    /// add up to `total_mm2` (none at all for no shared area).
    #[must_use]
    pub fn consistent(patches: &[Self], total_mm2: f64) -> bool {
        let valid = |patch: &Self| {
            let length_squared = patch.normal.iter().map(|c| c * c).sum::<f64>();
            patch.area_mm2.is_finite()
                && patch.area_mm2 > 0.0
                && patch.normal.iter().all(|c| c.is_finite())
                && (!patch.is_directed() || (length_squared - 1.0).abs() <= ROUNDING)
        };
        let sum = patches.iter().map(|patch| patch.area_mm2).sum::<f64>();
        patches.iter().all(valid) && (sum - total_mm2).abs() <= ROUNDING * total_mm2.max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_must_add_up_to_the_shared_area_with_unit_or_zero_normals() {
        let down = ContactPatch {
            normal: [0.0, 0.0, -1.0],
            area_mm2: 6.0,
        };
        let curved = ContactPatch {
            normal: [0.0; 3],
            area_mm2: 2.0,
        };
        assert!(ContactPatch::consistent(&[down, curved], 8.0));
        assert!(ContactPatch::consistent(&[], 0.0));
        assert!(!ContactPatch::consistent(&[down], 8.0));
        assert!(!ContactPatch::consistent(&[], 8.0));
        let long = ContactPatch {
            normal: [0.0, 0.0, -2.0],
            ..down
        };
        assert!(!ContactPatch::consistent(&[long], 6.0));
        assert_eq!(down.reversed().normal, [0.0, 0.0, 1.0]);
        assert!(!curved.is_directed() && !curved.reversed().is_directed());
    }
}
